// src-tauri/src/xray_manager.rs
use anyhow::{anyhow, Context, Result};
use pvpn_platform::{process as pprocess, triple};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct XrayState {
    child: Option<Child>,
}

pub type SharedXrayState = Arc<Mutex<XrayState>>;

pub fn new_state() -> SharedXrayState {
    Arc::new(Mutex::new(XrayState::default()))
}

/// Locates the xray binary and the directory holding geoip.dat/geosite.dat.
///
/// Both come from the same ordered directory list as every other sidecar
/// (crate::tun::sidecar_dirs), so a bundle, a dev checkout and a Windows
/// install all resolve with one rule instead of three hand-written lists.
pub fn xray_paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf)> {
    let dirs = crate::tun::sidecar_dirs(app);
    let bin = triple::find_sidecar("xray", &dirs)?;

    let assets = dirs
        .iter()
        .find(|d| d.join("geoip.dat").is_file())
        .map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.clone()))
        .or_else(|| bin.parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| {
            anyhow!(
                "geoip.dat not found and {} has no parent directory",
                bin.display()
            )
        })?;

    crate::logger::log("info", "xray", &format!("binary: {}", bin.display()));
    crate::logger::log("info", "xray", &format!("assets: {}", assets.display()));
    Ok((bin, assets))
}

pub async fn start(
    state: &SharedXrayState,
    bin: &Path,
    assets_dir: &Path,
    config: Value,
) -> Result<()> {
    let mut guard = state.lock().await;
    if guard.child.is_some() {
        return Err(anyhow!("xray already running"));
    }

    let config_str = serde_json::to_string(&config)?;

    let mut cmd = Command::new(bin);
    cmd.arg("run")
        .arg("-config")
        .arg("stdin:")
        .env("XRAY_LOCATION_ASSET", assets_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Without this a console window pops up on Windows for every engine start.
    pprocess::no_window(&mut cmd);

    let mut child = cmd.spawn().context("failed to spawn xray process")?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow!("xray stdin not captured"))?;
    let mut stdin = stdin;
    stdin.write_all(config_str.as_bytes()).await?;
    stdin.shutdown().await?;

    if let Some(out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                crate::logger::log("info", "xray", &line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                crate::logger::log("warn", "xray", &line);
            }
        });
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    match child.try_wait() {
        Ok(Some(status)) => return Err(anyhow!("xray exited with status: {}", status)),
        Ok(None) => {}
        Err(e) => return Err(anyhow!("xray try_wait failed: {}", e)),
    }

    guard.child = Some(child);
    Ok(())
}

pub async fn stop(state: &SharedXrayState) -> Result<()> {
    let mut guard = state.lock().await;
    if let Some(mut child) = guard.child.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    Ok(())
}

pub async fn is_running(state: &SharedXrayState) -> bool {
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
