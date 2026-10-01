// src-tauri/src/tun.rs
// Portable tunnel orchestration.
//
// Everything platform-specific lives in the pvpn-platform crate — see
// crates/pvpn-platform/src/net/mod.rs for the contract and for why it is shaped
// the way it is. This file holds only what is the same on every OS: resolving
// the node name, assembling the plan, and the 5 s supervisor.
//
// The order of operations used to live here. It moved into the platform layer
// when Linux arrived, because there the whole sequence runs inside a root helper
// and has to be atomic on the far side of an IPC boundary. macOS and Windows
// share one copy of that sequence in `net::local`, transcribed step for step
// from what this file used to do.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use pvpn_platform::net::{self, TunPlan};
use pvpn_platform::triple;
use tauri::Manager;
use tokio::sync::Mutex;

/// Name of the TUN device on this platform.
pub const TUN_NAME: &str = net::DEVICE;
/// Address assigned to the device; the same on every platform.
///
/// Re-exported rather than re-declared: the platform layer owns the value and
/// the test below pins the two together, so the device cannot be addressed here
/// with one number and configured there with another.
#[allow(dead_code)]
pub const TUN_ADDR: Ipv4Addr = net::DEVICE_ADDR;
/// SOCKS inbound that xray exposes and tun2socks dials.
/// Must stay equal to the inbound port in subscription::build_xray_config.
pub const SOCKS_PORT: u16 = 10808;

/// How often the supervisor re-checks the routes.
const WATCHDOG_PERIOD: Duration = Duration::from_secs(5);

/// Resolvers published on the tunnel interface, on the platforms that need them
/// (Linux does; macOS and Windows let the system resolver's queries be proxied
/// as ordinary traffic — see `configure_dns` in each platform module).
/// Override with `PROXYSVPN_DNS=9.9.9.9,149.112.112.112`.
const DEFAULT_DNS: &[&str] = &["1.1.1.1", "1.0.0.1"];

#[derive(Default)]
pub struct TunState {
    /// The plan of the tunnel that is up, if any. Also what the supervisor
    /// re-asserts, so it must stay exactly what `up` was given.
    plan: Option<TunPlan>,
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

/// Parse a comma separated resolver list, dropping anything unusable.
fn parse_dns_list(raw: &str) -> Vec<IpAddr> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<IpAddr>().ok())
        // A loopback or unspecified resolver cannot be reached through a
        // tunnel, and on Linux it would be published on the device and break
        // every lookup on the machine.
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect()
}

fn dns_servers() -> Vec<IpAddr> {
    if let Ok(raw) = std::env::var("PROXYSVPN_DNS") {
        let parsed = parse_dns_list(&raw);
        if !parsed.is_empty() {
            return parsed;
        }
        crate::logger::log(
            "warn",
            "tun",
            "PROXYSVPN_DNS is set but holds no usable address — falling back to the default",
        );
    }
    parse_dns_list(&DEFAULT_DNS.join(","))
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
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .ok_or_else(|| anyhow!("no ipv4 address for {}", host))
}

pub async fn start(
    state: &SharedTunState,
    app: &tauri::AppHandle,
    server_host: &str,
    socks_port: u16,
) -> Result<()> {
    let mut guard = state.lock().await;
    if guard.plan.is_some() {
        return Err(anyhow!("tun already running"));
    }

    // Before anything is changed: privileges on macOS and Windows, the polkit
    // dialog on Linux. A refusal here leaves the machine untouched.
    net::preflight().await?;

    let tun2socks = tun2socks_path(app)?;
    let server_ip = resolve_host(server_host).await?;
    let plan = TunPlan {
        server_ip,
        socks_port,
        tun2socks,
        dns: dns_servers(),
    };

    crate::logger::log("info", "tun", &format!("device {}", TUN_NAME));
    net::up(&plan).await?;

    guard.watchdog = Some(tokio::spawn(watchdog_loop(plan.clone())));
    guard.plan = Some(plan);
    Ok(())
}

/// Every 5 s: is the engine alive, and are our routes still in place?
///
/// Waking from sleep, a Wi-Fi change or a DHCP renewal can wipe our entries
/// while the engine stays alive; without this the app looks connected and
/// nothing flows.
async fn watchdog_loop(plan: TunPlan) {
    loop {
        tokio::time::sleep(WATCHDOG_PERIOD).await;

        if !net::engine_alive().await {
            crate::logger::log("warn", "watchdog", "engine died, stopping watchdog");
            return;
        }
        if let Err(e) = net::ensure(&plan).await {
            crate::logger::log("warn", "watchdog", &format!("route repair failed: {}", e));
        }
    }
}

pub async fn stop(state: &SharedTunState) -> Result<()> {
    let mut guard = state.lock().await;
    if let Some(wd) = guard.watchdog.take() {
        wd.abort();
    }
    let server_ip = guard.plan.as_ref().map(|p| p.server_ip);
    guard.plan = None;

    // The guard is deliberately held across `down`. An earlier version dropped
    // it here "so the status poll does not block behind teardown", which was
    // wrong on both counts: `is_running` below never takes this lock, so the
    // early drop bought nothing, and it let a concurrent `start` pass the
    // `plan.is_some()` check and run `up` *while* this `down` was still
    // deleting routes and killing the engine. The tray's "Отключить VPN" spawns
    // this function in its own task (see lib.rs), so a user who clicks it and
    // then immediately reconnects hit exactly that interleaving: either the
    // split defaults were removed from under a live engine — traffic leaving in
    // the clear while the UI said "connected" — or the fresh engine was killed
    // by the old teardown's stray sweep, leaving routes pointing at a dead
    // device. Before the platform split, `stop` held its lock the whole way
    // through for the same reason.
    net::down(server_ip).await
}

/// Lock-free status check: asks the platform, so it stays right even if our own
/// state drifted (a crashed engine, an external kill).
pub async fn is_running(_state: &SharedTunState) -> bool {
    net::engine_alive().await
}

pub async fn get_server_ip(state: &SharedTunState) -> Option<Ipv4Addr> {
    state.lock().await.plan.as_ref().map(|p| p.server_ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tun2socks dials the port xray listens on. The two constants live in
    /// different modules, so nothing but a test keeps them together.
    #[test]
    fn socks_port_matches_generated_xray_inbound() {
        let cfg = crate::subscription::build_xray_config(
            &crate::subscription::test_support::sample_vless(),
        );
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

    #[test]
    fn default_dns_list_is_usable() {
        let got = dns_servers();
        assert!(!got.is_empty());
        assert!(got.iter().all(|ip| !ip.is_loopback()));
    }

    #[test]
    fn dns_list_parsing_drops_junk() {
        assert_eq!(
            parse_dns_list(" 9.9.9.9 , nonsense ,,127.0.0.1, 2620:fe::fe "),
            vec![
                "9.9.9.9".parse::<IpAddr>().expect("ip"),
                "2620:fe::fe".parse::<IpAddr>().expect("ip"),
            ]
        );
        assert!(parse_dns_list("").is_empty());
        assert!(parse_dns_list("0.0.0.0").is_empty());
        assert!(parse_dns_list("127.0.0.53").is_empty());
    }
}
