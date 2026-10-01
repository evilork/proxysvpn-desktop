// src-tauri/src/tun/sys/macos.rs
//! macOS backend: `route`/`ifconfig`/`netstat` plus tun2socks as our own child.
//!
//! Behaviour is unchanged from the single-file `tun.rs` this was split out of:
//! same device name, same commands in the same order, same log level for
//! tun2socks, same crash breadcrumb file and format. The only structural
//! difference is that the tun2socks child now lives here instead of in
//! `TunState`, because on Linux the child belongs to the root helper.

use anyhow::{anyhow, Context, Result};
use std::process::Stdio;
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::contract::TunPlan;
use crate::paths;
use crate::privilege;

pub const TUN_NAME: &str = "utun225";

static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn child_slot() -> &'static Mutex<Option<Child>> {
    CHILD.get_or_init(|| Mutex::new(None))
}

async fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .with_context(|| format!("spawn {} {:?}", program, args))?;
    if !status.success() {
        return Err(anyhow!("{} {:?} failed: {}", program, args, status));
    }
    Ok(())
}

async fn current_default_gateway() -> Result<String> {
    let out = Command::new("route").args(["-n", "get", "default"]).output().await?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut iface: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("gateway:") {
            return Ok(rest.trim().to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("interface:") {
            iface = Some(rest.trim().to_string());
        }
    }
    if let Some(i) = iface {
        if i.starts_with("utun") {
            return Err(anyhow!("default route goes through {} — отключите другой VPN", i));
        }
    }
    Err(anyhow!("could not parse default gateway"))
}

pub async fn preflight() -> Result<()> {
    if !privilege::is_elevated() {
        return Err(anyhow!(
            "приложение не запущено от root — перезапустите через ProxysVPN Launcher"
        ));
    }
    Ok(())
}

pub async fn up(plan: &TunPlan) -> Result<()> {
    let mut slot = child_slot().lock().await;
    if slot.is_some() {
        return Err(anyhow!("tun already running"));
    }

    let server_ip = plan.server_ip.as_str();
    let original_gw = current_default_gateway().await?;
    crate::logger::log("info", "tun", &format!("original gateway: {}", original_gw));

    let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", server_ip]).await;
    run_cmd("/sbin/route", &["-n", "add", "-host", server_ip, &original_gw])
        .await
        .context("add host route for VPN server")?;

    let proxy = format!("socks5://127.0.0.1:{}", plan.socks_port);
    let mut cmd = Command::new(&plan.tun2socks);
    cmd.args(["-device", TUN_NAME, "-proxy", &proxy, "-loglevel", "info"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().context("spawn tun2socks")?;
    pipe_logs(&mut child);

    let mut ready = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(out) = Command::new("/sbin/ifconfig").arg(TUN_NAME).output().await {
            if out.status.success() {
                ready = true;
                break;
            }
        }
    }
    if !ready {
        let _ = child.kill().await;
        let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", server_ip]).await;
        return Err(anyhow!("{} did not come up within 5s", TUN_NAME));
    }

    run_cmd("/sbin/ifconfig", &[TUN_NAME, crate::tun::TUN_ADDR, crate::tun::TUN_ADDR, "up"])
        .await
        .context("assign IP to the tunnel device")?;
    add_split_defaults().await?;

    let hint = crate::tun::render_hint(std::process::id(), server_ip);
    if let Err(e) = std::fs::write(paths::route_hint(), hint) {
        crate::logger::log("warn", "tun", &format!("could not write route hint: {}", e));
    }

    *slot = Some(child);
    Ok(())
}

fn pipe_logs(child: &mut Child) {
    if let Some(out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                crate::logger::log("info", "tun2socks", &line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                crate::logger::log("warn", "tun2socks", &line);
            }
        });
    }
}

async fn add_split_defaults() -> Result<()> {
    run_cmd("/sbin/route", &["-n", "add", "-net", "0.0.0.0/1", "-interface", TUN_NAME])
        .await
        .context("add route 0.0.0.0/1")?;
    run_cmd("/sbin/route", &["-n", "add", "-net", "128.0.0.0/1", "-interface", TUN_NAME])
        .await
        .context("add route 128.0.0.0/1")?;
    Ok(())
}

pub async fn ensure(plan: &TunPlan) -> Result<()> {
    let server_ip = plan.server_ip.as_str();

    if !host_route_ok(server_ip).await {
        if let Ok(gw) = current_default_gateway().await {
            let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", server_ip]).await;
            let _ = run_cmd("/sbin/route", &["-n", "add", "-host", server_ip, &gw]).await;
            crate::logger::log("warn", "watchdog", "re-added the node host route");
        }
    }

    if !split_defaults_ok().await {
        let _ = add_split_defaults().await;
        crate::logger::log("warn", "watchdog", "re-added split-default routes");
    }
    Ok(())
}

/// Is the host route to the node still pointing at the physical link?
async fn host_route_ok(ip: &str) -> bool {
    match Command::new("/sbin/route").args(["-n", "get", "-host", ip]).output().await {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            for line in text.lines() {
                if line.trim().starts_with("interface:") && line.contains(TUN_NAME) {
                    return false; // went into the tunnel = the host route is gone
                }
            }
            text.contains("gateway:") || text.contains("interface:")
        }
        Err(_) => false,
    }
}

async fn split_defaults_ok() -> bool {
    match Command::new("netstat").args(["-rn", "-f", "inet"]).output().await {
        Ok(o) => {
            let t = String::from_utf8_lossy(&o.stdout);
            let low = t.lines().any(|l| l.starts_with("0/1") && l.contains(TUN_NAME));
            let high = t.lines().any(|l| l.starts_with("128.0/1") && l.contains(TUN_NAME));
            low && high
        }
        Err(_) => false,
    }
}

pub async fn down(server_ip: Option<&str>) -> Result<()> {
    let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", "0.0.0.0/1"]).await;
    let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", "128.0.0.0/1"]).await;
    if let Some(ip) = server_ip {
        let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", ip]).await;
    }
    let _ = run_cmd("/sbin/ifconfig", &[TUN_NAME, "down"]).await;

    if let Some(mut child) = child_slot().lock().await.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    let _ = run_cmd("/usr/bin/pkill", &["-x", "tun2socks"]).await;
    let _ = std::fs::remove_file(paths::route_hint());
    Ok(())
}

pub async fn engine_alive() -> bool {
    match Command::new("pgrep").arg("-x").arg("tun2socks").output().await {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Synchronous crash recovery. Killing leftover engines is
/// `privilege::kill_leftover_engines`; here we only undo network state.
pub fn purge_stale() {
    use std::process::Command as SyncCommand;

    let _ = SyncCommand::new("/sbin/route")
        .args(["-n", "delete", "-net", "0.0.0.0/1"])
        .status();
    let _ = SyncCommand::new("/sbin/route")
        .args(["-n", "delete", "-net", "128.0.0.0/1"])
        .status();
    let _ = SyncCommand::new("/sbin/ifconfig").args([TUN_NAME, "down"]).status();

    let hint = paths::route_hint();
    if let Ok(text) = std::fs::read_to_string(&hint) {
        if let Some(ip) = crate::tun::parse_hint_server_ip(&text) {
            let _ = SyncCommand::new("/sbin/route")
                .args(["-n", "delete", "-host", &ip])
                .status();
        }
        let _ = std::fs::remove_file(&hint);
    }
}
