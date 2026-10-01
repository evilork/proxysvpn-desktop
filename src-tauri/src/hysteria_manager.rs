// src-tauri/src/hysteria_manager.rs
// Manages the hysteria2 client process: writes a YAML config and runs
// hysteria in client mode, exposing a SOCKS5 inbound that tun2socks consumes.

use anyhow::{anyhow, Context, Result};
use pvpn_platform::{paths, process as pprocess, triple};
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
    triple::find_sidecar("hysteria", &crate::tun::sidecar_dirs(app))
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

    // Fixed location per platform (TMPDIR differs under sudo, and Windows has
    // no /tmp at all) — see pvpn_platform::paths.
    let cfg_path = paths::hy2_config_file()?;
    // Owner-only from the moment it exists, and never through a symlink: it
    // holds the node password, and on macOS this path is in /tmp and written by
    // root. See paths::write_private_file.
    paths::write_private_file(&cfg_path, yaml.as_bytes()).context("write hysteria config")?;

    crate::logger::log("info", "hysteria", &format!("config written: {}", cfg_path.display()));
    // The node address is deliberately absent: it is not public information and
    // this log is attached to support tickets. The remark identifies the node.
    crate::logger::log("info", "hysteria", &format!("node: {}", cfg.remark));
    crate::logger::log("info", "hysteria", &format!("binary: {}", bin.display()));

    let cfg_arg = cfg_path
        .to_str()
        .ok_or_else(|| anyhow!("hysteria config path is not valid UTF-8: {}", cfg_path.display()))?;
    let mut cmd = Command::new(&bin);
    cmd.args(["client", "-c", cfg_arg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Without this a console window pops up on Windows for every engine start.
    pprocess::no_window(&mut cmd);

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
    // A sidecar that outlived its handle still holds the SOCKS port.
    pvpn_platform::process::kill_by_name("hysteria").await;
    // The config holds the node password and hysteria is no longer reading it.
    // On Linux and Windows it lives in the user's profile, where it would
    // otherwise sit until the next connect overwrote it.
    if let Ok(cfg_path) = paths::hy2_config_file() {
        if let Err(e) = std::fs::remove_file(&cfg_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                crate::logger::log(
                    "warn",
                    "hysteria",
                    &format!("could not remove {}: {}", cfg_path.display(), e),
                );
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub async fn is_running(state: &SharedHysteriaState) -> bool {
    state.lock().await.child.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::test_support::sample_hy2;

    /// The YAML is hand-built, so its exact shape is worth pinning: hysteria
    /// silently ignores keys it does not understand, which turns a typo into a
    /// connection that "works" without TLS pinning.
    #[test]
    fn config_yaml_is_stable() {
        let yaml = build_config(&sample_hy2());
        assert_eq!(
            yaml,
            concat!(
                "server: node.example.invalid:8443\n",
                "auth: example-password-not-a-real-one\n",
                "tls:\n",
                "  sni: cover.example.invalid\n",
                "  insecure: true\n",
                "  pinSHA256: AA:BB:CC\n",
                "socks5:\n",
                "  listen: 127.0.0.1:10809\n",
                "fastOpen: true\n",
            )
        );
    }

    #[test]
    fn insecure_and_pin_are_omitted_when_unset() {
        let mut cfg = sample_hy2();
        cfg.insecure = false;
        cfg.pin_sha256 = String::new();
        let yaml = build_config(&cfg);
        assert!(!yaml.contains("insecure"), "{}", yaml);
        assert!(!yaml.contains("pinSHA256"), "{}", yaml);
        assert!(yaml.contains("  sni: cover.example.invalid\n"), "{}", yaml);
    }

    #[test]
    fn hysteria_listens_on_its_own_port() {
        // Sharing xray's inbound would make the two engines fight for 10808.
        assert_ne!(HY2_SOCKS_PORT, crate::tun::SOCKS_PORT);
        let yaml = build_config(&sample_hy2());
        assert!(yaml.contains(&format!("127.0.0.1:{}", HY2_SOCKS_PORT)), "{}", yaml);
    }

    /// The config carries the node password, so it must never land somewhere a
    /// different local user could read or replace it.
    ///
    /// The path differs per platform, and deliberately so: macOS keeps the
    /// pre-split literal in /tmp (a fixed path, because TMPDIR changes under
    /// sudo) and relies on mode 0600, while Windows and Linux put it in a
    /// per-user directory, which is strictly better. So this asserts the
    /// invariant that actually matters rather than one hard-coded string.
    #[test]
    fn config_path_is_app_private() {
        let path = pvpn_platform::paths::hy2_config_file().expect("config path");
        let text = path.to_string_lossy().to_ascii_lowercase();

        #[cfg(target_os = "macos")]
        assert_eq!(path, std::path::PathBuf::from("/tmp/proxysvpn-hy2.yaml"));

        #[cfg(target_os = "windows")]
        {
            assert!(text.contains("proxysvpn"), "{}", text);
            assert!(!text.starts_with(r"c:\windows\temp"), "{}", text);
        }

        // Linux must not fall back to a world-writable directory: unlike macOS
        // there is no reason to, and a predictable name in /tmp invites a
        // symlink swap by another local user.
        #[cfg(target_os = "linux")]
        {
            assert!(text.contains("proxysvpn"), "{}", text);
            assert!(!text.starts_with("/tmp/"), "{}", text);
        }

        assert!(path.is_absolute(), "{}", text);
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("yaml"));
    }
}
