// src-tauri/src/pidfile_windows.rs
//
// The Windows side of pidfile.rs: the same API, a different guarantee.
//
// On macOS and Linux a crashed app leaves its engines running, so every engine
// is written down when it starts and a later run stops only what it wrote down
// (pidfile.rs explains why a name-based `pkill` was not acceptable: it killed
// other VPN clients' engines too).
//
// Windows can do better than a list: every engine is put into a job object that
// kills its members the moment the last handle to the job closes, and that
// handle is held by this process only. A clean quit, a crash and an
// "End task" in Task Manager therefore all take xray, hysteria and tun2socks
// with them, and there is nothing left behind for a later run to find. Hence
// no file here, and `sweep` / `reap_all` have nothing to do. See
// `pvpn_platform::process::tie_to_app`.

use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::process::{Child, Command};

/// How long a killed engine gets to be reaped before we stop waiting for it.
const REAP_WAIT: Duration = Duration::from_secs(2);

const LOG_SOURCE: &str = "engine";

/// A child engine process that dies with the app, whatever way the app dies.
///
/// Its pid is also registered with the platform layer for as long as this
/// handle lives (`pvpn_platform::process::note_engine`): the Windows backend
/// lets only our own engines hold the SOCKS port tun2socks sends the machine's
/// traffic to.
pub struct Engine {
    name: &'static str,
    child: Child,
    pid: Option<u32>,
}

impl Engine {
    /// Spawn `cmd` and tie the process to the app's lifetime.
    ///
    /// `_bin` is part of the shared signature: the macOS and Linux version
    /// records it to recognise the process later; here the job object makes
    /// recognising it unnecessary.
    pub fn spawn(name: &'static str, cmd: &mut Command, _bin: &Path) -> std::io::Result<Self> {
        let child = cmd.spawn()?;
        let pid = child.id();
        if let Some(pid) = pid {
            pvpn_platform::process::note_engine(pid);
        }
        if let Err(e) = pvpn_platform::process::tie_to_app(&child) {
            // The engine still runs and is still stopped by `stop` and by
            // `kill_on_drop`; only a crash of the app would now leave it behind.
            crate::logger::log(
                "warn",
                LOG_SOURCE,
                &format!("{name} is not tied to the app's lifetime: {e:#}"),
            );
        }
        Ok(Self { name, child, pid })
    }

    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Terminate and reap. Windows has no SIGTERM for a console-less child,
    /// so this is the same TerminateProcess `taskkill /F` would send.
    pub async fn stop(mut self) {
        let pid = self.child.id();
        let outcome = match self.child.try_wait() {
            Ok(Some(_)) => "stale",
            _ => match self.child.kill().await {
                Ok(()) => "killed",
                Err(_) => "denied",
            },
        };
        // `kill` already waited once; this only bounds a pathological hang.
        let _ = tokio::time::timeout(REAP_WAIT, self.child.wait()).await;
        if let Some(pid) = pid {
            crate::logger::log(
                if outcome == "denied" { "warn" } else { "info" },
                LOG_SOURCE,
                &format!("kill pid={pid} name={} result={outcome}", self.name),
            );
        }
    }
}

impl Drop for Engine {
    /// `stop` consumes the handle, so this also runs after every stop.
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            pvpn_platform::process::forget_engine(pid);
        }
    }
}

/// Nothing of ours can outlive us on Windows (see the top of this file), and
/// another program's process of the same name is not ours to stop.
pub async fn sweep(_name: &'static str) {}

/// See `sweep`.
pub fn reap_all() {}
