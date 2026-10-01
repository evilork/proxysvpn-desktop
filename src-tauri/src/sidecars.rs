// src-tauri/src/sidecars.rs
//
// Where the engines live on disk: xray (with geoip.dat/geosite.dat), hysteria
// and tun2socks, on every desktop platform. One ordered directory list and one
// naming rule (`pvpn_platform::triple::find_sidecar`: `xray`, then
// `xray-<triple>`, with `.exe` on Windows) instead of a hand-written list per
// engine and per platform.

use std::path::PathBuf;

use tauri::Manager;

use crate::errors::{AppError, ErrorCode};

/// Directories a sidecar may live in, most specific first: next to the
/// executable (how a bundle ships it), then Tauri's resource directory, then
/// the repo layout used by `cargo tauri dev`.
///
/// macOS as root: only the root-owned copy made at launch
/// (`engine_stage.rs`), never the user-owned bundle; nothing at all when that
/// copy could not be made.
pub fn dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    match crate::engine_stage::staged() {
        Some(Ok(home)) => return vec![home.clone()],
        Some(Err(_)) => return Vec::new(),
        None => {}
    }

    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            // In a macOS bundle the sidecars sit in Contents/MacOS next to the
            // executable, while Tauri's own resources land in Contents/Resources.
            if let Some(contents) = dir.parent() {
                dirs.push(contents.join("Resources"));
                dirs.push(contents.join("Resources").join("_up_").join("binaries"));
            }
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        dirs.push(resource_dir.clone());
        dirs.push(resource_dir.join("binaries"));
        dirs.push(resource_dir.join("_up_").join("binaries"));
    }
    // Debug builds only. This reads an environment variable at *runtime*, so in
    // a shipped build anyone who can set CARGO_MANIFEST_DIR in our environment
    // could add a directory to the sidecar search — and on macOS and Windows
    // the process that execs from it is root/administrator.
    #[cfg(debug_assertions)]
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        dirs.push(PathBuf::from(manifest_dir).join("binaries"));
    }
    dirs
}

/// The sidecar called `stem`, or ENGINE_START_FAILED with every path that was
/// tried in the log under `source`. The list is a diagnostic, not a sentence
/// for a person: the window shows the translated phrase with one button.
pub fn find(app: &tauri::AppHandle, stem: &str, source: &str) -> Result<PathBuf, AppError> {
    #[cfg(target_os = "macos")]
    if let Some(Err(reason)) = crate::engine_stage::staged() {
        crate::logger::log(
            "error",
            source,
            &format!("{stem} not started: the engines were not copied at launch ({reason}); relaunch the app"),
        );
        return Err(AppError::new(ErrorCode::EngineStartFailed));
    }
    pvpn_platform::triple::find_sidecar(stem, &dirs(app)).map_err(|e| {
        crate::logger::log("error", source, &format!("{e:#}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })
}
