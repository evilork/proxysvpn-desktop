// src-tauri/crates/pvpn-platform/src/helper/server.rs
//! Helper main loop: line-delimited JSON on stdin/stdout, one request at a time.

//! Never call `crate::log` from this file or anything it drives: its fallback
//! mirrors to stdout, and stdout here *is* the protocol pipe. Use `emit_log`.
//!
//! Note the asymmetry that makes that trap real: `crate::log`'s fallback sink
//! prints to stdout, so a single stray `crate::log::info` from inside the helper
//! would inject a non-JSON line into the pipe and the GUI would log a parse
//! warning instead of the message.

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use super::proto::{
    self, invoking_uid_from_env, sidecar_is_trusted, validate_up, FileFacts, Frame, Request,
};
use super::{HELPER_DEV_FLAG, HELPER_FLAG};
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

fn dev_mode() -> bool {
    std::env::args().skip(1).any(|arg| arg == HELPER_DEV_FLAG)
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

/// The helper execs a path it was handed by an unprivileged peer, so the file
/// must be no easier to tamper with than the helper binary itself. See
/// `proto::sidecar_is_trusted`.
fn check_sidecar(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::metadata(path)
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
            check_sidecar(&valid.tun2socks)?;
            if let Some(active) = guard.take() {
                net::down(active);
            }
            let fresh = net::up(&valid).map_err(|e| format!("{:#}", e))?;
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
            net::ensure(active).map_err(|e| format!("{:#}", e))?;
            Ok(true)
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
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
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
