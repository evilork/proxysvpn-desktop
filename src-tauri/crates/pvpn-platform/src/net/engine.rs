// src-tauri/crates/pvpn-platform/src/net/engine.rs
//
// The tun2socks process, on the platforms where the app itself is privileged
// and therefore spawns it: macOS (root via launchctl) and Windows (elevated via
// the manifest). On Linux the root helper owns the process instead, so this
// module is not compiled there.
//
// The handle lives here rather than in the GUI's state because `up` has to be
// able to undo itself — if the device never appears, the child it just started
// must die inside the same function, not two layers up. The GUI only ever asks
// "is it alive".

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::log;
use crate::process::no_window;

/// The engine we started, if any. `tokio::sync::Mutex` because teardown awaits
/// the child while holding it.
static CHILD: Mutex<Option<Child>> = Mutex::const_new(None);

/// Starts tun2socks with the given argv and keeps the handle.
///
/// Any previous handle is killed first: a second `up` without a `down` would
/// otherwise leak a process that still holds the device.
pub async fn spawn(bin: &Path, args: &[String]) -> Result<()> {
    kill().await;

    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // If we are killed without running teardown, the engine goes too —
        // otherwise it keeps the device and the routes alive with nobody
        // supervising them.
        .kill_on_drop(true);
    no_window(&mut cmd);

    let mut child = cmd.spawn().context("spawn tun2socks")?;
    // Windows: a crash of the app takes the engine with it (see
    // `process::tie_to_app`); a no-op on macOS.
    if let Err(e) = crate::process::tie_to_app(&child) {
        log::warn(
            "tun2socks",
            &format!("not tied to the app's lifetime: {:#}", e),
        );
    }

    // Both streams are drained on their own tasks: a full pipe would otherwise
    // block tun2socks the moment it gets chatty.
    if let Some(out) = child.stdout.take() {
        pump(out, "info");
    }
    if let Some(err) = child.stderr.take() {
        pump(err, "warn");
    }

    *CHILD.lock().await = Some(child);
    Ok(())
}

fn pump<R>(stream: R, level: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log::log(level, "tun2socks", &line);
        }
    });
}

/// Did the engine exit on its own? Used by `up` to turn "the device never
/// appeared" into the real reason.
pub async fn exited() -> Option<std::process::ExitStatus> {
    let mut guard = CHILD.lock().await;
    match guard.as_mut() {
        Some(child) => child.try_wait().ok().flatten(),
        None => None,
    }
}

/// True while our own child is running. Deliberately does not fall back to
/// scanning the process table: a tun2socks that is not ours is a stray, and
/// reporting it as "connected" is how a half-dead tunnel looks healthy.
pub async fn alive() -> bool {
    let mut guard = CHILD.lock().await;
    match guard.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    }
}

/// Kills the engine and waits for it, so the device is really gone before the
/// caller continues. Safe to call when nothing is running.
pub async fn kill() {
    let mut guard = CHILD.lock().await;
    if let Some(mut child) = guard.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}
