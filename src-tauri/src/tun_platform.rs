// src-tauri/src/tun_platform.rs
//
// The tunnel on Windows and Linux: the same API as tun.rs, which is the macOS
// one, built on the platform layer's contract (pvpn-platform, net/mod.rs)
// instead of on `route` and `ifconfig`. lib.rs imports this module under the
// name `tun`, so the core cannot tell which of the two it is driving.
//
// ── What the platform layer does, and what stays here ──────────────────────
// The order of operations — host route to the node, tun2socks, device
// address, both halves of the default route, DNS, and the rollback of each step
// when a later one fails — lives in pvpn-platform. It has to: on Linux the
// whole sequence runs inside a root helper on the far side of a pipe, and on
// Windows it is shared with the reference implementation the platform layer
// tests with a fake backend. What remains here is what the 0.3.1 core asks of
// a tunnel and the platform layer does not decide: which node, which DNS, what
// the person is told when it fails, and whether the network changed under us.
//
// ── Where this differs from the macOS tunnel ──────────────────────────────
//   * A location change moves only the host route (`net::retarget`), as on
//     macOS: the device, the split defaults, DNS and tun2socks stay up.
//   * A dead tun2socks is brought back by lowering and raising the tunnel
//     (`restart_engine`). On Windows the Wintun adapter dies with tun2socks
//     and on Linux the helper owns the device, so there is nothing to keep.
//   * Route repair reports only "the network changed". Which route the
//     platform put back is in the log, written by the platform layer itself.
//   * DNS: Linux publishes `subscription::TUNNEL_DNS` on the tunnel device
//     (systemd-resolved, or /etc/resolv.conf), the same in-tunnel resolver
//     macOS hands to the system (sysdns.rs); xray answers it through dns-out.
//     Windows leaves the system resolver alone (see net/windows.rs).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;

use pvpn_platform::net::{self, TunPlan};
use pvpn_platform::paths;
use tokio::sync::Mutex;

use crate::errors::{AppError, ErrorCode};

/// The only SOCKS port tun2socks ever speaks to: xray's front inbound, for
/// both protocols. `xray_manager::FRONT_SOCKS_PORT` is the same number and a
/// test there keeps the two from drifting; the Linux helper accepts only the
/// ports in its allow-list, which a test below checks.
pub const SOCKS_PORT: u16 = 10808;

#[derive(Default)]
pub struct TunState {
    /// The plan of the tunnel that is up, if any. Also what the supervisor
    /// re-asserts, so it must stay exactly what the platform was given. The
    /// node address in it is never logged.
    plan: Option<TunPlan>,
    /// How the machine reached the internet the last time we looked. A change
    /// means the network changed under us.
    physical: Option<PhysicalRoute>,
}

pub type SharedTunState = Arc<Mutex<TunState>>;

pub fn new_state() -> SharedTunState {
    Arc::new(Mutex::new(TunState::default()))
}

/// The real way out of this machine: gateway and interface. The same shape as
/// on macOS, because the core binds xray's sockets to `interface`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalRoute {
    /// `None` on a link-level default (a PPP or LTE link has no gateway).
    pub gateway: Option<String>,
    /// The name xray's `sockopt.interface` matches: the adapter alias on
    /// Windows ("Ethernet", "Wi-Fi"), the device on Linux ("enp0s3").
    pub interface: String,
}

/// What one pass of route maintenance found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouteRepair {
    /// The machine changed how it reaches the internet — Wi-Fi to cable, a
    /// wake from sleep on a different network. The engine config names the old
    /// interface and has to be rebuilt.
    pub network_changed: bool,
}

impl RouteRepair {
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Facts
// ───────────────────────────────────────────────────────────────────────────

pub fn tun2socks_path(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    crate::sidecars::find(app, "tun2socks", "tun")
}

/// The current physical default route.
///
/// NETWORK_OFFLINE when there is none: a machine with no default route is not a
/// machine whose VPN failed. TUN_FAILED when there is one but its interface has
/// no name we can hand to xray: binding to "" would silently mean "do not bind"
/// and bring back the routing loop xray_manager.rs describes.
pub async fn physical_default() -> Result<PhysicalRoute, AppError> {
    let route = net::physical_route().await.map_err(|e| {
        crate::logger::log("error", "tun", &format!("no physical default route: {e:#}"));
        AppError::new(ErrorCode::NetworkOffline)
    })?;
    let interface = route
        .if_name
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            crate::logger::log("error", "tun", "physical default route has no interface name");
            AppError::new(ErrorCode::TunFailed)
        })?;
    Ok(PhysicalRoute {
        gateway: route.next_hop,
        interface,
    })
}

/// Resolve the node name to one IPv4 address.
///
/// IPv4 only: the tunnel installs IPv4 half-defaults, so an IPv6 node address
/// would be routed outside it. The name is NOT logged: it is a node address,
/// and the log is a file support asks people to paste into a chat. The
/// address it resolves to is taught to the log's redactor before anything
/// can write it: the platform layer names it in route commands and their
/// failures, in forms the structural rules may not recognise.
async fn resolve_host(host: &str) -> Result<Ipv4Addr, AppError> {
    let ip = resolve_v4(host).await?;
    crate::logger::remember_node_host(&ip.to_string());
    Ok(ip)
}

async fn resolve_v4(host: &str) -> Result<Ipv4Addr, AppError> {
    let lookup = format!("{host}:443");
    let resolved = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        lookup
            .to_socket_addrs()
            .ok()
            .map(|it| it.collect::<Vec<_>>())
    })
    .await
    .ok()
    .flatten();

    resolved
        .into_iter()
        .flatten()
        .find_map(|a| match a.ip() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .ok_or_else(|| {
            crate::logger::log("error", "tun", "node address did not resolve");
            AppError::new(ErrorCode::NoRoute)
        })
}

/// Parse a comma separated resolver list, dropping anything unusable.
fn parse_dns_list(raw: &str) -> Vec<IpAddr> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<IpAddr>().ok())
        // A loopback or unspecified resolver cannot be reached through the
        // tunnel, and on Linux it would be published on the device and break
        // every lookup on the machine (the helper refuses it anyway).
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect()
}

/// Resolvers for the tunnel device: the in-tunnel address xray answers, unless
/// `PROXYSVPN_DNS=9.9.9.9,149.112.112.112` says otherwise.
fn dns_servers() -> Vec<IpAddr> {
    if let Ok(raw) = std::env::var("PROXYSVPN_DNS") {
        let parsed = parse_dns_list(&raw);
        if !parsed.is_empty() {
            return parsed;
        }
        crate::logger::log(
            "warn",
            "tun",
            "PROXYSVPN_DNS is set but holds no usable address — using the tunnel resolver",
        );
    }
    parse_dns_list(crate::subscription::TUNNEL_DNS)
}

/// Which code a failed raise is. The platform layer reports in text; the
/// window needs one of the codes it has a phrase and a button for.
fn raise_error(err: &anyhow::Error) -> ErrorCode {
    let text = format!("{err:#}").to_lowercase();
    if text.contains("tun2socks") {
        // "spawn tun2socks", "tun2socks exited early": the engine, not routes.
        ErrorCode::EngineStartFailed
    } else if text.contains("шлюз") || text.contains("маршрута по умолчанию") {
        // The platform layer's own words for "no default route".
        ErrorCode::NetworkOffline
    } else {
        ErrorCode::TunFailed
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Up, moved, repaired, down
// ───────────────────────────────────────────────────────────────────────────

/// What `start` makes of the tunnel it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Raise {
    /// Nothing is up: raise it.
    Fresh,
    /// A plan is still recorded but its tun2socks is gone. Nothing clears the
    /// plan when the engine dies (`is_running` only asks the platform, and
    /// `restart_engine` needs the plan), so the repair ladder's raise (C1,
    /// and C2–C4 when the tunnel is down) used to be refused here every time
    /// and the ladder ended Failed until a manual Retry. The remains are torn
    /// down and the tunnel raised again, as on macOS.
    OverDeadEngine,
    /// A live tunnel: a second raise would fight it for the device.
    AlreadyUp,
}

fn raise_kind(plan_recorded: bool, engine_alive: bool) -> Raise {
    match (plan_recorded, engine_alive) {
        (false, _) => Raise::Fresh,
        (true, false) => Raise::OverDeadEngine,
        (true, true) => Raise::AlreadyUp,
    }
}

pub async fn start(
    state: &SharedTunState,
    app: &tauri::AppHandle,
    server_host: &str,
) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    let recorded = guard.plan.as_ref().map(|plan| plan.server_ip);
    let alive = match recorded {
        Some(_) => net::engine_alive().await,
        None => false,
    };
    match raise_kind(recorded.is_some(), alive) {
        Raise::Fresh => {}
        Raise::AlreadyUp => {
            crate::logger::log("error", "tun", "start called while tun is up");
            return Err(AppError::new(ErrorCode::TunFailed));
        }
        Raise::OverDeadEngine => {
            crate::logger::log(
                "warn",
                "tun",
                "the tunnel's engine is gone; clearing what is left of it before raising again",
            );
            // The guard stays held across the teardown, as in `stop`.
            if let Err(e) = net::down(recorded).await {
                crate::logger::log("warn", "tun", &format!("teardown of the dead tunnel: {e:#}"));
            }
            guard.plan = None;
            guard.physical = None;
            forget_route_hint();
        }
    }

    // Before anything is changed: elevation on Windows, the polkit dialog on
    // Linux (and from an AppImage the one-time copy of its helper). A refusal
    // here leaves the machine untouched, and it is the person's own answer,
    // so it gets the code with that phrase — unless the machine cannot
    // elevate at all (no pkexec, no polkit agent), SteamOS needs something
    // from them (a password, Desktop Mode), or the copy failed: each of those
    // gets its own code (errors::preflight_code).
    net::preflight().await.map_err(|e| {
        crate::logger::log("error", "tun", &format!("no privileges: {e:#}"));
        AppError::new(crate::errors::preflight_code(&e))
    })?;

    let tun2socks = tun2socks_path(app)?;
    let server_ip = resolve_host(server_host).await?;
    let physical = physical_default().await?;
    crate::logger::log(
        "info",
        "tun",
        &format!("physical route via {}", physical.interface),
    );

    let plan = TunPlan {
        server_ip,
        socks_port: SOCKS_PORT,
        tun2socks,
        dns: dns_servers(),
    };
    net::up(&plan).await.map_err(|e| {
        crate::logger::log("error", "tun", &format!("tunnel did not come up: {e:#}"));
        AppError::new(raise_error(&e))
    })?;

    guard.plan = Some(plan);
    guard.physical = Some(physical);
    Ok(())
}

/// Put back whatever the system took away, and say whether the network
/// changed. Never restarts tun2socks: a dead engine is the supervisor's
/// business, and re-adding routes to a device that is gone would only fail.
pub async fn ensure_routes(state: &SharedTunState) -> RouteRepair {
    let mut repair = RouteRepair::default();
    let (plan, known) = {
        let guard = state.lock().await;
        (guard.plan.clone(), guard.physical.clone())
    };
    let Some(plan) = plan else {
        return repair;
    };

    let physical = match physical_default().await {
        Ok(p) => p,
        // No default route at all: the machine is offline. Nothing to repair,
        // and the probe layer is the one that says so out loud.
        Err(_) => return repair,
    };
    if known.as_ref() != Some(&physical) {
        repair.network_changed = true;
        state.lock().await.physical = Some(physical);
    }

    if let Err(e) = net::ensure(&plan).await {
        crate::logger::log("warn", "tun", &format!("route repair failed: {e:#}"));
    }
    repair
}

/// Move the live tunnel to a different node without lowering it: the new host
/// route goes in before the old one goes out (`net::retarget`).
///
/// The state lock is held across the move, so a concurrent `stop` cannot
/// interleave with it and remove routes from under a half-moved tunnel.
pub async fn retarget(state: &SharedTunState, server_host: &str) -> Result<(), AppError> {
    let new_ip = resolve_host(server_host).await?;
    let physical = physical_default().await?;

    let mut guard = state.lock().await;
    let Some(current) = guard.plan.clone() else {
        crate::logger::log("error", "tun", "retarget called while tun is down");
        return Err(AppError::new(ErrorCode::TunFailed));
    };
    if current.server_ip == new_ip {
        // Same address behind a different label, or the same node again.
        return Ok(());
    }

    let next = TunPlan {
        server_ip: new_ip,
        ..current.clone()
    };
    net::retarget(Some(current.server_ip), &next)
        .await
        .map_err(|e| {
            crate::logger::log("error", "tun", &format!("host route not moved: {e:#}"));
            AppError::new(ErrorCode::TunFailed)
        })?;
    guard.plan = Some(next);
    guard.physical = Some(physical);
    drop(guard);

    // The crash breadcrumb must name the route that now exists.
    persist_route_hint(state).await;
    Ok(())
}

/// Bring tun2socks back after it died.
///
/// Lower and raise rather than respawn in place: on Windows the Wintun adapter
/// and every route on it died with the process, and on Linux the helper owns
/// the device and raises it as one step. The host route to the node is put
/// back by the same raise.
pub async fn restart_engine(state: &SharedTunState, app: &tauri::AppHandle) -> Result<(), AppError> {
    let tun2socks = tun2socks_path(app)?;

    let mut guard = state.lock().await;
    let Some(current) = guard.plan.clone() else {
        crate::logger::log("error", "tun", "restart called while tun is down");
        return Err(AppError::new(ErrorCode::EngineDied));
    };
    let plan = TunPlan {
        tun2socks,
        ..current
    };

    if let Err(e) = net::down(Some(plan.server_ip)).await {
        crate::logger::log("warn", "tun", &format!("teardown before restart: {e:#}"));
    }
    if let Err(e) = net::up(&plan).await {
        // Nothing is up any more; say so in the state rather than pretending.
        guard.plan = None;
        guard.physical = None;
        forget_route_hint();
        crate::logger::log("error", "tun", &format!("tunnel did not come back: {e:#}"));
        return Err(AppError::new(raise_error(&e)));
    }
    guard.plan = Some(plan);
    Ok(())
}

pub async fn stop(state: &SharedTunState) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    let server_ip = guard.plan.take().map(|p| p.server_ip);
    guard.physical = None;

    // This runs before every connect, not only on a real disconnect. With
    // nothing raised and no engine of ours alive there is nothing to tear down,
    // and on Windows a teardown is half a dozen netsh runs for nothing.
    if server_ip.is_none() && !net::engine_alive().await {
        return Ok(());
    }

    // The guard is held across `down`: a `start` that got in between would
    // pass the "not up" check and raise a tunnel while this teardown deletes
    // its routes and kills its engine (the race PR #3 fixed in main).
    let result = net::down(server_ip).await;
    forget_route_hint();
    result.map_err(|e| {
        crate::logger::log("error", "tun", &format!("teardown failed: {e:#}"));
        AppError::new(ErrorCode::TunFailed)
    })
}

/// Is the tunnel's engine alive? Asked of the platform layer, which holds the
/// handle on Windows and asks the helper on Linux, so a tun2socks belonging to
/// another application never counts.
pub async fn is_running(_state: &SharedTunState) -> bool {
    net::engine_alive().await
}

// ───────────────────────────────────────────────────────────────────────────
// Surviving our own death
// ───────────────────────────────────────────────────────────────────────────

/// Write down the node whose host route we installed, so that a run which
/// follows a crash can remove a route it did not install itself. Owner-only,
/// never through a symlink (`write_private_file`). On Linux the root helper
/// keeps its own hint and cleans up from it; this one is for Windows.
pub async fn persist_route_hint(state: &SharedTunState) {
    let Some(ip) = state.lock().await.plan.as_ref().map(|p| p.server_ip) else {
        return;
    };
    let path = match paths::route_hint_file() {
        Ok(path) => path,
        Err(e) => {
            crate::logger::log("warn", "tun", &format!("no route hint location: {e:#}"));
            return;
        }
    };
    let contents = net::format_route_hint(std::process::id(), ip);
    if let Err(e) = paths::write_private_file(&path, contents.as_bytes()) {
        crate::logger::log("warn", "tun", &format!("route hint not written: {e:#}"));
    }
}

pub fn forget_route_hint() {
    if let Ok(path) = paths::route_hint_file() {
        let _ = std::fs::remove_file(path);
    }
}

/// Remove what a previous run left behind. Synchronous on purpose: it runs at
/// startup and on signals, where there is no runtime to await on.
pub fn purge_stale_routes() {
    let stale: Vec<Ipv4Addr> = paths::route_hint_file()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| net::parse_route_hint(&text))
        .unwrap_or_default();
    net::purge_stale(&stale);
    forget_route_hint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_and_address_come_from_the_platform_layer() {
        // probe.rs samples the counters of exactly this device.
        assert_eq!(crate::probe::TUN_IFACE, net::DEVICE);
        assert!(net::is_our_device(net::DEVICE));
        assert_eq!(net::DEVICE_ADDR.to_string(), "198.18.0.1");
    }

    /// The Linux helper refuses any SOCKS port that is not one of this app's
    /// engine ports: the port decides where every packet on the machine goes.
    #[test]
    fn the_socks_port_is_one_the_helper_accepts() {
        use pvpn_platform::helper::proto::ALLOWED_SOCKS_PORTS;
        assert!(ALLOWED_SOCKS_PORTS.contains(&SOCKS_PORT));
        assert!(ALLOWED_SOCKS_PORTS.contains(&crate::hysteria_manager::HY2_SOCKS_PORT));
        assert_eq!(SOCKS_PORT, crate::xray_manager::FRONT_SOCKS_PORT);
    }

    /// The default resolver is the in-tunnel address xray answers, and it
    /// survives the same filter the helper applies.
    #[test]
    fn the_default_resolver_is_the_tunnel_one() {
        let expected: IpAddr = crate::subscription::TUNNEL_DNS.parse().expect("ip");
        assert_eq!(parse_dns_list(crate::subscription::TUNNEL_DNS), vec![expected]);
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

    #[tokio::test]
    async fn a_literal_v4_resolves_to_itself() {
        let ip = resolve_host("203.0.113.7").await.expect("literal resolves");
        assert_eq!(ip, Ipv4Addr::new(203, 0, 113, 7));
    }

    #[test]
    fn a_failed_raise_gets_the_code_with_the_right_phrase() {
        let engine = anyhow::anyhow!("start tun2socks failed, rolling back: spawn tun2socks");
        assert_eq!(raise_error(&engine), ErrorCode::EngineStartFailed);
        let offline = anyhow::anyhow!("нет сетевого подключения — не найден шлюз по умолчанию");
        assert_eq!(raise_error(&offline), ErrorCode::NetworkOffline);
        let routes = anyhow::anyhow!("route 0.0.0.0/1 into proxysvpn0");
        assert_eq!(raise_error(&routes), ErrorCode::TunFailed);
    }

    /// Windows and Linux: tun2socks died while a plan was recorded. A raise
    /// must clear the remains and go ahead, not refuse as if the tunnel were
    /// up — that refusal failed every rung of the repair ladder.
    #[test]
    fn a_raise_after_the_engine_died_goes_ahead() {
        assert_eq!(raise_kind(false, false), Raise::Fresh);
        assert_eq!(raise_kind(false, true), Raise::Fresh, "an engine of nobody's plan is not ours to fight");
        assert_eq!(raise_kind(true, false), Raise::OverDeadEngine);
        assert_eq!(raise_kind(true, true), Raise::AlreadyUp);
    }

    #[test]
    fn a_clean_pass_reports_nothing() {
        assert!(RouteRepair::default().is_clean());
        assert!(!RouteRepair {
            network_changed: true
        }
        .is_clean());
    }
}
