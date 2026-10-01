// src-tauri/crates/pvpn-platform/src/helper/server.rs
//! Helper main loop: line-delimited JSON on stdin/stdout, one request at a time.

//! Never call `crate::log` from this file or anything it drives: its fallback
//! mirrors to stdout, and stdout here *is* the protocol pipe. Use `emit_log`.
//!
//! Note the asymmetry that makes that trap real: `crate::log`'s fallback sink
//! prints to stdout, so a single stray `crate::log::info` from inside the helper
//! would inject a non-JSON line into the pipe and the GUI would log a parse
//! warning instead of the message.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use super::proto::{
    self, invoking_uid_from_env, read_bounded_line, sidecar_is_trusted, validate_up, FileFacts,
    Frame, Line, Request,
};
use super::{HELPER_DEV_FLAG, HELPER_FLAG};
use crate::net::linux_logic;
use crate::net::linux_priv::{self as net, Tunnel};

static OUT: OnceLock<Mutex<std::io::Stdout>> = OnceLock::new();
static TUNNEL: OnceLock<Mutex<Option<Tunnel>>> = OnceLock::new();

fn out() -> &'static Mutex<std::io::Stdout> {
    OUT.get_or_init(|| Mutex::new(std::io::stdout()))
}

fn tunnel() -> &'static Mutex<Option<Tunnel>> {
    TUNNEL.get_or_init(|| Mutex::new(None))
}

/// A poisoned mutex here means a previous handler panicked. The state behind it
/// stays valid (an `Option<Tunnel>`), and refusing to clean up afterwards would
/// leave the user's routing table broken, so we deliberately recover.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn emit(frame: &Frame) {
    let Ok(line) = proto::encode(frame) else { return };
    let mut guard = lock(out());
    let _ = guard.write_all(line.as_bytes());
    let _ = guard.flush();
}

/// Send one log line to the GUI, which mirrors it into the app log.
///
/// The same privileged primitives also run inline when the whole app happens to
/// be root (sudo on X11). There is no pipe then, so fall back to the app logger
/// instead of printing protocol frames onto the user's terminal.
pub fn emit_log(level: &str, source: &str, message: &str) {
    if in_helper_process() {
        emit(&Frame::Log {
            level: level.to_string(),
            source: source.to_string(),
            message: message.to_string(),
        });
    } else {
        crate::log::log(level, source, message);
    }
}

static IN_HELPER: OnceLock<bool> = OnceLock::new();
static HELPER_STARTED: AtomicBool = AtomicBool::new(false);

/// Cached: `emit_log` runs once per sidecar log line.
fn in_helper_process() -> bool {
    HELPER_STARTED.load(Ordering::Relaxed) || *IN_HELPER.get_or_init(is_helper_invocation)
}

fn respond(id: u64, result: Result<bool, String>) {
    let frame = match result {
        Ok(engine_alive) => Frame::Response {
            id,
            ok: true,
            error: None,
            engine_alive,
        },
        Err(error) => Frame::Response {
            id,
            ok: false,
            error: Some(error),
            engine_alive: false,
        },
    };
    emit(&frame);
}

/// Were we started as the helper rather than as the GUI?
pub fn is_helper_invocation() -> bool {
    std::env::args().skip(1).any(|arg| arg == HELPER_FLAG)
}

/// `--dev` is honoured only by a debug build — see `proto::dev_mode_allowed`
/// for why an environment variable was not enough.
fn dev_mode() -> bool {
    proto::dev_mode_allowed(
        std::env::args().skip(1).any(|arg| arg == HELPER_DEV_FLAG),
        cfg!(debug_assertions),
    )
}

/// Does `path` live in the same directory as the helper executable? Then it is
/// exactly as tamper-proof as the helper itself (the AppImage case).
fn beside_helper(path: &std::path::Path) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let Ok(exe_dir) = exe
        .parent()
        .ok_or(())
        .and_then(|d| std::fs::canonicalize(d).map_err(|_| ()))
    else {
        return false;
    };
    match path.parent().map(std::fs::canonicalize) {
        Some(Ok(dir)) => dir == exe_dir,
        _ => false,
    }
}

/// The repo's `binaries` directory, for a debug `--dev` run only. A path baked
/// in at compile time, never read from the environment: in a release build
/// the branch does not exist at all.
fn dev_sidecar_dir() -> Option<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    if dev_mode() {
        return Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("binaries"),
        );
    }
    None
}

/// The tun2socks this helper runs: found next to its own executable, never
/// named by the peer (see `proto::UpParams`). `current_exe` is
/// `/proc/self/exe`, already canonical, and on the .deb its directory is the
/// root-owned `/usr/bin`, so nothing between this lookup and the exec can be
/// swapped by an unprivileged process.
fn own_sidecar() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate the helper: {e}"))?;
    let dev = dev_sidecar_dir();
    let dirs = proto::sidecar_dirs(&exe, dev.as_deref());
    let path = crate::triple::find_sidecar(proto::SIDECAR_NAME, &dirs).map_err(|e| format!("{e:#}"))?;
    check_sidecar(&path)?;
    Ok(path)
}

/// The sidecar must be no easier to tamper with than the helper binary
/// itself. See `proto::sidecar_is_trusted`.
fn check_sidecar(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    // symlink_metadata, not metadata: a link would otherwise be judged by the
    // facts of its target while `Command` re-resolves the path at exec time,
    // and the peer can replace the link in between. A link is not a regular
    // file, so `sidecar_is_trusted` refuses it outright.
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot stat {}: {}", path.display(), e))?;
    let facts = FileFacts {
        uid: meta.uid(),
        mode: meta.mode(),
        is_regular: meta.is_file(),
    };
    sidecar_is_trusted(
        &facts,
        invoking_uid_from_env(),
        dev_mode(),
        beside_helper(path),
    )
    .map_err(|e| format!("refusing to run {}: {}", path.display(), e))
}

/// Refuse to point the machine at a SOCKS port that a program of another user
/// is listening on.
///
/// tun2socks (ours, root) sends every packet of the machine to
/// 127.0.0.1:<port>. Between the stop and the start of an xray restart — a
/// location change, a revived engine — nobody holds that port, and any local
/// user could bind it and receive all of it, other users' traffic included.
/// Only root and the person who started the helper may hold it. No listener
/// at all is fine: that is the restart itself.
fn check_socks_listener(port: u16) -> Result<(), String> {
    // World-readable, and the helper is root: a read failure means no /proc,
    // where nothing else here would work either. Read as "nobody listens".
    let tcp = std::fs::read_to_string("/proc/net/tcp").unwrap_or_default();
    let tcp6 = std::fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
    let owners = linux_logic::loopback_listener_uids(&tcp, &tcp6, port);
    match linux_logic::foreign_listener(&owners, invoking_uid_from_env()) {
        None => Ok(()),
        Some(uid) => Err(format!(
            "порт {port} занят программой другого пользователя (uid {uid}) — туннель на него не направлен"
        )),
    }
}

fn handle(req: Request) -> Result<bool, String> {
    let mut guard = lock(tunnel());
    match req {
        Request::Hello => Ok(guard.as_mut().map(net::engine_alive).unwrap_or(false)),
        Request::Status => Ok(guard.as_mut().map(net::engine_alive).unwrap_or(false)),
        Request::Down => {
            if let Some(active) = guard.take() {
                net::down(active);
            }
            Ok(false)
        }
        Request::Up(params) => {
            let valid = validate_up(&params)?;
            let tun2socks = own_sidecar()?;
            check_socks_listener(valid.socks_port)?;
            if let Some(active) = guard.take() {
                net::down(active);
            }
            let fresh = net::up(&valid, &tun2socks).map_err(|e| format!("{:#}", e))?;
            *guard = Some(fresh);
            Ok(true)
        }
        Request::Ensure(params) => {
            let valid = validate_up(&params)?;
            let active = guard.as_ref().ok_or_else(|| "tunnel is not up".to_string())?;
            // A repair must be about the tunnel we actually hold, never a way to
            // aim our routes at a different address.
            if active.server_ip() != valid.server_ip {
                return Err("ensure refers to a different node than the active one".to_string());
            }
            // Someone else took the port the machine is pointed at: lower the
            // tunnel rather than keep feeding them. The GUI sees the engine
            // gone and says protection dropped; its next Up is refused for as
            // long as the stranger holds the port.
            if let Err(refusal) = check_socks_listener(valid.socks_port) {
                emit_log("error", "helper", &refusal);
                if let Some(active) = guard.take() {
                    net::down(active);
                }
                return Err(refusal);
            }
            net::ensure(active).map_err(|e| format!("{:#}", e))?;
            Ok(true)
        }
        Request::Retarget(params) => {
            // The same validation as `Up`: a retarget can point the host route
            // at nothing an `Up` could not, and it execs nothing.
            let valid = validate_up(&params)?;
            let active = guard
                .as_mut()
                .ok_or_else(|| "tunnel is not up".to_string())?;
            net::retarget(active, valid.server_ip).map_err(|e| format!("{:#}", e))?;
            Ok(net::engine_alive(active))
        }
    }
}

fn teardown() {
    let mut guard = lock(tunnel());
    if let Some(active) = guard.take() {
        net::down(active);
    }
}

/// Block the termination signals in every thread and wait for them in one
/// dedicated thread, so teardown runs as ordinary code instead of inside a
/// signal handler (where allocating or spawning a process is not allowed).
///
/// Side effect worth knowing: the sidecars we spawn afterwards inherit the
/// blocked mask, so a SIGTERM aimed at tun2socks would be ignored. We always
/// stop it with SIGKILL (`Child::kill`), and the crash purge uses `pkill -9`,
/// so nothing depends on tun2socks honouring SIGTERM.
fn watch_signals() {
    // SAFETY: sigemptyset/sigaddset initialise a stack-allocated sigset_t;
    // pthread_sigmask with SIG_BLOCK on the only thread that exists so far is
    // inherited by every thread we spawn later. No pointer outlives this scope.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, sig);
        }
        if libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
            emit_log("warn", "helper", "could not block termination signals");
            return;
        }
        std::thread::spawn(move || {
            let mut caught: libc::c_int = 0;
            // SAFETY: `set` is a valid, fully initialised sigset_t copied into
            // this thread; `caught` is a live stack slot for the output.
            let rc = libc::sigwait(&set, &mut caught);
            if rc == 0 {
                emit_log("info", "helper", &format!("signal {} — tearing down", caught));
            }
            teardown();
            std::process::exit(0);
        });
    }
}

/// Run as the privileged helper. Never returns.
pub fn run_helper() -> ! {
    if !crate::privilege::is_elevated() {
        eprintln!("proxysvpn: --helper must run as root (it is started through pkexec)");
        std::process::exit(2);
    }

    HELPER_STARTED.store(true, Ordering::Relaxed);
    watch_signals();
    net::purge_stale_sync();
    emit_log("info", "helper", "privileged helper ready");

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    loop {
        let line = match read_bounded_line(&mut reader) {
            Ok(Line::Eof) => break,
            Ok(Line::Request(line)) => line,
            Ok(Line::TooLong) => {
                emit_log("warn", "helper", "request line over the size limit, ignored");
                continue;
            }
            Err(e) => {
                emit_log("warn", "helper", &format!("stdin error: {}", e));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match proto::decode(&line) {
            Ok(Frame::Request { id, req }) => {
                let result = handle(req);
                respond(id, result);
            }
            // Only the GUI speaks on this pipe, and it only sends requests.
            Ok(_) => emit_log("warn", "helper", "ignoring a non-request frame"),
            Err(e) => emit_log("warn", "helper", &format!("undecodable request: {}", e)),
        }
    }

    // EOF: the GUI is gone.
    teardown();
    std::process::exit(0);
}
