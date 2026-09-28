// src-tauri/src/hysteria_manager.rs
//
// The hysteria2 client process. It is no longer the front of the chain: since
// the unified fork layer (see xray_manager.rs) tun2socks always talks to xray,
// and xray reaches a Hysteria2 node through this process's SOCKS listener.
//
//   tun2socks -> xray:10808 --socks--> hysteria:10809 -> node
//
// Two consequences worth naming. Split routing now applies on Hysteria2
// locations, where it simply did not exist before. And this process is a
// dependency of the tunnel rather than the tunnel itself, so the supervisor in
// lib.rs watches it exactly like it watches the other two, and stopping the
// VPN stops it — a hysteria that outlived the app kept port 10809 and was the
// whole of "it will not connect a second time".

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::errors::{AppError, ErrorCode};
use crate::pidfile::Engine;
use crate::subscription::Hy2Config;

/// SOCKS port for hysteria — distinct from xray's 10808 because both now run
/// at the same time, which is the point of the unified layer.
pub const HY2_SOCKS_PORT: u16 = 10809;

/// Where the client config goes.
///
/// A fixed path on purpose: under `sudo` and under `launchctl asuser` the
/// process has two different `TMPDIR`s, and a config written to one of them
/// would be invisible to a later run trying to clean it up.
const CONFIG_PATH: &str = "/tmp/proxysvpn-hy2.yaml";

/// The file holds the node's auth password and its address, so it is readable
/// by its owner and nobody else. Everything on this machine runs as root, so
/// 0600 costs nothing and keeps the secret off a shared Mac.
#[cfg(unix)]
const CONFIG_MODE: u32 = 0o600;

/// How long the SOCKS listener gets to appear before we call the start a
/// failure. Measured, not guessed: on this machine the listener is up in
/// 150-250 ms, and the old blind 600 ms sleep reported success even when
/// hysteria had already exited on a bad config.
const LISTEN_TIMEOUT: Duration = Duration::from_millis(3000);
const LISTEN_POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
pub struct HysteriaState {
    child: Option<Engine>,
}

pub type SharedHysteriaState = Arc<Mutex<HysteriaState>>;

pub fn new_state() -> SharedHysteriaState {
    Arc::new(Mutex::new(HysteriaState::default()))
}

fn current_target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        "unknown"
    }
}

pub fn hysteria_path(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    let triple = current_target_triple();
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("hysteria"));
            candidates.push(dir.join(format!("hysteria-{}", triple)));
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("hysteria"));
        candidates.push(resource_dir.join(format!("hysteria-{}", triple)));
        candidates.push(resource_dir.join("binaries").join("hysteria"));
        candidates.push(
            resource_dir
                .join("binaries")
                .join(format!("hysteria-{}", triple)),
        );
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        candidates.push(base.join("binaries").join(format!("hysteria-{}", triple)));
    }

    candidates
        .iter()
        .find(|p| p.exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .ok_or_else(|| {
            crate::logger::log(
                "error",
                "hysteria",
                &format!("binary not found; tried: {:?}", candidates),
            );
            AppError::new(ErrorCode::EngineStartFailed)
        })
}

/// Build the hysteria client YAML config.
///
/// Kept as a pure function of the node so the shape can be tested without a
/// binary, a network or a filesystem.
pub fn build_config(cfg: &Hy2Config) -> String {
    let mut yaml = String::new();
    yaml.push_str(&format!("server: {}:{}\n", cfg.host, cfg.port));
    yaml.push_str(&format!("auth: {}\n", cfg.password));
    yaml.push_str("tls:\n");
    yaml.push_str(&format!("  sni: {}\n", cfg.sni));
    if cfg.insecure {
        yaml.push_str("  insecure: true\n");
    }
    if !cfg.pin_sha256.is_empty() {
        yaml.push_str(&format!("  pinSHA256: {}\n", cfg.pin_sha256));
    }
    yaml.push_str("socks5:\n");
    yaml.push_str(&format!("  listen: 127.0.0.1:{}\n", HY2_SOCKS_PORT));
    yaml.push_str("fastOpen: true\n");
    yaml
}

fn write_config(yaml: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(CONFIG_PATH);
    std::fs::write(&path, yaml).map_err(|e| {
        crate::logger::log("error", "hysteria", &format!("config write failed: {e}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best effort: a config we could write but not lock down still starts
        // the tunnel, and refusing to connect over file permissions would be a
        // worse trade for the person in front of the screen.
        if let Err(e) =
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(CONFIG_MODE))
        {
            crate::logger::log("warn", "hysteria", &format!("chmod failed: {e}"));
        }
    }
    Ok(path)
}

pub async fn start(
    state: &SharedHysteriaState,
    app: &tauri::AppHandle,
    cfg: &Hy2Config,
) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    if guard.child.is_some() {
        crate::logger::log("error", "hysteria", "start called while already running");
        return Err(AppError::new(ErrorCode::EngineStartFailed));
    }

    // Asked before spawning, because hysteria answers a taken port by exiting
    // with a message nobody reads, and "port busy" is the one start failure
    // where pressing the button again genuinely helps once the holder is gone.
    if crate::xray_manager::port_in_use(HY2_SOCKS_PORT).await {
        crate::logger::log("error", "hysteria", "socks port is already held");
        return Err(AppError::new(ErrorCode::PortBusy));
    }

    let bin = hysteria_path(app)?;
    let cfg_path = write_config(&build_config(cfg))?;
    let cfg_arg = cfg_path.to_str().ok_or_else(|| {
        crate::logger::log("error", "hysteria", "config path is not valid utf-8");
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    // The node's address is deliberately not written here. `logger::redact`
    // would mask a numeric one anyway, but a host NAME is only masked once
    // `remember_node_host` has been told about it, and a log line is not the
    // place to find out whether that happened.
    crate::logger::log("info", "hysteria", "starting client");

    let mut cmd = Command::new(&bin);
    cmd.args(["client", "-c", cfg_arg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = Engine::spawn("hysteria", &mut cmd, &bin).map_err(|e| {
        crate::logger::log("error", "hysteria", &format!("spawn failed: {e}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    // "info" as the DEFAULT level, not as a verdict: hysteria colours its own
    // level word and logger::log honours it. Labelling this stream `warn` is
    // what used to turn the log window into an orange wall.
    pump(child.child().stdout.take());
    pump(child.child().stderr.take());

    // Success means "the port answers", not "the process was spawned". A bad
    // password or a burnt address makes hysteria exit within a moment, and the
    // old code reported that as a working tunnel.
    match await_listener(child.child()).await {
        Ok(()) => {
            guard.child = Some(child);
            Ok(())
        }
        Err(err) => {
            child.stop().await;
            Err(err)
        }
    }
}

fn pump<R>(stream: Option<R>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let Some(stream) = stream else { return };
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            crate::logger::log("info", "hysteria", &line);
        }
    });
}

/// Wait until the SOCKS listener accepts a connection, or the process dies.
async fn await_listener(child: &mut Child) -> Result<(), AppError> {
    let deadline = tokio::time::Instant::now() + LISTEN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                crate::logger::log("error", "hysteria", &format!("exited at once: {status}"));
                return Err(AppError::new(ErrorCode::EngineStartFailed));
            }
            Ok(None) => {}
            Err(e) => {
                crate::logger::log("error", "hysteria", &format!("try_wait failed: {e}"));
                return Err(AppError::new(ErrorCode::EngineStartFailed));
            }
        }

        if crate::xray_manager::port_in_use(HY2_SOCKS_PORT).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            crate::logger::log("error", "hysteria", "socks listener did not appear in time");
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
        tokio::time::sleep(LISTEN_POLL).await;
    }
}

pub async fn stop(state: &SharedHysteriaState) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    if let Some(child) = guard.child.take() {
        child.stop().await;
    }
    // A hysteria we lost the handle to holds 10809 and makes the next connect
    // fail for a reason nobody can see, so the sweep is unconditional — over
    // the hysterias WE started, not every process of that name on the Mac.
    crate::pidfile::sweep("hysteria").await;
    let _ = std::fs::remove_file(Path::new(CONFIG_PATH));
    Ok(())
}

/// Is OUR hysteria alive? Asked of the child handle, so a stray copy on the
/// machine cannot keep the screen green over a dead tunnel.
pub async fn is_running(state: &SharedHysteriaState) -> bool {
    let mut guard = state.lock().await;
    match guard.child.as_mut() {
        Some(c) => match c.try_wait() {
            Ok(None) => true,
            _ => {
                guard.child = None;
                false
            }
        },
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Hy2Config {
        Hy2Config {
            password: "secret".into(),
            host: "node.example".into(),
            port: 443,
            sni: "sni.example".into(),
            pin_sha256: String::new(),
            insecure: false,
            remark: "Нидерланды".into(),
        }
    }

    #[test]
    fn config_listens_where_xray_will_dial() {
        let yaml = build_config(&cfg());
        assert!(yaml.contains(&format!("listen: 127.0.0.1:{HY2_SOCKS_PORT}")));
        assert!(yaml.contains("server: node.example:443"));
    }

    #[test]
    fn optional_fields_stay_out_when_empty() {
        // An empty pinSHA256 written as a key makes hysteria refuse the file.
        let yaml = build_config(&cfg());
        assert!(!yaml.contains("pinSHA256"));
        assert!(!yaml.contains("insecure"));
    }

    #[test]
    fn pinned_and_insecure_nodes_are_written_out() {
        let mut c = cfg();
        c.insecure = true;
        c.pin_sha256 = "AA:BB".into();
        let yaml = build_config(&c);
        assert!(yaml.contains("insecure: true"));
        assert!(yaml.contains("pinSHA256: AA:BB"));
    }

    #[test]
    fn the_two_engines_never_share_a_port() {
        assert_ne!(HY2_SOCKS_PORT, crate::xray_manager::FRONT_SOCKS_PORT);
    }
}
