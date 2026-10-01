// src-tauri/src/paths.rs
//! Single source of truth for build triples and every filesystem path the app
//! uses. Before this module the triple was computed in three places (tun,
//! xray_manager, hysteria_manager) with three different — and partly wrong —
//! answers, and the paths were hardcoded macOS literals.
//!
//! macOS paths are deliberately byte-identical to the previous hardcoded ones:
//! a release built from this branch must find the same files as before.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use tauri::Manager;

/// Rust target triple of the running binary. Used to locate sidecars that
/// Tauri ships as `<name>-<triple>` in dev and as `<name>` in a bundle.
pub fn target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else {
        "unknown"
    }
}

/// Executable suffix for the host. Tauri appends it *after* the triple.
pub fn exe_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    }
}

/// Every place a sidecar may live, most specific first.
///
/// Pure function so it can be unit-tested without an `AppHandle`:
///   * `exe_dir`    — directory of the running binary (bundle layout),
///   * `resource_dir` — Tauri resource directory, if resolvable,
///   * `manifest_dir` — `CARGO_MANIFEST_DIR` in `cargo run`.
pub fn sidecar_candidates(
    name: &str,
    exe_dir: Option<&std::path::Path>,
    resource_dir: Option<&std::path::Path>,
    manifest_dir: Option<&std::path::Path>,
) -> Vec<PathBuf> {
    let triple = target_triple();
    let suffix = exe_suffix();
    let plain = format!("{}{}", name, suffix);
    let tripled = format!("{}-{}{}", name, triple, suffix);
    let mut out: Vec<PathBuf> = Vec::new();

    if let Some(dir) = exe_dir {
        out.push(dir.join(&plain));
        out.push(dir.join(&tripled));
    }
    if let Some(dir) = resource_dir {
        out.push(dir.join(&plain));
        out.push(dir.join(&tripled));
        out.push(dir.join("binaries").join(&plain));
        out.push(dir.join("binaries").join(&tripled));
    }
    if let Some(dir) = manifest_dir {
        out.push(dir.join("binaries").join(&tripled));
    }
    out
}

/// Resolve a bundled sidecar binary (`xray`, `tun2socks`, `hysteria`).
pub fn sidecar_path(app: &tauri::AppHandle, name: &str) -> Result<PathBuf> {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_ref().and_then(|p| p.parent());
    let resource_dir = app.path().resource_dir().ok();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from);

    let candidates = sidecar_candidates(
        name,
        exe_dir,
        resource_dir.as_deref(),
        manifest_dir.as_deref(),
    );

    candidates
        .iter()
        .find(|p| p.exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .ok_or_else(|| anyhow!("{} binary not found; tried: {:?}", name, candidates))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var(key).ok().filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Rotating text log mirrored from the in-memory ring buffer.
pub fn log_file() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(home_dir()?.join("Library/Logs/ProxysVPN/app.log"))
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var("XDG_STATE_HOME")
            .ok()
            .filter(|s| s.starts_with('/'))
            .map(PathBuf::from)
            .or_else(|| Some(home_dir()?.join(".local/state")))?;
        Some(base.join("ProxysVPN/logs/app.log"))
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("LOCALAPPDATA")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| Some(home_dir()?.join("AppData\\Local")))?;
        Some(base.join("ProxysVPN\\logs\\app.log"))
    }
}

/// hysteria client config. Holds the node password, so it must never be
/// world-readable (callers chmod it to 0600 on unix).
pub fn hysteria_config() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        // Unchanged from the pre-split build: a fixed path, because under
        // `sudo` TMPDIR differs from the user's.
        PathBuf::from("/tmp/proxysvpn-hy2.yaml")
    }
    #[cfg(target_os = "linux")]
    {
        // hysteria runs unprivileged on Linux, so the per-user runtime dir
        // (mode 0700, tmpfs) is the right home for a file with a password.
        if let Some(dir) = std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|s| s.starts_with('/'))
        {
            return PathBuf::from(dir).join("proxysvpn").join("hy2.yaml");
        }
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/proxysvpn-hy2-{}.yaml", uid))
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("PROGRAMDATA")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "C:\\ProgramData".to_string());
        PathBuf::from(base).join("ProxysVPN").join("hy2.yaml")
    }
}

/// Crash breadcrumb: the node IP whose host route must be removed if we die
/// without running teardown.
pub fn route_hint() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        // Same file (and same `pid=`/`server_ip=` format) as before the split.
        PathBuf::from("/tmp/proxysvpn-desktop.pid")
    }
    #[cfg(target_os = "linux")]
    {
        linux_runtime_dir().join("route-hint")
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("PROGRAMDATA")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "C:\\ProgramData".to_string());
        PathBuf::from(base).join("ProxysVPN").join("route-hint")
    }
}

/// Root-owned state of the privileged Linux helper: route hint and the
/// `/etc/resolv.conf` backup. `/run` is a tmpfs, so a reboot cleans it.
#[cfg(target_os = "linux")]
pub fn linux_runtime_dir() -> PathBuf {
    PathBuf::from("/run/proxysvpn")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn triple_is_known_for_this_host() {
        assert_ne!(target_triple(), "unknown", "host triple must be mapped");
    }

    #[test]
    fn triple_has_no_exe_suffix_baked_in() {
        assert!(!target_triple().ends_with(".exe"));
    }

    #[test]
    fn candidates_prefer_exe_dir_then_resources_then_manifest() {
        let got = sidecar_candidates(
            "tun2socks",
            Some(Path::new("/app/bin")),
            Some(Path::new("/app/res")),
            Some(Path::new("/src")),
        );
        let plain = format!("tun2socks{}", exe_suffix());
        assert_eq!(got[0], PathBuf::from("/app/bin").join(&plain));
        assert_eq!(got[1], PathBuf::from("/app/bin").join(format!("tun2socks-{}{}", target_triple(), exe_suffix())));
        assert!(got.iter().any(|p| p.starts_with("/app/res")));
        assert!(got.last().unwrap().starts_with("/src"));
    }

    #[test]
    fn candidates_skip_missing_roots() {
        let got = sidecar_candidates("xray", None, None, None);
        assert!(got.is_empty());
    }

    #[test]
    fn hysteria_config_is_absolute() {
        assert!(hysteria_config().is_absolute());
    }

    #[test]
    fn route_hint_is_absolute() {
        assert!(route_hint().is_absolute());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_paths_are_unchanged() {
        assert_eq!(hysteria_config(), PathBuf::from("/tmp/proxysvpn-hy2.yaml"));
        assert_eq!(route_hint(), PathBuf::from("/tmp/proxysvpn-desktop.pid"));
        if let Some(home) = home_dir() {
            assert_eq!(
                log_file().expect("log path"),
                home.join("Library/Logs/ProxysVPN/app.log")
            );
        }
    }
}
