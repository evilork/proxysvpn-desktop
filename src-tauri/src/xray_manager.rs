// src-tauri/src/xray_manager.rs
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tauri::Manager;
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

pub fn xray_paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf)> {
    let bin = crate::paths::sidecar_path(app, "xray")?;

    // geoip.dat/geosite.dat ship as Tauri resources. Their directory differs per
    // platform: Contents/Resources inside a .app, /usr/lib/<product> in a .deb,
    // the squashfs root in an AppImage, src-tauri/binaries in dev.
    let mut asset_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            asset_dirs.push(dir.to_path_buf());
            if let Some(contents) = dir.parent() {
                asset_dirs.push(contents.join("Resources"));
                asset_dirs.push(contents.join("Resources").join("_up_").join("binaries"));
            }
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        // Same order as before the platform split, so macOS keeps resolving to
        // the directory it resolved to before.
        asset_dirs.push(resource_dir.clone());
        asset_dirs.push(resource_dir.join("binaries"));
        asset_dirs.push(resource_dir.join("_up_").join("binaries"));
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        asset_dirs.push(PathBuf::from(manifest_dir).join("binaries"));
    }

    let assets = asset_dirs
        .iter()
        .find(|p| p.join("geoip.dat").exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .or_else(|| bin.parent().map(Path::to_path_buf))
        .ok_or_else(|| anyhow!("could not locate the xray geo assets"))?;

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
