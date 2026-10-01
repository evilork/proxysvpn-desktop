// src-tauri/src/tun/mod.rs
//! Portable half of the tunnel: state, name resolution, the crash breadcrumb
//! and the 5 s supervisor. Everything that touches an interface, a routing
//! table or a resolver lives in `sys::active` (see `sys/contract.rs`).

pub mod sys;

use anyhow::{anyhow, Result};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use tokio::sync::Mutex;

pub use sys::active::TUN_NAME;
pub use sys::contract::TunPlan;

/// Address of our side of the tunnel. 198.18.0.0/15 is the benchmark range
/// (RFC 2544) — it is not routed on the public internet, which is exactly what
/// a local TUN wants.
pub const TUN_ADDR: &str = "198.18.0.1";

/// SOCKS5 port of the xray front end. Must stay in sync with the inbound that
/// `subscription::build_xray_config` emits.
pub const SOCKS_PORT: u16 = 10808;

/// Resolvers published on the tunnel interface when the platform needs them
/// (Linux does; macOS leaves the system resolver alone and lets the queries be
/// proxied as ordinary traffic). Override with `PROXYSVPN_DNS=9.9.9.9,149.112.112.112`.
const DEFAULT_DNS: &[&str] = &["1.1.1.1", "1.0.0.1"];

#[derive(Default)]
pub struct TunState {
    plan: Option<TunPlan>,
    watchdog: Option<tokio::task::JoinHandle<()>>,
}

pub type SharedTunState = Arc<Mutex<TunState>>;

pub fn new_state() -> SharedTunState {
    Arc::new(Mutex::new(TunState::default()))
}

/// Parse a comma separated resolver list, dropping anything unusable.
fn parse_dns_list(raw: &str) -> Vec<IpAddr> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<IpAddr>().ok())
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

/// Crash breadcrumb written while the tunnel is up: if we die without running
/// teardown, the next start removes the host route named here.
pub fn render_hint(pid: u32, server_ip: &str) -> String {
    format!("pid={}\nserver_ip={}\n", pid, server_ip)
}

/// Read the breadcrumb back. The hint file lives in a world-writable directory
/// on macOS, so the value is validated as an IPv4 literal before it can ever
/// reach a `route`/`ip` argument list.
pub fn parse_hint_server_ip(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("server_ip=") {
            let ip: Ipv4Addr = rest.trim().parse().ok()?;
            return Some(ip.to_string());
        }
    }
    None
}

async fn resolve_host(host: &str) -> Result<String> {
    let lookup = format!("{}:443", host);
    let addrs = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        lookup.to_socket_addrs().map(|it| it.collect::<Vec<_>>())
    })
    .await?
    .map_err(|e| anyhow!("dns lookup failed: {}", e))?;
    let addr = addrs
        .into_iter()
        .find(|a| a.is_ipv4())
        .ok_or_else(|| anyhow!("no ipv4 address for the node"))?;
    Ok(addr.ip().to_string())
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

    sys::active::preflight().await?;

    let tun2socks = crate::paths::sidecar_path(app, "tun2socks")?;
    let server_ip = resolve_host(server_host).await?;
    let plan = TunPlan {
        server_ip,
        socks_port,
        tun2socks,
        dns: dns_servers(),
    };

    crate::logger::log("info", "tun", &format!("device {}", TUN_NAME));
    sys::active::up(&plan).await?;

    guard.watchdog = Some(tokio::spawn(watchdog_loop(plan.clone())));
    guard.plan = Some(plan);
    Ok(())
}

/// Every 5 s: is the engine alive, and are our routes still in the table?
/// A network change or a wake from sleep drops them, which used to show up as
/// "the VPN dies after a while".
async fn watchdog_loop(plan: TunPlan) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        if !sys::active::engine_alive().await {
            crate::logger::log("warn", "watchdog", "engine died, stopping watchdog");
            return;
        }
        if let Err(e) = sys::active::ensure(&plan).await {
            crate::logger::log("warn", "watchdog", &format!("route repair failed: {}", e));
        }
    }
}

pub async fn stop(state: &SharedTunState) -> Result<()> {
    let mut guard = state.lock().await;
    if let Some(wd) = guard.watchdog.take() {
        wd.abort();
    }
    let server_ip = guard.plan.as_ref().map(|p| p.server_ip.clone());
    guard.plan = None;
    drop(guard);

    sys::active::down(server_ip.as_deref()).await
}

/// Lock-free: asks the platform backend, not our own state, so a tunnel left
/// over from a previous process is reported too.
pub async fn is_running(_state: &SharedTunState) -> bool {
    sys::active::engine_alive().await
}

/// Synchronous crash recovery: called at startup, from the signal handlers and
/// from `RunEvent::Exit`, where there is no runtime to await on.
pub fn purge_stale() {
    sys::active::purge_stale();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_socks_port_matches_the_xray_inbound() {
        // The config builder and the tunnel must agree, or tun2socks dials a
        // port nobody listens on.
        assert_eq!(SOCKS_PORT, 10808);
    }

    #[test]
    fn tun_addr_is_in_the_benchmark_range() {
        let addr: Ipv4Addr = TUN_ADDR.parse().expect("TUN_ADDR must be an IPv4 literal");
        assert_eq!(addr.octets()[0], 198);
        assert!(matches!(addr.octets()[1], 18 | 19));
    }

    #[test]
    fn tun_name_fits_the_kernel_limit() {
        // IFNAMSIZ is 16 including the NUL on Linux; macOS utun names are
        // shorter still.
        assert!(!TUN_NAME.is_empty());
        assert!(TUN_NAME.len() < 16, "{TUN_NAME} is too long for an interface name");
        assert!(TUN_NAME.chars().all(|c| c.is_ascii_alphanumeric()));
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
    }

    #[test]
    fn hint_round_trips() {
        let text = render_hint(4242, "203.0.113.9");
        assert!(text.contains("pid=4242"));
        assert_eq!(parse_hint_server_ip(&text).as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn hint_rejects_anything_that_is_not_an_ipv4_literal() {
        for bad in [
            "",
            "server_ip=\n",
            "server_ip=; rm -rf /\n",
            "server_ip=-net default\n",
            "server_ip=example.com\n",
            "server_ip=2001:db8::1\n",
            "pid=1\n",
        ] {
            assert_eq!(parse_hint_server_ip(bad), None, "{bad:?} must not be trusted");
        }
    }
}
