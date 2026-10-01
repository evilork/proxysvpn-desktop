// src-tauri/src/hysteria_manager.rs
// Manages the hysteria2 client process: writes a YAML config and runs
// hysteria in client mode, exposing a SOCKS5 inbound that tun2socks consumes.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::subscription::Hy2Config;

/// SOCKS port for hysteria — distinct from xray's 10808 to avoid clashes.
pub const HY2_SOCKS_PORT: u16 = 10809;

#[derive(Default)]
pub struct HysteriaState {
    child: Option<Child>,
}

pub type SharedHysteriaState = Arc<Mutex<HysteriaState>>;

pub fn new_state() -> SharedHysteriaState {
    Arc::new(Mutex::new(HysteriaState::default()))
}

pub fn hysteria_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    crate::paths::sidecar_path(app, "hysteria")
}

/// Build the hysteria client YAML config.
fn build_config(cfg: &Hy2Config) -> String {
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

pub async fn start(
    state: &SharedHysteriaState,
    app: &tauri::AppHandle,
    cfg: &Hy2Config,
) -> Result<()> {
    let mut guard = state.lock().await;
    if guard.child.is_some() {
        return Err(anyhow!("hysteria already running"));
    }

    let bin = hysteria_path(app)?;
    let yaml = build_config(cfg);

    // Fixed path per platform (под sudo TMPDIR может отличаться).
    let cfg_path = crate::paths::hysteria_config();
    if let Some(dir) = cfg_path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::write(&cfg_path, &yaml).context("write hysteria config")?;
    // The file holds the node password, so it must not be world readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cfg_path, std::fs::Permissions::from_mode(0o600))
            .context("restrict hysteria config permissions")?;
    }

    crate::logger::log("info", "hysteria", &format!("config written: {}", cfg_path.display()));
    crate::logger::log("info", "hysteria", &format!("server {}:{}", cfg.host, cfg.port));
    crate::logger::log("info", "hysteria", &format!("binary: {}", bin.display()));

    let mut cmd = Command::new(&bin);
    cmd.arg("client")
        .arg("-c")
        .arg(&cfg_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().context("spawn hysteria")?;

    if let Some(out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                crate::logger::log("info", "hysteria", &line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                crate::logger::log("warn", "hysteria", &line);
            }
        });
    }

    // Give hysteria a moment to establish the SOCKS listener.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    guard.child = Some(child);
    Ok(())
}

pub async fn stop(state: &SharedHysteriaState) -> Result<()> {
    let mut guard = state.lock().await;
    if let Some(mut child) = guard.child.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    // A leftover hysteria keeps the SOCKS port busy and the next connect fails.
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/pkill").args(["-x", "hysteria"]).status().await;
    #[cfg(target_os = "linux")]
    {
        // SAFETY: getuid() takes no arguments and cannot fail.
        let uid = unsafe { libc::getuid() }.to_string();
        let _ = Command::new("pkill")
            .args(["-u", &uid, "-x", "hysteria"])
            .status()
            .await;
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn is_running(state: &SharedHysteriaState) -> bool {
    state.lock().await.child.is_some()
}
