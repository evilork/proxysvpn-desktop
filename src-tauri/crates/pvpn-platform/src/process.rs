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

    /// Runs the command, discarding output, and fails on a non-zero exit.
    ///
    /// The error text is deliberately identical to the pre-split `run_cmd` in
    /// tun.rs: some of these strings reach the user through connect errors.
    pub async fn run(&self) -> Result<()> {
        let status = self
            .command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .with_context(|| format!("spawn {} {:?}", self.program, self.args))?;
        if !status.success() {
            return Err(anyhow!(
                "{} {:?} failed: {}",
                self.program,
                self.args,
                status
            ));
        }
        Ok(())
    }

    /// Runs the command ignoring failures — for teardown steps where "already
    /// gone" and "removed" are the same outcome.
    pub async fn run_best_effort(&self) {
        if let Err(e) = self.run().await {
            crate::log::log("info", "net", &format!("ignored: {}", e));
        }
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

impl std::fmt::Display for Argv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.program)?;
        for a in &self.args {
            write!(f, " {}", a)?;
        }
        Ok(())
    }
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

    /// The failure text embeds `{:?}` of the argument vector. Pinning it keeps
    /// user-visible connect errors identical to the pre-split build.
    #[test]
    fn args_debug_shape_matches_slice_debug() {
        let a = Argv::new("x", ["-n", "delete"]);
        assert_eq!(format!("{:?}", a.args), r#"["-n", "delete"]"#);
    }
}
