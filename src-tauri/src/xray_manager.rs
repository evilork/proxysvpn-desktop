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
    let triple = current_target_triple();
    let mut bin_candidates: Vec<PathBuf> = Vec::new();
    let mut asset_dirs: Vec<PathBuf> = Vec::new();

    // 1. Directory of the current executable (in .app: Contents/MacOS/)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            bin_candidates.push(dir.join("xray"));
            bin_candidates.push(dir.join(format!("xray-{}", triple)));
            asset_dirs.push(dir.to_path_buf());
            // And sibling Resources/ — Tauri puts geoip.dat/geosite.dat there
            if let Some(contents) = dir.parent() {
                asset_dirs.push(contents.join("Resources"));
                asset_dirs.push(contents.join("Resources").join("_up_").join("binaries"));
            }
        }
    }

    // 2. Tauri-provided resource_dir (Resources folder)
    if let Ok(resource_dir) = app.path().resource_dir() {
        bin_candidates.push(resource_dir.join("xray"));
        bin_candidates.push(resource_dir.join(format!("xray-{}", triple)));
        bin_candidates.push(resource_dir.join("binaries").join("xray"));
        bin_candidates.push(resource_dir.join("binaries").join(format!("xray-{}", triple)));
        asset_dirs.push(resource_dir.clone());
        asset_dirs.push(resource_dir.join("binaries"));
        // Tauri resource paths with _up_ prefix
        asset_dirs.push(resource_dir.join("_up_").join("binaries"));
    }

    // 3. Dev mode — look relative to Cargo manifest
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        bin_candidates.push(base.join("binaries").join(format!("xray-{}", triple)));
        asset_dirs.push(base.join("binaries"));
    }

    let bin = bin_candidates
        .iter()
        .find(|p| p.exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .ok_or_else(|| anyhow!("xray binary not found; tried: {:?}", bin_candidates))?;

    let assets = asset_dirs
        .iter()
        .find(|p| p.join("geoip.dat").exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .unwrap_or_else(|| bin.parent().unwrap().to_path_buf());

    println!("[xray] binary: {}", bin.display());
    println!("[xray] assets: {}", assets.display());
    Ok((bin, assets))
}

fn current_target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc.exe"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unknown"
    }
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
