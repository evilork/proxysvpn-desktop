// src-tauri/crates/pvpn-platform/src/triple.rs
// Single source of truth for the Rust target triple and for how sidecar
// binaries are named on disk.
//
// Before the platform split this logic existed three times (tun.rs,
// xray_manager.rs, hysteria_manager.rs), each copy knowing a different subset
// of platforms, and one of them glued the executable suffix into the triple
// itself ("x86_64-pc-windows-msvc.exe"), which could never match a real file.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Tauri's `externalBin` convention: `<stem>-<triple>` with the platform's
/// executable suffix appended *after* the triple.
pub fn triple_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

pub fn exe_suffix_for(os: &str) -> &'static str {
    if os == "windows" {
        ".exe"
    } else {
        ""
    }
}

/// Triple of the running binary. `std::env::consts` is resolved at compile
/// time, so this is the build target, not a runtime guess.
pub fn target_triple() -> Result<&'static str> {
    triple_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        anyhow!(
            "unsupported platform {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })
}

pub fn exe_suffix() -> &'static str {
    exe_suffix_for(std::env::consts::OS)
}

/// File names a sidecar may have, in search order: the bundled name Tauri
/// installs (`xray.exe`) first, then the repo name (`xray-<triple>.exe`) used
/// in dev checkouts.
pub fn sidecar_file_names(stem: &str) -> Vec<String> {
    let suffix = exe_suffix();
    let mut names = vec![format!("{}{}", stem, suffix)];
    if let Ok(triple) = target_triple() {
        names.push(format!("{}-{}{}", stem, triple, suffix));
    }
    names
}

/// Resolves a sidecar by scanning `dirs` in order and, within each directory,
/// the names from [`sidecar_file_names`].
///
/// Error message lists every candidate: a missing sidecar is the single most
/// common packaging mistake and the log needs to show where we looked.
pub fn find_sidecar(stem: &str, dirs: &[PathBuf]) -> Result<PathBuf> {
    let names = sidecar_file_names(stem);
    let mut tried: Vec<PathBuf> = Vec::with_capacity(dirs.len() * names.len());

    for dir in dirs {
        for name in &names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(canonical(&candidate));
            }
            tried.push(candidate);
        }
    }

    Err(anyhow!("{} binary not found; tried: {:?}", stem, tried))
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_triple() {
        let expected = [
            ("macos", "aarch64", "aarch64-apple-darwin"),
            ("macos", "x86_64", "x86_64-apple-darwin"),
            ("windows", "x86_64", "x86_64-pc-windows-msvc"),
            ("windows", "aarch64", "aarch64-pc-windows-msvc"),
            ("linux", "x86_64", "x86_64-unknown-linux-gnu"),
            ("linux", "aarch64", "aarch64-unknown-linux-gnu"),
        ];
        for (os, arch, triple) in expected {
            assert_eq!(triple_for(os, arch), Some(triple), "{}-{}", os, arch);
        }
    }

    #[test]
    fn unknown_platform_has_no_triple() {
        assert_eq!(triple_for("freebsd", "x86_64"), None);
        assert_eq!(triple_for("windows", "riscv64"), None);
    }

    #[test]
    fn exe_suffix_only_on_windows() {
        assert_eq!(exe_suffix_for("windows"), ".exe");
        assert_eq!(exe_suffix_for("macos"), "");
        assert_eq!(exe_suffix_for("linux"), "");
    }

    /// Regression guard for the pre-split bug where the suffix was part of the
    /// triple: the file Tauri produces is `xray-<triple>.exe`, never
    /// `xray-x86_64-pc-windows-msvc.exe` built from a triple that ends in
    /// ".exe".
    #[test]
    fn sidecar_name_shape() {
        let triple = triple_for("windows", "x86_64").expect("triple");
        assert!(!triple.ends_with(".exe"));
        assert_eq!(
            format!("{}-{}{}", "xray", triple, exe_suffix_for("windows")),
            "xray-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn sidecar_names_prefer_bundled_name() {
        let names = sidecar_file_names("tun2socks");
        assert_eq!(names[0], format!("tun2socks{}", exe_suffix()));
        assert!(names.len() >= 2, "repo-style name must also be tried");
    }

    #[test]
    fn missing_sidecar_lists_candidates() {
        let dirs = vec![PathBuf::from("/nonexistent-a"), PathBuf::from("/nonexistent-b")];
        let err = find_sidecar("xray", &dirs).expect_err("must fail");
        let text = err.to_string();
        assert!(text.contains("/nonexistent-a"), "{}", text);
        assert!(text.contains("/nonexistent-b"), "{}", text);
    }

    #[test]
    fn finds_sidecar_in_first_matching_dir() {
        let dir = std::env::temp_dir().join(format!("pvpn-triple-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let name = format!("probe{}", exe_suffix());
        let file = dir.join(&name);
        std::fs::write(&file, b"x").expect("write");

        let found = find_sidecar("probe", &[PathBuf::from("/nonexistent"), dir.clone()])
            .expect("must find");
        assert_eq!(found.file_name().and_then(|n| n.to_str()), Some(name.as_str()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
