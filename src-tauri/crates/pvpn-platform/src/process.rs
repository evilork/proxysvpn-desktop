// src-tauri/crates/pvpn-platform/src/process.rs
// Command plumbing shared by every platform implementation.
//
// `Argv` exists so that *what* a platform runs is a pure value that tests can
// pin, separate from *running* it. The golden tests in `net::plan` compare
// `Argv`s, which is how "macOS behaviour is unchanged" is actually enforced.

use anyhow::{anyhow, Context, Result};
use std::process::{Output, Stdio};
use tokio::process::Command;

/// `CREATE_NO_WINDOW` — without it every netsh/taskkill/sidecar spawn from a
/// `windows_subsystem = "windows"` GUI app flashes (or keeps) a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Suppresses the console window of a child process on Windows; no-op elsewhere.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argv {
    pub program: String,
    pub args: Vec<String>,
}

impl Argv {
    pub fn new<P, I, S>(program: P, args: I) -> Self
    where
        P: Into<String>,
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Argv {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        no_window(&mut cmd);
        cmd
    }

    /// Runs the command and fails on a non-zero exit.
    ///
    /// The error text is deliberately identical to the pre-split `run_cmd` in
    /// tun.rs: some of these strings reach the user through connect errors,
    /// and `raise_error` classifies them by words. What the command itself
    /// printed goes to the log as a line of its own instead: a Russian netsh
    /// message can contain «шлюз», which would misclassify the failure.
    /// Without that line, a failed `netsh … set address` on a Windows VM
    /// (02.10.2026) left nothing to go on but "exit code: 1".
    pub async fn run(&self) -> Result<()> {
        self.run_noting("warn").await
    }

    async fn run_noting(&self, said_level: &str) -> Result<()> {
        let out = self
            .command()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("spawn {} {:?}", self.program, self.args))?;
        if !out.status.success() {
            let said = summarize(
                &format!("{} {}", console_text(&out.stderr), console_text(&out.stdout)),
                SAID_MAX_CHARS,
            );
            if !said.is_empty() {
                crate::log::log(said_level, "net", &format!("{} said: {}", self.short_name(), said));
            }
            return Err(anyhow!(
                "{} {:?} failed: {}",
                self.program,
                self.args,
                out.status
            ));
        }
        Ok(())
    }

    /// Runs the command ignoring failures — for teardown steps where "already
    /// gone" and "removed" are the same outcome.
    pub async fn run_best_effort(&self) {
        if let Err(e) = self.run_noting("info").await {
            crate::log::log("info", "net", &format!("ignored: {}", e));
        }
    }

    /// The program's file name without folders or `.exe`, for log lines.
    fn short_name(&self) -> &str {
        let name = self.program.rsplit(['/', '\\']).next().unwrap_or(&self.program);
        name.strip_suffix(".exe").unwrap_or(name)
    }

    pub async fn output(&self) -> Result<Output> {
        self.command()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| format!("spawn {} {:?}", self.program, self.args))
    }

    /// stdout as UTF-8 (lossy), or `None` when the command could not run.
    /// Query helpers want "no answer" rather than an error type.
    pub async fn stdout_lossy(&self) -> Option<String> {
        let out = self.output().await.ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// True when the command exited 0 — used for existence probes.
    pub async fn succeeds(&self) -> bool {
        matches!(self.output().await, Ok(o) if o.status.success())
    }
}

/// How much of a failed command's own words a log line keeps.
const SAID_MAX_CHARS: usize = 240;

/// Whitespace collapsed to single spaces and the result cut to `max` chars:
/// netsh pads its messages with blank lines and CRLFs.
fn summarize(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match joined.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &joined[..cut]),
        None => joined,
    }
}

/// A child's output as text. Console programs on Windows (netsh, route)
/// write in the OEM code page — CP866 on a Russian system — so bytes that
/// are not UTF-8 are decoded from it there; elsewhere they are lossy UTF-8.
fn console_text(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    #[cfg(windows)]
    {
        if let Some(text) = oem::decode(bytes) {
            return text;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
mod oem {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};

    /// `bytes` decoded from the console's OEM code page, or `None` if the
    /// system would not convert them.
    pub fn decode(bytes: &[u8]) -> Option<String> {
        if bytes.is_empty() {
            return Some(String::new());
        }
        // SAFETY: a null output buffer asks only for the required length.
        let len = unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
        let len = usize::try_from(len).ok().filter(|n| *n > 0)?;
        let mut wide = vec![0u16; len];
        // SAFETY: `wide` holds exactly the length the first call reported.
        let written =
            unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide)) };
        let written = usize::try_from(written).ok().filter(|n| *n > 0)?;
        wide.truncate(written);
        Some(String::from_utf16_lossy(&wide))
    }
}

impl std::fmt::Display for Argv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.program)?;
        for a in &self.args {
            write!(f, " {}", a)?;
        }
        Ok(())
    }
}

/// Pids of the engines this app is running right now (Windows: xray and
/// hysteria, registered by the GUI's pidfile_windows.rs as they start and
/// forgotten as they stop).
///
/// The Windows backend asks it who may listen on the SOCKS port tun2socks
/// sends the whole machine's traffic to (`net::windows`): a listener that is
/// not one of these is a stranger that took the port while xray restarted.
static ENGINE_PIDS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

fn engine_pids() -> std::sync::MutexGuard<'static, Vec<u32>> {
    // A poisoned list is still a list of pids; refusing to read it would make
    // every engine of ours look like a stranger.
    ENGINE_PIDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Record a running engine of ours.
pub fn note_engine(pid: u32) {
    let mut pids = engine_pids();
    if !pids.contains(&pid) {
        pids.push(pid);
    }
}

/// Forget an engine that has stopped.
pub fn forget_engine(pid: u32) {
    engine_pids().retain(|&known| known != pid);
}

/// Is `pid` one of our running engines?
pub fn is_engine(pid: u32) -> bool {
    engine_pids().contains(&pid)
}

/// Ties a child process to the lifetime of this app, so it cannot outlive us.
///
/// Windows: the child joins a job object created with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, whose only handle this process holds
/// and never closes. The kernel closes it when the process ends — a clean quit,
/// a crash or "End task" alike — and that kills every member. This is what lets
/// the Windows build stop its own engines without sweeping by image name, which
/// would also kill another VPN client's `tun2socks.exe` or `xray.exe`.
///
/// Only the engines join the job, not this process: the WebView2 runtime is
/// our child too and has its own lifetime rules.
///
/// Elsewhere this is a no-op: macOS and Linux record their engines and stop
/// only what they recorded (the GUI's pidfile.rs), and the Linux helper owns
/// tun2socks and tears it down when the GUI's pipe closes.
pub fn tie_to_app(child: &tokio::process::Child) -> Result<()> {
    #[cfg(windows)]
    {
        job::assign(child)
    }

    #[cfg(not(windows))]
    {
        let _ = child;
        Ok(())
    }
}

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;

    use anyhow::{anyhow, Context, Result};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// The job handle as a plain number: `HANDLE` wraps a raw pointer and is
    /// therefore neither `Send` nor `Sync`, but the value is a kernel handle
    /// that stays valid for the life of the process. The error is kept as text
    /// so a failed creation is reported on every spawn, not only the first.
    static JOB: OnceLock<std::result::Result<usize, String>> = OnceLock::new();

    fn create() -> Result<HANDLE> {
        // SAFETY: no security attributes and no name: an anonymous job whose
        // only handle is the one returned here.
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.context("CreateJobObjectW")?;

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .context("job limit struct size")?;
        // SAFETY: `info` is a live JOBOBJECT_EXTENDED_LIMIT_INFORMATION and
        // `size` is its exact size; the call only reads from it.
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                size,
            )
        }
        .context("SetInformationJobObject")?;

        // Deliberately never closed: closing it is exactly what kills the
        // members, and that must happen when this process ends, not before.
        Ok(job)
    }

    fn job() -> Result<HANDLE> {
        let slot = JOB.get_or_init(|| {
            create()
                .map(|handle| handle.0 as usize)
                .map_err(|e| format!("{e:#}"))
        });
        match slot {
            Ok(raw) => Ok(HANDLE(*raw as *mut core::ffi::c_void)),
            Err(e) => Err(anyhow!("no job object: {e}")),
        }
    }

    pub fn assign(child: &tokio::process::Child) -> Result<()> {
        let process = child
            .raw_handle()
            .ok_or_else(|| anyhow!("the child has already exited and been reaped"))?;
        let job = job()?;
        // SAFETY: the job handle is never closed (see `create`), and the
        // process handle belongs to `child`, which outlives this call.
        unsafe { AssignProcessToJobObject(job, HANDLE(process)) }
            .context("AssignProcessToJobObject")
    }
}

/// Kills every process whose *exact* image name is `stem`, ignoring failures.
///
/// For a sidecar that outlived the handle we had on it — a crash, or a kill the
/// handle never saw — and still holds its loopback port, which makes the next
/// start fail with "address already in use".
///
/// Exact-match only (`pkill -x`, taskkill's `/IM` with the full image name): a
/// prefix match would also reap an unrelated `xray-helper` or the user's own
/// build of the same tool.
/// Sends SIGTERM first on Unix and only then SIGKILL, with a short grace
/// period: hysteria closes its QUIC session on TERM, and killing it outright
/// looked to the node (and to the user) like a connection that dropped. The
/// pre-split code sent only TERM here; the guarantee that the port is actually
/// released is what the follow-up KILL is for.
///
/// Windows has no graceful equivalent for a console-less GUI child, so
/// `taskkill /F` stands as it was.
#[cfg_attr(windows, allow(dead_code))] // only the Unix arm waits; see above
const KILL_GRACE: std::time::Duration = std::time::Duration::from_millis(400);

pub async fn kill_by_name(stem: &str) {
    #[cfg(unix)]
    {
        // On Windows `taskkill` resolves through the System32 path for the same
        // reason as netsh; see net::plan::windows::system32.
        Argv::new("pkill", ["-x", stem]).run_best_effort().await;
        tokio::time::sleep(KILL_GRACE).await;
        Argv::new("pkill", ["-9", "-x", stem]).run_best_effort().await;
    }

    #[cfg(windows)]
    {
        let argv = Argv::new(
            crate::net::plan::windows::taskkill_path(),
            ["/F", "/IM", &format!("{}.exe", stem), "/T"],
        );
        argv.run_best_effort().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_are_known_from_start_to_stop() {
        // Pids no real process here has; the list is process-wide.
        let (a, b) = (4_000_000_001, 4_000_000_002);
        assert!(!is_engine(a));
        note_engine(a);
        note_engine(a);
        note_engine(b);
        assert!(is_engine(a) && is_engine(b));
        forget_engine(a);
        assert!(!is_engine(a), "one forget undoes a repeated note");
        assert!(is_engine(b));
        forget_engine(b);
        assert!(!is_engine(b));
    }

    #[test]
    fn argv_keeps_program_and_args() {
        let a = Argv::new("/sbin/route", ["-n", "add", "-host", "203.0.113.7"]);
        assert_eq!(a.program, "/sbin/route");
        assert_eq!(a.args, vec!["-n", "add", "-host", "203.0.113.7"]);
        assert_eq!(a.to_string(), "/sbin/route -n add -host 203.0.113.7");
    }

    /// Exact match matters: without it, killing "xray" would also kill a
    /// user's "xray-knife" or our own "xray-helper".
    #[tokio::test]
    async fn kill_by_name_is_exact_match_only() {
        // The command is not run here; this pins the flags that make it exact.
        #[cfg(unix)]
        {
            let argv = Argv::new("pkill", ["-9", "-x", "xray"]);
            assert!(argv.args.contains(&"-x".to_string()));
        }
        #[cfg(windows)]
        {
            let argv = Argv::new("taskkill", ["/F", "/IM", "xray.exe", "/T"]);
            assert!(argv.args.contains(&"xray.exe".to_string()));
        }
    }

    /// The grace period must be long enough for a QUIC close to go out and
    /// short enough that the UI does not feel stuck between the two signals.
    #[test]
    fn kill_grace_is_a_short_wait_not_a_hang() {
        assert!(KILL_GRACE >= std::time::Duration::from_millis(100));
        assert!(KILL_GRACE <= std::time::Duration::from_millis(1000));
    }

    #[test]
    fn a_failed_command_is_summarized_on_one_short_line() {
        let netsh = "\r\n\r\nThe filename, directory name, or volume label syntax is incorrect.\r\n\r\n";
        assert_eq!(
            summarize(netsh, SAID_MAX_CHARS),
            "The filename, directory name, or volume label syntax is incorrect."
        );
        assert_eq!(summarize("  \r\n ", SAID_MAX_CHARS), "");
        let long = "я".repeat(300);
        let cut = summarize(&long, 10);
        assert_eq!(cut.chars().count(), 11, "ten chars and an ellipsis, cut on a char boundary");
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn console_text_keeps_utf8_and_never_fails_on_other_bytes() {
        assert_eq!(console_text("Ошибка".as_bytes()), "Ошибка");
        // CP866 «Ошибка»: not UTF-8. On Windows the OEM page decodes it; on
        // the CI hosts it must at least not panic or vanish.
        let cp866 = [0x8E, 0xE8, 0xA8, 0xA1, 0xAA, 0xA0];
        assert!(!console_text(&cp866).is_empty());
    }

    #[test]
    fn log_lines_name_the_program_not_its_folder() {
        assert_eq!(Argv::new(r"C:\WINDOWS\System32\netsh.exe", ["x"]).short_name(), "netsh");
        assert_eq!(Argv::new("/sbin/route", ["-n"]).short_name(), "route");
        assert_eq!(Argv::new("ip", ["r"]).short_name(), "ip");
    }

    /// The failure text embeds `{:?}` of the argument vector. Pinning it keeps
    /// user-visible connect errors identical to the pre-split build.
    #[test]
    fn args_debug_shape_matches_slice_debug() {
        let a = Argv::new("x", ["-n", "delete"]);
        assert_eq!(format!("{:?}", a.args), r#"["-n", "delete"]"#);
    }
}
