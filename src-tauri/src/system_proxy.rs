// src-tauri/src/system_proxy.rs
// Sets/unsets the macOS system SOCKS proxy via `networksetup`.
// macOS only for now. Windows/Linux: TODO.

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

/// Discover active network services (Wi-Fi, Ethernet, etc.).
async fn list_network_services() -> Result<Vec<String>> {
    let out = Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
        .await
        .context("networksetup list failed")?;
    let text = String::from_utf8_lossy(&out.stdout);
    // First line is a header ("An asterisk ..."), skip it. Services prefixed
    // with * are disabled.
    Ok(text
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty() && !l.starts_with('*'))
        .map(|l| l.trim().to_string())
        .collect())
}

/// Enable SOCKS proxy pointing at `host:port` on every active network service.
pub async fn enable_socks(host: &str, port: u16) -> Result<()> {
    let services = list_network_services().await?;
    if services.is_empty() {
        return Err(anyhow!("no active network services found"));
    }
    for svc in &services {
        let _ = Command::new("networksetup")
            .args([
                "-setsocksfirewallproxy",
                svc,
                host,
                &port.to_string(),
            ])
            .status()
            .await;
        let _ = Command::new("networksetup")
            .args(["-setsocksfirewallproxystate", svc, "on"])
            .status()
            .await;
    }
    Ok(())
}

/// Disable SOCKS proxy on every active service.
pub async fn disable_socks() -> Result<()> {
    let services = list_network_services().await?;
    for svc in &services {
        let _ = Command::new("networksetup")
            .args(["-setsocksfirewallproxystate", svc, "off"])
            .status()
            .await;
    }
    Ok(())
}
