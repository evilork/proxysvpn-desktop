// src-tauri/src/tun.rs
// Portable tunnel orchestration. Everything platform-specific lives in the
// pvpn-platform crate (see crates/pvpn-platform/src/net/mod.rs for the
// contract); this file only decides the order of operations, which is the same
// on every OS:
//
//   1. host route to the node through the physical link  (so the engines can
//      still reach it once the default route points inside the tunnel)
//   2. start tun2socks, which creates the TUN device itself
//   3. address the device
//   4. install 0.0.0.0/1 + 128.0.0.0/1 — more specific than the physical
//      default, which therefore stays in the table untouched
//
// Teardown runs the reverse, in the order pinned by net::TEARDOWN_ORDER.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use pvpn_platform::net::{self, TeardownStep};
use pvpn_platform::{privilege, process as pprocess, triple};
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Name of the TUN device on this platform.
pub const TUN_NAME: &str = net::DEVICE;
/// Address assigned to the device; the same on every platform.
pub const TUN_ADDR: Ipv4Addr = net::DEVICE_ADDR;
/// SOCKS inbound that xray exposes and tun2socks dials.
/// Must stay equal to the inbound port in subscription::build_xray_config.
pub const SOCKS_PORT: u16 = 10808;

/// How long the device may take to appear after tun2socks starts.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the supervisor re-checks the routes.
const WATCHDOG_PERIOD: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct TunState {
    child: Option<Child>,
    server_ip: Option<Ipv4Addr>,
    physical_route: Option<net::PhysicalRoute>,
    watchdog: Option<tokio::task::JoinHandle<()>>,
}

pub type SharedTunState = Arc<Mutex<TunState>>;

pub fn new_state() -> SharedTunState {
    Arc::new(Mutex::new(TunState::default()))
}

/// Directories a sidecar may live in, most specific first: next to the
/// executable (how a bundle ships it), then Tauri's resource directory, then
/// the repo layout used by `cargo tauri dev`.
pub fn sidecar_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
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
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        dirs.push(PathBuf::from(manifest_dir).join("binaries"));
    }
    dirs
}

pub fn tun2socks_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    triple::find_sidecar("tun2socks", &sidecar_dirs(app))
}

/// Resolves the node name to a single IPv4 address.
///
/// IPv4 only: the tunnel installs IPv4 half-defaults, so an IPv6 node address
/// would be routed outside it.
async fn resolve_host(host: &str) -> Result<Ipv4Addr> {
    let lookup = format!("{}:443", host);
    let addrs = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        lookup.to_socket_addrs().ok().map(|it| it.collect::<Vec<_>>())
    })
    .await
    .context("dns lookup task panicked")?
    .ok_or_else(|| anyhow!("dns lookup failed for {}", host))?;

    addrs
        .into_iter()
        .find_map(|a| match a.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(_) => None,
        })
        .ok_or_else(|| anyhow!("no ipv4 address for {}", host))
}

fn spawn_tun2socks(bin: &std::path::Path, socks_port: u16) -> Result<Child> {
    let mut cmd = Command::new(bin);
    cmd.args([
        "-device",
        &net::tun2socks_device_arg(),
        "-proxy",
        &format!("socks5://127.0.0.1:{}", socks_port),
        "-loglevel",
        "info",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    pprocess::no_window(&mut cmd);

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
    Ok(child)
}

pub async fn start(
    state: &SharedTunState,
    app: &tauri::AppHandle,
    server_host: &str,
    socks_port: u16,
) -> Result<()> {
    if !privilege::is_elevated() {
        return Err(anyhow!("{}", privilege::missing_privileges_message()));
    }

    let mut guard = state.lock().await;
    if guard.child.is_some() {
        return Err(anyhow!("tun already running"));
    }

    let tun2socks = tun2socks_path(app)?;
    let server_ip = resolve_host(server_host).await?;
    let physical = net::physical_route().await?;

    crate::logger::log("info", "tun", &format!("server {} -> {}", server_host, server_ip));
    crate::logger::log("info", "tun", &format!("original gateway: {}", physical));
    crate::logger::log("info", "tun", &format!("tun2socks: {}", tun2socks.display()));

    net::add_host_route(server_ip, &physical).await?;

    let mut child = spawn_tun2socks(&tun2socks, socks_port)?;

    // The device is created by tun2socks, not by us.
    if let Err(e) = net::wait_for_device(DEVICE_TIMEOUT).await {
        let _ = child.kill().await;
        net::delete_host_route(server_ip).await;
        return Err(e);
    }

    if let Err(e) = net::configure_device().await {
        let _ = child.kill().await;
        net::delete_host_route(server_ip).await;
        return Err(e);
    }

    if let Err(e) = net::add_split_defaults().await {
        net::delete_split_defaults().await;
        let _ = child.kill().await;
        net::delete_host_route(server_ip).await;
        return Err(e);
    }

    crate::logger::log(
        "info",
        "tun",
        &format!("device {} up with {}", TUN_NAME, TUN_ADDR),
    );

    guard.child = Some(child);
    guard.server_ip = Some(server_ip);
    guard.physical_route = Some(physical);

    guard.watchdog = Some(tokio::spawn(watchdog_loop(server_ip)));
    Ok(())
}

/// Re-installs routes that the OS dropped underneath us.
///
/// Waking from sleep, a Wi-Fi change or a DHCP renewal can wipe our entries
/// while both engines stay alive; without this the app looks connected and
/// nothing flows. Runs every 5s and stops as soon as tun2socks is gone.
async fn watchdog_loop(server_ip: Ipv4Addr) {
    loop {
        tokio::time::sleep(WATCHDOG_PERIOD).await;

        if !net::engine_alive("tun2socks").await {
            crate::logger::log("warn", "watchdog", "tun2socks died, stopping watchdog");
            return;
        }

        if !net::host_route_ok(server_ip).await {
            match net::physical_route().await {
                Ok(route) => match net::add_host_route(server_ip, &route).await {
                    Ok(()) => crate::logger::log(
                        "warn",
                        "watchdog",
                        &format!("re-added host route {} via {}", server_ip, route),
                    ),
                    Err(e) => crate::logger::log(
                        "error",
                        "watchdog",
                        &format!("could not re-add host route: {}", e),
                    ),
                },
                Err(e) => crate::logger::log(
                    "error",
                    "watchdog",
                    &format!("no physical route to re-add the host route: {}", e),
                ),
            }
        }

        if !net::split_defaults_ok().await {
            match net::add_split_defaults().await {
                Ok(()) => crate::logger::log("warn", "watchdog", "re-added split-default routes"),
                Err(e) => crate::logger::log(
                    "error",
                    "watchdog",
                    &format!("could not re-add split-default routes: {}", e),
                ),
            }
        }
    }
}

pub async fn stop(state: &SharedTunState) -> Result<()> {
    let mut guard = state.lock().await;
    if let Some(wd) = guard.watchdog.take() {
        wd.abort();
    }
    let server_ip = guard.server_ip;

    // Order is not a detail: see net::TeardownStep for why each step waits for
    // the previous one.
    for step in net::TEARDOWN_ORDER {
        match step {
            TeardownStep::SplitDefaults => net::delete_split_defaults().await,
            TeardownStep::HostRoute => {
                if let Some(ip) = server_ip {
                    net::delete_host_route(ip).await;
                }
            }
            TeardownStep::DeviceDown => net::device_down().await,
            TeardownStep::KillOwnedEngine => {
                if let Some(mut child) = guard.child.take() {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
            TeardownStep::KillStrayEngines => net::kill_stray("tun2socks").await,
        }
    }

    guard.server_ip = None;
    guard.physical_route = None;
    Ok(())
}

/// Lock-free status check: asks the OS, so it stays right even if our state
/// drifted (crashed engine, external kill).
pub async fn is_running(_state: &SharedTunState) -> bool {
    net::engine_alive("tun2socks").await
}

pub async fn get_server_ip(state: &SharedTunState) -> Option<Ipv4Addr> {
    state.lock().await.server_ip
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tun2socks dials the port xray listens on. The two constants live in
    /// different modules, so nothing but a test keeps them together.
    #[test]
    fn socks_port_matches_generated_xray_inbound() {
        let cfg =
            crate::subscription::build_xray_config(&crate::subscription::test_support::sample_vless());
        let port = cfg["inbounds"][0]["port"]
            .as_u64()
            .expect("inbound port in generated config");
        assert_eq!(port, SOCKS_PORT as u64);
    }

    /// The device name in this module and the one the platform layer configures
    /// must be the same string, or we would address an interface that tun2socks
    /// never created.
    #[test]
    fn device_name_comes_from_the_platform_layer() {
        assert_eq!(TUN_NAME, net::DEVICE);
        assert!(net::is_our_device(TUN_NAME));
    }

    #[test]
    fn device_address_is_the_shared_one() {
        assert_eq!(TUN_ADDR, net::DEVICE_ADDR);
        assert_eq!(TUN_ADDR.to_string(), "198.18.0.1");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_device_name_is_unchanged() {
        assert_eq!(TUN_NAME, "utun225");
    }

    #[tokio::test]
    async fn resolve_host_accepts_a_literal_v4() {
        let ip = resolve_host("203.0.113.7").await.expect("literal resolves");
        assert_eq!(ip, Ipv4Addr::new(203, 0, 113, 7));
    }
}
