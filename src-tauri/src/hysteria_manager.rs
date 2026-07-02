// src-tauri/src/hysteria_manager.rs
// Manages the hysteria2 client process: writes a YAML config and runs
// hysteria in client mode, exposing a SOCKS5 inbound that tun2socks consumes.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tauri::Manager;
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

fn current_target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        "unknown"
    }
}

pub fn hysteria_path(app: &tauri::AppHandle) -> Result<PathBuf> {
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
        candidates.push(resource_dir.join("binaries").join(format!("hysteria-{}", triple)));
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        candidates.push(base.join("binaries").join(format!("hysteria-{}", triple)));
    }

    candidates
        .iter()
        .find(|p| p.exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .ok_or_else(|| anyhow!("hysteria binary not found; tried: {:?}", candidates))
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

    // Фиксированный путь (под sudo TMPDIR может отличаться).
    let cfg_path = std::path::PathBuf::from("/tmp/proxysvpn-hy2.yaml");
    std::fs::write(&cfg_path, &yaml).context("write hysteria config")?;

    crate::logger::log("info", "hysteria", &format!("config written: {}", cfg_path.display()));
    crate::logger::log("info", "hysteria", &format!("server {}:{}", cfg.host, cfg.port));
    crate::logger::log("info", "hysteria", &format!("binary: {}", bin.display()));

    let mut cmd = Command::new(&bin);
    cmd.args(["client", "-c", cfg_path.to_str().unwrap()])
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
    let _ = Command::new("/usr/bin/pkill")
        .args(["-x", "hysteria"])
        .status()
        .await;
    Ok(())
}

#[allow(dead_code)]
pub async fn is_running(state: &SharedHysteriaState) -> bool {
    state.lock().await.child.is_some()
}
