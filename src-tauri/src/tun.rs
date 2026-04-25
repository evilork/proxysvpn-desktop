// src-tauri/src/tun.rs
// TUN setup assuming the parent process already has root privileges.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

pub const TUN_NAME: &str = "utun225";
pub const TUN_ADDR: &str = "198.18.0.1";
pub const SOCKS_PORT: u16 = 10808;

#[derive(Default)]
pub struct TunState {
    child: Option<Child>,
    server_ip: Option<String>,
    original_gateway: Option<String>,
}

pub type SharedTunState = Arc<Mutex<TunState>>;

pub fn new_state() -> SharedTunState {
    Arc::new(Mutex::new(TunState::default()))
}

pub fn tun2socks_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    let triple = current_target_triple();
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("tun2socks"));
            candidates.push(dir.join(format!("tun2socks-{}", triple)));
        }
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("tun2socks"));
        candidates.push(resource_dir.join(format!("tun2socks-{}", triple)));
        candidates.push(resource_dir.join("binaries").join("tun2socks"));
        candidates.push(resource_dir.join("binaries").join(format!("tun2socks-{}", triple)));
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        candidates.push(base.join("binaries").join(format!("tun2socks-{}", triple)));
    }

    candidates
        .iter()
        .find(|p| p.exists())
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .ok_or_else(|| anyhow!("tun2socks binary not found; tried: {:?}", candidates))
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

pub fn is_root() -> bool {
    unsafe { libc::getuid() == 0 }
}

async fn resolve_host(host: &str) -> Result<String> {
    let lookup = format!("{}:443", host);
    let addrs: Vec<_> = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        lookup.to_socket_addrs().ok().map(|it| it.collect::<Vec<_>>())
    })
    .await?
    .ok_or_else(|| anyhow!("dns lookup failed for {}", host))?;
    let addr = addrs
        .into_iter()
        .find(|a| a.is_ipv4())
        .ok_or_else(|| anyhow!("no ipv4 address for {}", host))?;
    Ok(addr.ip().to_string())
}

async fn current_default_gateway() -> Result<String> {
    let out = Command::new("route").args(["-n", "get", "default"]).output().await?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("gateway:") {
            return Ok(rest.trim().to_string());
        }
    }
    Err(anyhow!("could not parse default gateway"))
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

pub async fn start(
    state: &SharedTunState,
    app: &tauri::AppHandle,
    server_host: &str,
) -> Result<()> {
    if !is_root() {
        return Err(anyhow!(
            "приложение не запущено от root — перезапустите через ProxysVPN Launcher"
        ));
    }

    let mut guard = state.lock().await;
    if guard.child.is_some() {
        return Err(anyhow!("tun already running"));
    }

    let tun2socks = tun2socks_path(app)?;
    let server_ip = resolve_host(server_host).await?;
    let original_gw = current_default_gateway().await?;

    crate::logger::log("info", "tun", &format!("server {} -> {}", server_host, server_ip));
    crate::logger::log("info", "tun", &format!("original gateway: {}", original_gw));
    println!("[tun] tun2socks: {}", tun2socks.display());

    let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", &server_ip]).await;
    run_cmd(
        "/sbin/route",
        &["-n", "add", "-host", &server_ip, &original_gw],
    )
    .await
    .context("add host route for VPN server")?;

    let mut cmd = Command::new(&tun2socks);
    cmd.args([
        "-device", TUN_NAME,
        "-proxy", &format!("socks5://127.0.0.1:{}", SOCKS_PORT),
        "-loglevel", "info",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    let mut child = cmd.spawn().context("spawn tun2socks")?;

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

    let mut ready = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let out = Command::new("/sbin/ifconfig").arg(TUN_NAME).output().await.ok();
        if let Some(o) = out {
            if o.status.success() {
                ready = true;
                break;
            }
        }
    }
    if !ready {
        let _ = child.kill().await;
        let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", &server_ip]).await;
        return Err(anyhow!("utun225 did not come up within 5s"));
    }

    run_cmd(
        "/sbin/ifconfig",
        &[TUN_NAME, TUN_ADDR, TUN_ADDR, "up"],
    )
    .await
    .context("assign IP to utun225")?;

    run_cmd(
        "/sbin/route",
        &["-n", "add", "-net", "0.0.0.0/1", "-interface", TUN_NAME],
    )
    .await
    .context("add route 0.0.0.0/1")?;
    run_cmd(
        "/sbin/route",
        &["-n", "add", "-net", "128.0.0.0/1", "-interface", TUN_NAME],
    )
    .await
    .context("add route 128.0.0.0/1")?;

    guard.child = Some(child);
    guard.server_ip = Some(server_ip);
    guard.original_gateway = Some(original_gw);
    Ok(())
}

pub async fn stop(state: &SharedTunState) -> Result<()> {
    let mut guard = state.lock().await;
    let server_ip = guard.server_ip.clone();

    let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", "0.0.0.0/1"]).await;
    let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", "128.0.0.0/1"]).await;

    if let Some(ref ip) = server_ip {
        let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", ip]).await;
    }

    let _ = run_cmd("/sbin/ifconfig", &[TUN_NAME, "down"]).await;

    if let Some(mut child) = guard.child.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    let _ = run_cmd("/usr/bin/pkill", &["-x", "tun2socks"]).await;

    guard.server_ip = None;
    guard.original_gateway = None;
    Ok(())
}

/// Lock-free status check — pgrep doesn't need our state.
pub async fn is_running(_state: &SharedTunState) -> bool {
    if let Ok(out) = Command::new("pgrep").arg("-x").arg("tun2socks").output().await {
        !out.stdout.is_empty()
    } else {
        false
    }
}

pub async fn get_server_ip(state: &SharedTunState) -> Option<String> {
    state.lock().await.server_ip.clone()
}
