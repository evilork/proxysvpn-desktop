// src-tauri/src/pidfile.rs
//
// Which engine processes are OURS, written down so that only those are ever
// stopped.
//
// ── Why this exists ────────────────────────────────────────────────────────
// Until 28.09.2026 every stop, every launch and every quit ran
// `pkill -9 -x xray` — and the same for tun2socks, hysteria and sing-box. That
// is "kill every process on this Mac with that name", done as root, so nothing
// stood in its way. xray, sing-box and tun2socks are the engines of most other
// VPN clients a person keeps installed beside ours: connecting or quitting
// ProxysVPN silently cut the other client's tunnel.
//
// Now every engine we spawn goes through `Engine::spawn`, which writes one
// line to `engine.pids` — the pid, the engine's name and the exact binary we
// ran — before anything else can fail. A stop, and a launch after a crash,
// signals only a pid from that file, and only after the kernel confirms
// (proc_pidpath) that the pid still runs that very binary out of the folder
// our own executable lives in (Contents/MacOS in the bundle, target/debug in
// development). A pid that has since gone to another program fails the check
// and is merely forgotten. SIGTERM first, SIGKILL after `GRACE`.
//
// ── What the file is, and is not ──────────────────────────────────────────
// It lives beside the other state in /Library/Application Support/ProxysVPN
// (the app runs as root), falls back to the per-user folder like every other
// store here, is written atomically (temp file + rename) with mode 0600, and
// is only believed when it is a plain file owned by our own effective user and
// writable by nobody else: a root process that kills whatever a user-writable
// file names would be a gift to anyone else on the Mac.
//
// ── Linux ─────────────────────────────────────────────────────────────────
// The same file and the same rules, in the per-user data folder (appdirs.rs):
// the GUI is unprivileged there and spawns xray and hysteria as the user, and
// the kernel names a process's image through /proc/<pid>/exe instead of
// proc_pidpath. tun2socks never goes through this file on Linux: it belongs to
// the root helper, which owns and stops it (pvpn-platform, net/linux_priv.rs).
// Windows has its own, simpler answer (pidfile_windows.rs).

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};

/// The only names a line may carry. sing-box runs only inside the iOS
/// extension today; it is listed so a desktop build that ever spawns one gets
/// the same treatment rather than a new `pkill`.
const ENGINES: [&str; 4] = ["xray", "tun2socks", "hysteria", "sing-box"];

/// How long an engine gets to leave on SIGTERM before SIGKILL. xray, hysteria
/// and tun2socks all close their listeners and exit in milliseconds; two
/// seconds only matters for one that hangs, and bounds how long a quit waits.
const GRACE: Duration = Duration::from_secs(2);

/// How often a signalled process is looked at again during `GRACE`.
const POLL: Duration = Duration::from_millis(50);

const FILE_NAME: &str = "engine.pids";
const FORMAT_VERSION: u32 = 1;
const LOG_SOURCE: &str = "engine";

// ───────────────────────────────────────────────────────────────────────────
// One line of the file
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    pid: u32,
    name: String,
    /// The binary as the kernel will report it: canonical, absolute.
    path: PathBuf,
}

impl Entry {
    /// A line we would act on. Everything else is dropped on read, because a
    /// pid of 0 or a negative one means "a whole process group" to kill(2),
    /// -1 means "every process we may signal", and 1 is launchd.
    fn is_sane(&self) -> bool {
        signal_pid(self.pid).is_some()
            && ENGINES.contains(&self.name.as_str())
            && self.path.is_absolute()
            && self
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|file| names_engine(file, &self.name))
    }
}

/// `xray`, or `xray-aarch64-apple-darwin` as the development fallback in
/// `xray_manager::xray_paths` names it.
fn names_engine(file: &str, engine: &str) -> bool {
    file == engine
        || file
            .strip_prefix(engine)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// The pid as kill(2) takes it, or `None` for one we must never signal by
/// number: 0 and below (process groups, "everyone"), 1 (launchd), ourselves,
/// and anything past `pid_t`.
fn signal_pid(pid: u32) -> Option<i32> {
    let raw = i32::try_from(pid).ok()?;
    (raw > 1 && pid != std::process::id()).then_some(raw)
}

#[derive(Serialize)]
struct FileOut<'a> {
    version: u32,
    engines: &'a [Entry],
}

#[derive(Deserialize)]
struct FileIn {
    version: u32,
    #[serde(default)]
    engines: Vec<serde_json::Value>,
}

/// Why a file was not read at all.
#[derive(Debug, PartialEq, Eq)]
enum Unreadable {
    NotJson,
    UnknownVersion,
}

/// The sane lines of a file, and how many were thrown away.
///
/// Lines are parsed one by one so a single bad line (a negative pid, a name we
/// do not know) costs that line only, not every engine written beside it.
fn parse(text: &str) -> Result<(Vec<Entry>, usize), Unreadable> {
    let file: FileIn = serde_json::from_str(text).map_err(|_| Unreadable::NotJson)?;
    if file.version != FORMAT_VERSION {
        return Err(Unreadable::UnknownVersion);
    }
    let total = file.engines.len();
    let mut out: Vec<Entry> = Vec::with_capacity(total);
    for value in file.engines {
        let Ok(entry) = serde_json::from_value::<Entry>(value) else {
            continue;
        };
        if entry.is_sane() && !out.iter().any(|e| e.pid == entry.pid) {
            out.push(entry);
        }
    }
    let dropped = total - out.len();
    Ok((out, dropped))
}

// ───────────────────────────────────────────────────────────────────────────
// The file
// ───────────────────────────────────────────────────────────────────────────

/// Every read-modify-write of the file in this process goes through here, so
/// a stop in one task and a spawn in another cannot lose each other's line.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Same locations as `netmem.rs`, in the same order (appdirs.rs): on macOS
/// the launcher starts us as root with two different `HOME`s depending on the
/// path it takes, and the system folder is the one both of them see.
fn dir_candidates() -> Vec<PathBuf> {
    crate::appdirs::state_dirs()
}

/// `engine.pids`, wherever it lives. Tests point it at a temp folder.
#[derive(Debug, Clone)]
struct PidFile {
    dirs: Vec<PathBuf>,
}

impl PidFile {
    fn system() -> Self {
        Self {
            dirs: dir_candidates(),
        }
    }

    /// Every sane line from every location, first location first. A missing
    /// file is an empty one; a corrupt or untrusted one is ignored out loud
    /// and replaced by the next write.
    fn load(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        for dir in &self.dirs {
            let path = dir.join(FILE_NAME);
            let Some(text) = read_trusted(&path) else {
                continue;
            };
            match parse(&text) {
                Ok((entries, dropped)) => {
                    if dropped > 0 {
                        crate::logger::log(
                            "warn",
                            LOG_SOURCE,
                            &format!("{FILE_NAME}: dropped {dropped} unusable line(s)"),
                        );
                    }
                    for entry in entries {
                        if !out.iter().any(|e| e.pid == entry.pid) {
                            out.push(entry);
                        }
                    }
                }
                Err(why) => {
                    crate::logger::log("warn", LOG_SOURCE, &format!("{FILE_NAME} ignored: {why:?}"))
                }
            }
        }
        out
    }

    /// Write the whole list to the first location that takes it and remove
    /// the file from every other one, so after any save exactly one file
    /// holds everything. An empty list removes the file everywhere.
    fn save(&self, entries: &[Entry]) -> bool {
        if entries.is_empty() {
            for dir in &self.dirs {
                let _ = std::fs::remove_file(dir.join(FILE_NAME));
            }
            return true;
        }
        let body = FileOut {
            version: FORMAT_VERSION,
            engines: entries,
        };
        let Ok(text) = serde_json::to_string(&body) else {
            return false;
        };
        let written = self
            .dirs
            .iter()
            .position(|dir| write_atomic(&dir.join(FILE_NAME), text.as_bytes()).is_ok());
        let Some(winner) = written else {
            crate::logger::log(
                "warn",
                LOG_SOURCE,
                &format!("{FILE_NAME} could not be written"),
            );
            return false;
        };
        for (i, dir) in self.dirs.iter().enumerate() {
            if i != winner {
                let _ = std::fs::remove_file(dir.join(FILE_NAME));
            }
        }
        true
    }

    fn record(&self, name: &str, pid: u32, bin: &Path) {
        // Canonical because proc_pidpath answers with the resolved path, and
        // a `..` or a symlink in ours would make our own engine look foreign.
        let path = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
        let entry = Entry {
            pid,
            name: name.to_string(),
            path,
        };
        if !entry.is_sane() {
            crate::logger::log(
                "warn",
                LOG_SOURCE,
                &format!("not recorded pid={pid} name={name}: unusable line"),
            );
            return;
        }
        let _guard = lock();
        let mut all = self.load();
        // The same pid again can only be a new process: the old one is gone.
        all.retain(|e| e.pid != pid);
        all.push(entry);
        self.save(&all);
    }

    fn forget(&self, pid: u32) {
        let _guard = lock();
        let mut all = self.load();
        let before = all.len();
        all.retain(|e| e.pid != pid);
        if all.len() != before {
            self.save(&all);
        }
    }

    /// Stop every recorded engine (or only those called `only`) that the
    /// kernel confirms is ours, then drop the lines that are finished with.
    ///
    /// Blocking: up to `grace` plus a moment. The lock is NOT held while
    /// waiting, and the file is re-read before the final write, so an engine
    /// recorded meanwhile keeps its line.
    fn reap(&self, own_dirs: &[PathBuf], only: Option<&str>, grace: Duration) -> Vec<(Entry, Outcome)> {
        let targets: Vec<Entry> = {
            let _guard = lock();
            self.load()
                .into_iter()
                .filter(|e| only.is_none_or(|name| e.name == name))
                .collect()
        };
        if targets.is_empty() {
            return Vec::new();
        }

        let mut done: Vec<(Entry, Outcome)> = Vec::with_capacity(targets.len());
        let mut pending: Vec<Entry> = Vec::new();
        for entry in targets {
            match judge_in(&entry, exe_of(entry.pid).as_deref(), own_dirs) {
                Verdict::Ours => match send(entry.pid, libc::SIGTERM) {
                    Sent::Delivered => pending.push(entry),
                    Sent::NoSuchProcess => done.push((entry, Outcome::Stale)),
                    Sent::Refused => done.push((entry, Outcome::Denied)),
                },
                Verdict::Gone => done.push((entry, Outcome::Stale)),
                Verdict::Reused => done.push((entry, Outcome::NotOurs)),
                Verdict::OtherCopy => done.push((entry, Outcome::OtherCopy)),
            }
        }

        // Signal everything first and wait once, so a quit with three engines
        // costs one grace period at most, not three.
        let deadline = Instant::now() + grace;
        loop {
            let (alive, left): (Vec<Entry>, Vec<Entry>) =
                pending.into_iter().partition(|e| still_ours(e, own_dirs));
            done.extend(left.into_iter().map(|e| (e, Outcome::Terminated)));
            pending = alive;
            if pending.is_empty() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(POLL);
        }
        for entry in pending {
            // Asked once more right before the signal nobody can ignore: in
            // the grace period the engine may have left and its pid gone to
            // somebody else.
            let outcome = if still_ours(&entry, own_dirs) {
                match send(entry.pid, libc::SIGKILL) {
                    Sent::Delivered => Outcome::Killed,
                    Sent::NoSuchProcess => Outcome::Terminated,
                    Sent::Refused => Outcome::Denied,
                }
            } else {
                Outcome::Terminated
            };
            done.push((entry, outcome));
        }

        for (entry, outcome) in &done {
            log_outcome(&entry.name, entry.pid, *outcome);
        }

        let _guard = lock();
        let mut all = self.load();
        let before = all.len();
        all.retain(|e| !done.iter().any(|(d, o)| d == e && !o.keeps_line()));
        if all.len() != before {
            self.save(&all);
        }
        done
    }
}

/// The file's text, if it is a plain file only we could have written.
fn read_trusted(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    // symlink_metadata: a link planted in place of the file is refused, not
    // followed.
    let meta = std::fs::symlink_metadata(path).ok()?;
    // SAFETY: geteuid reads a credential of this process; it takes no
    // arguments, touches no memory of ours and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if !meta.file_type().is_file() || meta.uid() != euid || meta.mode() & 0o022 != 0 {
        crate::logger::log(
            "warn",
            LOG_SOURCE,
            &format!("{FILE_NAME} ignored: not a private file of this user"),
        );
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Temp file in the same folder, then rename: a crash mid-write leaves either
/// the old file or the new one, never half of one. No fsync: the file only
/// has to survive our death, not the machine's, and after a power cut there
/// are no engines left to find.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no folder")
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    // Left over from a write this pid never finished; `create_new` below
    // would otherwise fail on it forever.
    let _ = std::fs::remove_file(&tmp);

    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// ───────────────────────────────────────────────────────────────────────────
// Asking the kernel
// ───────────────────────────────────────────────────────────────────────────

/// What a recorded line turns out to be right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Runs the binary we recorded, from our own folder: ours to stop.
    Ours,
    /// No such process, or only its zombie (which has no image left).
    Gone,
    /// The pid now belongs to some other program.
    Reused,
    /// Our engine binary, but from another copy of the app (another folder).
    /// Not ours to stop; its own copy, or the kernel, will end it.
    OtherCopy,
}

/// Pure so every branch is testable without a process in that state.
fn judge(entry: &Entry, running: Option<&Path>, own_dir: &Path) -> Verdict {
    let Some(running) = running else {
        return Verdict::Gone;
    };
    if running != entry.path {
        return Verdict::Reused;
    }
    if running.parent() != Some(own_dir) {
        return Verdict::OtherCopy;
    }
    Verdict::Ours
}

/// `judge` against every folder this copy runs engines from: the folder of
/// our own executable and, on macOS as root, the root-owned copy
/// (`engine_stage.rs`). Ours when any of them says so.
fn judge_in(entry: &Entry, running: Option<&Path>, own_dirs: &[PathBuf]) -> Verdict {
    let mut verdict = match running {
        None => Verdict::Gone,
        Some(path) if path != entry.path => Verdict::Reused,
        Some(_) => Verdict::OtherCopy,
    };
    for dir in own_dirs {
        if judge(entry, running, dir) == Verdict::Ours {
            verdict = Verdict::Ours;
        }
    }
    verdict
}

fn still_ours(entry: &Entry, own_dirs: &[PathBuf]) -> bool {
    judge_in(entry, exe_of(entry.pid).as_deref(), own_dirs) == Verdict::Ours
}

/// The executable a live process runs, as the kernel reports it; `None` for
/// a pid that does not exist, a zombie, or one we may not inspect.
#[cfg(target_os = "macos")]
fn exe_of(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let raw = signal_pid(pid)?;
    let size = usize::try_from(libc::PROC_PIDPATHINFO_MAXSIZE).ok()?;
    let mut buf = vec![0u8; size];
    let len = u32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` is valid for writes of `len` bytes for the whole call and
    // proc_pidpath writes at most `len` bytes into it; it keeps no pointer.
    let written = unsafe { libc::proc_pidpath(raw, buf.as_mut_ptr().cast(), len) };
    let written = usize::try_from(written).ok().filter(|n| *n > 0)?;
    buf.truncate(written);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

/// Linux: the kernel's own link to the image. A zombie has none, and a process
/// of another user cannot be read — both answer `None`, which `judge` reads as
/// "gone", exactly like proc_pidpath on macOS. Every engine this file records
/// on Linux belongs to the GUI's own user.
#[cfg(target_os = "linux")]
fn exe_of(pid: u32) -> Option<PathBuf> {
    let raw = signal_pid(pid)?;
    std::fs::read_link(format!("/proc/{raw}/exe")).ok()
}

enum Sent {
    Delivered,
    NoSuchProcess,
    Refused,
}

fn send(pid: u32, signal: libc::c_int) -> Sent {
    let Some(raw) = signal_pid(pid) else {
        return Sent::Refused;
    };
    // SAFETY: kill(2) takes two integers and touches no memory of ours;
    // `raw` is > 1, so it can never address a process group, init or
    // "every process".
    if unsafe { libc::kill(raw, signal) } == 0 {
        return Sent::Delivered;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Sent::NoSuchProcess,
        _ => Sent::Refused,
    }
}

/// The folders our engines run from, canonical like the recorded paths: the
/// folder our own executable lives in and, on macOS as root, this copy's
/// root-owned engine folder (`engine_stage.rs`). Empty when unknown.
fn own_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    match std::env::current_exe().and_then(std::fs::canonicalize) {
        Ok(exe) => dirs.extend(exe.parent().map(Path::to_path_buf)),
        Err(e) => crate::logger::log("warn", LOG_SOURCE, &format!("own folder unknown: {e}")),
    }
    #[cfg(target_os = "macos")]
    dirs.extend(crate::engine_stage::current_home());
    dirs
}

// ───────────────────────────────────────────────────────────────────────────
// Outcomes and the log line
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Left on SIGTERM within the grace period.
    Terminated,
    /// Still there after the grace period: SIGKILL.
    Killed,
    /// Was already gone; the line is dropped.
    Stale,
    /// The pid had gone to another program: not touched, line dropped.
    NotOurs,
    /// Another copy of the app's engine: not touched, line kept.
    OtherCopy,
    /// The kernel refused the signal (a root engine and a non-root us).
    Denied,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Terminated => "terminated",
            Outcome::Killed => "killed",
            Outcome::Stale => "stale",
            Outcome::NotOurs => "not-ours",
            Outcome::OtherCopy => "other-copy",
            Outcome::Denied => "denied",
        }
    }

    /// Lines that still describe a live engine somebody may stop later.
    fn keeps_line(self) -> bool {
        matches!(self, Outcome::OtherCopy | Outcome::Denied)
    }
}

/// One `key=value` line per engine: support greps for `name=xray`, and the
/// first word is not a level word, so the logger keeps the level given here.
fn log_outcome(name: &str, pid: u32, outcome: Outcome) {
    let level = match outcome {
        Outcome::Killed | Outcome::Denied => "warn",
        _ => "info",
    };
    crate::logger::log(
        level,
        LOG_SOURCE,
        &format!("kill pid={pid} name={name} result={}", outcome.as_str()),
    );
}

// ───────────────────────────────────────────────────────────────────────────
// What the rest of the app uses
// ───────────────────────────────────────────────────────────────────────────

/// A child engine process, written down in `engine.pids` for as long as it is
/// held. The only way the managers spawn one, so no engine can exist that a
/// crash recovery would not know about.
pub struct Engine {
    name: &'static str,
    /// Taken at spawn: `Child::id` forgets it once the child is reaped, and
    /// the line still has to go.
    pid: Option<u32>,
    child: Child,
    file: PidFile,
}

impl Engine {
    /// Spawn `cmd`, which runs `bin`, and record it before anything can fail.
    pub fn spawn(name: &'static str, cmd: &mut Command, bin: &Path) -> std::io::Result<Self> {
        Self::spawn_with(PidFile::system(), name, cmd, bin)
    }

    fn spawn_with(
        file: PidFile,
        name: &'static str,
        cmd: &mut Command,
        bin: &Path,
    ) -> std::io::Result<Self> {
        let child = cmd.spawn()?;
        let pid = child.id();
        if let Some(pid) = pid {
            file.record(name, pid, bin);
        }
        Ok(Self {
            name,
            pid,
            child,
            file,
        })
    }

    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// SIGTERM, up to `GRACE` to leave, then SIGKILL; the line goes with it.
    pub async fn stop(self) {
        self.stop_within(GRACE).await;
    }

    async fn stop_within(mut self, grace: Duration) {
        let outcome = self.terminate(grace).await;
        if let Some(pid) = self.pid {
            log_outcome(self.name, pid, outcome);
        }
        // Drop forgets the line.
    }

    /// Signals by the handle's pid, which is exact: until the child is
    /// reaped the pid cannot be handed to anyone else, zombie or not.
    async fn terminate(&mut self, grace: Duration) -> Outcome {
        // Already reaped (an `is_running` saw it exit): nothing to signal.
        let Some(pid) = self.child.id() else {
            return Outcome::Stale;
        };
        if matches!(send(pid, libc::SIGTERM), Sent::Delivered) {
            if let Ok(Ok(_)) = tokio::time::timeout(grace, self.child.wait()).await {
                return Outcome::Terminated;
            }
        }
        match self.child.kill().await {
            Ok(()) => Outcome::Killed,
            Err(_) => Outcome::Denied,
        }
    }
}

impl Drop for Engine {
    /// A held engine that is dropped is being killed by `kill_on_drop`, or has
    /// already exited; either way its line is finished.
    fn drop(&mut self) {
        if let Some(pid) = self.pid.take() {
            self.file.forget(pid);
        }
    }
}

/// Stop recorded `name` engines we no longer hold a handle to — a process a
/// crash left behind would otherwise keep its port or device and make the
/// next start fail. Runs on a blocking thread: it may wait up to `GRACE`.
pub async fn sweep(name: &'static str) {
    let own = own_dirs();
    if own.is_empty() {
        return;
    }
    let file = PidFile::system();
    let joined = tokio::task::spawn_blocking(move || {
        file.reap(&own, Some(name), GRACE);
    })
    .await;
    if let Err(e) = joined {
        crate::logger::log(
            "warn",
            LOG_SOURCE,
            &format!("sweep of {name} did not finish: {e}"),
        );
    }
}

/// Stop every recorded engine that is ours. Synchronous on purpose: it runs
/// at launch, on signals and at exit, where there is no runtime to await on.
pub fn reap_all() {
    let own = own_dirs();
    if own.is_empty() {
        return;
    }
    PidFile::system().reap(&own, None, GRACE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child as StdChild, Command as StdCommand, Stdio};

    // ── Helpers ────────────────────────────────────────────────────────────

    /// A fresh, canonical folder: canonical because /var is a link to
    /// /private/var and the kernel reports executables by the real path.
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "proxysvpn-pidfile-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::canonicalize(&dir).expect("canonical temp dir")
    }

    fn file_at(dir: &Path) -> PidFile {
        PidFile {
            dirs: vec![dir.to_path_buf()],
        }
    }

    fn entry(pid: u32, name: &str, path: &str) -> Entry {
        Entry {
            pid,
            name: name.into(),
            path: PathBuf::from(path),
        }
    }

    const STAND_IN_MODE: &str = "PROXYSVPN_PIDFILE_STAND_IN";
    const STAND_IN_READY: &str = "PROXYSVPN_PIDFILE_STAND_IN_READY";

    /// Not a test of its own: the body of the stand-in engine the process
    /// tests start. It is this very test binary, copied under an engine's
    /// name (a copied system binary is killed by the OS on launch) and run
    /// with a filter that selects only this function.
    #[test]
    #[ignore = "runs only as the stand-in engine started by the tests below"]
    fn stand_in_engine() {
        match std::env::var(STAND_IN_MODE).as_deref() {
            Ok("stubborn") => {
                // SAFETY: setting a signal disposition to SIG_IGN installs no
                // handler code and touches no memory of ours.
                unsafe {
                    libc::signal(libc::SIGTERM, libc::SIG_IGN);
                }
            }
            Ok("plain") => {}
            _ => return,
        }
        if let Ok(ready) = std::env::var(STAND_IN_READY) {
            std::fs::write(ready, b"up").expect("ready marker");
        }
        std::thread::sleep(Duration::from_secs(30));
    }

    /// This test binary, copied into `dir` as `name`.
    fn install_stand_in(dir: &Path, name: &str) -> PathBuf {
        let target = dir.join(name);
        std::fs::copy(std::env::current_exe().expect("test binary"), &target)
            .expect("copy stand-in");
        std::fs::canonicalize(&target).expect("canonical stand-in")
    }

    /// A running stand-in that never outlives its test, even a failed one.
    struct StandIn(StdChild);

    impl StandIn {
        fn start(bin: &Path, mode: &str) -> Self {
            let ready = bin.with_extension(format!("ready-{mode}"));
            let _ = std::fs::remove_file(&ready);
            let child = StdCommand::new(bin)
                .args([
                    "pidfile::tests::stand_in_engine",
                    "--exact",
                    "--ignored",
                    "--test-threads=1",
                    "-q",
                ])
                .env(STAND_IN_MODE, mode)
                .env(STAND_IN_READY, &ready)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start stand-in");
            let started = Self(child);
            // Signalled before it has set up, a stubborn stand-in would die
            // like a plain one and the test would prove nothing.
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.exists() {
                assert!(Instant::now() < deadline, "stand-in did not come up");
                std::thread::sleep(Duration::from_millis(20));
            }
            started
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }

        fn is_alive(&mut self) -> bool {
            matches!(self.0.try_wait(), Ok(None))
        }

        /// SIGKILL is delivered, not awaited: the exit lands a moment later.
        fn exits_within(&mut self, limit: Duration) -> bool {
            let deadline = Instant::now() + limit;
            while self.is_alive() {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            true
        }
    }

    impl Drop for StandIn {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn outcome_of(results: &[(Entry, Outcome)], pid: u32) -> Option<Outcome> {
        results.iter().find(|(e, _)| e.pid == pid).map(|(_, o)| *o)
    }

    // ── The file ───────────────────────────────────────────────────────────

    #[test]
    fn a_written_file_reads_back_the_same_and_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("roundtrip");
        let file = file_at(&dir);
        file.record(
            "xray",
            4242,
            Path::new("/Applications/P.app/Contents/MacOS/xray"),
        );
        file.record(
            "tun2socks",
            4243,
            Path::new("/Applications/P.app/Contents/MacOS/tun2socks"),
        );

        let all = file.load();
        assert_eq!(
            all,
            vec![
                entry(4242, "xray", "/Applications/P.app/Contents/MacOS/xray"),
                entry(
                    4243,
                    "tun2socks",
                    "/Applications/P.app/Contents/MacOS/tun2socks"
                ),
            ]
        );
        let mode = std::fs::metadata(dir.join(FILE_NAME))
            .expect("file")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "the file names our processes; nobody else reads it"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != FILE_NAME)
            .collect();
        assert!(
            leftovers.is_empty(),
            "the temp file of the atomic write must not stay behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_pid_twice_is_one_line_and_forgetting_the_last_removes_the_file() {
        let dir = temp_dir("forget");
        let file = file_at(&dir);
        file.record("xray", 5000, Path::new("/a/xray"));
        file.record("hysteria", 5000, Path::new("/a/hysteria"));
        assert_eq!(file.load(), vec![entry(5000, "hysteria", "/a/hysteria")]);

        file.forget(5000);
        assert!(file.load().is_empty());
        assert!(
            !dir.join(FILE_NAME).exists(),
            "an empty list leaves no file"
        );
        file.forget(5000); // forgetting what is not there is harmless
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_ignored_and_replaced_by_the_next_write() {
        let dir = temp_dir("corrupt");
        let file = file_at(&dir);
        write_atomic(
            &dir.join(FILE_NAME),
            b"{\"version\":1,\"engines\":[{\"pid\":12",
        )
        .expect("write");
        assert!(file.load().is_empty());

        file.record("xray", 6000, Path::new("/a/xray"));
        assert_eq!(file.load(), vec![entry(6000, "xray", "/a/xray")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_version_is_not_read() {
        let text = r#"{"version":2,"engines":[{"pid":4242,"name":"xray","path":"/a/xray"}]}"#;
        assert_eq!(parse(text), Err(Unreadable::UnknownVersion));
        assert_eq!(parse("not json"), Err(Unreadable::NotJson));
        assert_eq!(parse(""), Err(Unreadable::NotJson));
    }

    #[test]
    fn lines_that_could_hurt_are_dropped_and_the_rest_survive() {
        let own = std::process::id();
        let text = format!(
            r#"{{"version":1,"engines":[
                {{"pid":0,"name":"xray","path":"/a/xray"}},
                {{"pid":1,"name":"xray","path":"/a/xray"}},
                {{"pid":-1,"name":"xray","path":"/a/xray"}},
                {{"pid":{own},"name":"xray","path":"/a/xray"}},
                {{"pid":2147483648,"name":"xray","path":"/a/xray"}},
                {{"pid":7001,"name":"launchd","path":"/sbin/launchd"}},
                {{"pid":7002,"name":"xray","path":"a/xray"}},
                {{"pid":7003,"name":"xray","path":"/Applications/Safari.app/Contents/MacOS/Safari"}},
                {{"pid":7004,"name":"xray","path":"/a/xrayfake"}},
                {{"pid":"7005","name":"xray","path":"/a/xray"}},
                {{"pid":7006,"name":"xray","path":"/a/xray-aarch64-apple-darwin"}},
                {{"pid":7007,"name":"sing-box","path":"/a/sing-box"}},
                {{"pid":7007,"name":"xray","path":"/a/xray"}}
            ]}}"#
        );
        let (entries, dropped) = parse(&text).expect("readable");
        assert_eq!(
            entries,
            vec![
                entry(7006, "xray", "/a/xray-aarch64-apple-darwin"),
                entry(7007, "sing-box", "/a/sing-box"),
            ]
        );
        assert_eq!(dropped, 11);
    }

    #[test]
    fn a_pid_that_means_a_group_everyone_launchd_or_us_is_never_signalled() {
        assert_eq!(signal_pid(0), None);
        assert_eq!(signal_pid(1), None);
        assert_eq!(
            signal_pid(u32::MAX),
            None,
            "would be -1 as pid_t: every process"
        );
        assert_eq!(signal_pid(1 << 31), None);
        assert_eq!(signal_pid(std::process::id()), None);
        assert_eq!(signal_pid(4242), Some(4242));
        assert!(matches!(send(u32::MAX, 0), Sent::Refused));
    }

    #[test]
    fn a_file_others_could_write_is_not_believed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("untrusted");
        let file = file_at(&dir);
        file.record("xray", 8000, Path::new("/a/xray"));
        std::fs::set_permissions(dir.join(FILE_NAME), std::fs::Permissions::from_mode(0o666))
            .expect("chmod");
        assert!(
            file.load().is_empty(),
            "a world-writable list of pids to kill is nobody's list"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_in_place_of_the_file_is_not_followed() {
        let dir = temp_dir("symlink");
        let real = temp_dir("symlink-target");
        file_at(&real).record("xray", 8100, Path::new("/a/xray"));
        std::os::unix::fs::symlink(real.join(FILE_NAME), dir.join(FILE_NAME)).expect("symlink");
        assert!(file_at(&dir).load().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&real);
    }

    #[test]
    fn the_first_usable_folder_wins_and_the_others_are_merged_into_it() {
        let base = temp_dir("dirs");
        // A folder that cannot exist: its parent is a regular file.
        std::fs::write(base.join("blocker"), b"").expect("blocker");
        let blocked = base.join("blocker").join("sub");
        let first = base.join("first");
        let second = base.join("second");

        let only_second = PidFile {
            dirs: vec![blocked.clone(), second.clone()],
        };
        only_second.record("xray", 9000, Path::new("/a/xray"));
        assert!(
            second.join(FILE_NAME).exists(),
            "falls back past a folder it cannot create"
        );

        let both = PidFile {
            dirs: vec![first.clone(), second.clone()],
        };
        both.record("hysteria", 9001, Path::new("/a/hysteria"));
        assert!(first.join(FILE_NAME).exists());
        assert!(
            !second.join(FILE_NAME).exists(),
            "one file holds everything after a save"
        );
        assert_eq!(
            file_at(&first).load(),
            vec![
                entry(9000, "xray", "/a/xray"),
                entry(9001, "hysteria", "/a/hysteria")
            ]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    // ── The ownership check ────────────────────────────────────────────────

    #[test]
    fn only_our_own_binary_from_our_own_folder_is_ours() {
        let own = Path::new("/Applications/ProxysVPN.app/Contents/MacOS");
        let line = entry(
            4242,
            "xray",
            "/Applications/ProxysVPN.app/Contents/MacOS/xray",
        );

        assert_eq!(judge(&line, None, own), Verdict::Gone);
        assert_eq!(
            judge(
                &line,
                Some(Path::new("/Applications/Other VPN.app/Contents/MacOS/xray")),
                own
            ),
            Verdict::Reused,
            "same name, another client: the reason this file exists"
        );
        assert_eq!(
            judge(&line, Some(Path::new("/usr/libexec/xpcproxy")), own),
            Verdict::Reused
        );
        assert_eq!(
            judge(
                &line,
                Some(Path::new("/Applications/ProxysVPN.app/Contents/MacOS/xray")),
                own
            ),
            Verdict::Ours
        );

        let elsewhere = entry(
            4243,
            "xray",
            "/Volumes/ProxysVPN/ProxysVPN.app/Contents/MacOS/xray",
        );
        assert_eq!(
            judge(&elsewhere, Some(&elsewhere.path), own),
            Verdict::OtherCopy,
            "our binary from another copy of the app is left to that copy"
        );
    }

    /// macOS as root runs the engines from a root-owned copy
    /// (engine_stage.rs), not from the bundle. Those must still count as ours,
    /// or a stop would leave them running and a launch after a crash would
    /// never clean them up; and a copy of another app folder must not.
    #[test]
    fn engines_run_from_this_copys_staged_folder_are_ours_too() {
        let bundle = PathBuf::from("/Applications/ProxysVPN.app/Contents/MacOS");
        let staged = PathBuf::from("/Library/Application Support/ProxysVPN/engines/0123456789abcdef");
        let own = [bundle.clone(), staged.clone()];

        let from_stage = entry(4244, "xray", &staged.join("xray").to_string_lossy());
        assert_eq!(judge_in(&from_stage, Some(&from_stage.path), &own), Verdict::Ours);
        let from_bundle = entry(4245, "xray", &bundle.join("xray").to_string_lossy());
        assert_eq!(judge_in(&from_bundle, Some(&from_bundle.path), &own), Verdict::Ours);

        let other = entry(4246, "xray", "/Library/Application Support/ProxysVPN/engines/ffffffffffffffff/xray");
        assert_eq!(judge_in(&other, Some(&other.path), &own), Verdict::OtherCopy);
        assert_eq!(judge_in(&other, None, &own), Verdict::Gone);
        assert_eq!(judge_in(&other, Some(Path::new("/usr/bin/true")), &own), Verdict::Reused);
        assert_eq!(judge_in(&other, Some(&other.path), &[]), Verdict::OtherCopy);
    }

    // ── Real processes ─────────────────────────────────────────────────────

    #[test]
    fn a_recorded_engine_is_stopped_and_an_unrelated_namesake_survives() {
        let base = temp_dir("namesake");
        let ours_dir = base.join("ours");
        let theirs_dir = base.join("theirs");
        std::fs::create_dir_all(&ours_dir).expect("ours");
        std::fs::create_dir_all(&theirs_dir).expect("theirs");
        let ours_bin = install_stand_in(&ours_dir, "sing-box");
        let theirs_bin = install_stand_in(&theirs_dir, "sing-box");

        let mut ours = StandIn::start(&ours_bin, "plain");
        let mut theirs = StandIn::start(&theirs_bin, "plain");
        let file = file_at(&base.join("state"));
        file.record("sing-box", ours.pid(), &ours_bin);

        let results = file.reap(std::slice::from_ref(&ours_dir), None, Duration::from_secs(2));
        assert_eq!(outcome_of(&results, ours.pid()), Some(Outcome::Terminated));
        // Terminated means the kernel no longer shows our binary at that pid;
        // the exit itself can land a moment later.
        assert!(ours.exits_within(Duration::from_secs(2)));
        assert!(
            theirs.is_alive(),
            "another program's sing-box is not ours to stop"
        );
        assert_eq!(
            outcome_of(&results, theirs.pid()),
            None,
            "it was never even looked at"
        );
        assert!(file.load().is_empty(), "the line of a stopped engine goes");
        drop(theirs);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_engine_that_ignores_sigterm_is_killed_after_the_grace_period() {
        let base = temp_dir("stubborn");
        let bin = install_stand_in(&base, "xray");
        let mut stubborn = StandIn::start(&bin, "stubborn");
        let file = file_at(&base.join("state"));
        file.record("xray", stubborn.pid(), &bin);

        let started = Instant::now();
        let results = file.reap(std::slice::from_ref(&base), None, Duration::from_millis(300));
        assert_eq!(outcome_of(&results, stubborn.pid()), Some(Outcome::Killed));
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "SIGKILL only after the grace"
        );
        assert!(stubborn.exits_within(Duration::from_secs(2)));
        assert!(file.load().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_pid_now_running_something_else_is_left_alone_and_forgotten() {
        let base = temp_dir("reused");
        let ours_dir = base.join("ours");
        let theirs_dir = base.join("theirs");
        std::fs::create_dir_all(&ours_dir).expect("ours");
        std::fs::create_dir_all(&theirs_dir).expect("theirs");
        let theirs_bin = install_stand_in(&theirs_dir, "xray");
        let mut theirs = StandIn::start(&theirs_bin, "plain");
        // Our line, their pid: what a reused pid looks like after a reboot.
        let file = file_at(&base.join("state"));
        file.record("xray", theirs.pid(), &ours_dir.join("xray"));

        let results = file.reap(std::slice::from_ref(&ours_dir), None, Duration::from_secs(1));
        assert_eq!(outcome_of(&results, theirs.pid()), Some(Outcome::NotOurs));
        assert!(theirs.is_alive());
        assert!(
            file.load().is_empty(),
            "a line that points at a stranger is worthless"
        );
        drop(theirs);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn another_copy_of_the_app_is_left_alone_and_its_line_kept() {
        let base = temp_dir("other-copy");
        let ours_dir = base.join("ours");
        let other_dir = base.join("other");
        std::fs::create_dir_all(&ours_dir).expect("ours");
        std::fs::create_dir_all(&other_dir).expect("other");
        let other_bin = install_stand_in(&other_dir, "tun2socks");
        let mut other = StandIn::start(&other_bin, "plain");
        let file = file_at(&base.join("state"));
        file.record("tun2socks", other.pid(), &other_bin);

        let results = file.reap(std::slice::from_ref(&ours_dir), None, Duration::from_secs(1));
        assert_eq!(outcome_of(&results, other.pid()), Some(Outcome::OtherCopy));
        assert!(other.is_alive());
        assert_eq!(file.load().len(), 1, "that copy still needs its line");
        drop(other);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_dead_pid_is_dropped_as_stale() {
        let base = temp_dir("stale");
        let bin = install_stand_in(&base, "hysteria");
        let gone = StandIn::start(&bin, "plain");
        let pid = gone.pid();
        drop(gone); // killed and reaped
        let file = file_at(&base.join("state"));
        file.record("hysteria", pid, &bin);

        let results = file.reap(std::slice::from_ref(&base), None, Duration::from_secs(1));
        assert_eq!(outcome_of(&results, pid), Some(Outcome::Stale));
        assert!(file.load().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_sweep_of_one_engine_leaves_the_others_running_and_recorded() {
        let base = temp_dir("only");
        let xray_bin = install_stand_in(&base, "xray");
        let tun_bin = install_stand_in(&base, "tun2socks");
        let mut xray = StandIn::start(&xray_bin, "plain");
        let mut tun = StandIn::start(&tun_bin, "plain");
        let file = file_at(&base.join("state"));
        file.record("xray", xray.pid(), &xray_bin);
        file.record("tun2socks", tun.pid(), &tun_bin);

        let results = file.reap(std::slice::from_ref(&base), Some("xray"), Duration::from_secs(2));
        assert_eq!(results.len(), 1);
        assert!(xray.exits_within(Duration::from_secs(2)));
        assert!(tun.is_alive());
        assert_eq!(
            file.load(),
            vec![entry(
                tun.pid(),
                "tun2socks",
                tun_bin.to_str().expect("utf-8")
            )]
        );
        drop(tun);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn a_held_engine_stops_gracefully_and_takes_its_line_with_it() {
        let base = temp_dir("held");
        let bin = install_stand_in(&base, "hysteria");
        let file = file_at(&base.join("state"));
        let ready = base.join("hysteria.ready-held");

        let mut cmd = Command::new(&bin);
        cmd.args([
            "pidfile::tests::stand_in_engine",
            "--exact",
            "--ignored",
            "--test-threads=1",
            "-q",
        ])
        .env(STAND_IN_MODE, "plain")
        .env(STAND_IN_READY, &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        let mut engine =
            Engine::spawn_with(file.clone(), "hysteria", &mut cmd, &bin).expect("spawn");
        let pid = engine.pid.expect("pid");
        assert_eq!(
            file.load(),
            vec![entry(pid, "hysteria", bin.to_str().expect("utf-8"))]
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "stand-in did not come up");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(
            engine.terminate(Duration::from_secs(2)).await,
            Outcome::Terminated
        );
        assert!(exe_of(pid).is_none(), "the process is gone");
        drop(engine);
        assert!(
            file.load().is_empty(),
            "dropping the handle forgets the line"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn a_held_engine_that_ignores_sigterm_is_killed() {
        let base = temp_dir("held-stubborn");
        let bin = install_stand_in(&base, "xray");
        let file = file_at(&base.join("state"));
        let ready = base.join("xray.ready-held");

        let mut cmd = Command::new(&bin);
        cmd.args([
            "pidfile::tests::stand_in_engine",
            "--exact",
            "--ignored",
            "--test-threads=1",
            "-q",
        ])
        .env(STAND_IN_MODE, "stubborn")
        .env(STAND_IN_READY, &ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        let engine = Engine::spawn_with(file.clone(), "xray", &mut cmd, &bin).expect("spawn");
        let pid = engine.pid.expect("pid");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "stand-in did not come up");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        engine.stop_within(Duration::from_millis(300)).await;
        assert!(exe_of(pid).is_none(), "SIGKILL after the grace period");
        assert!(file.load().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
