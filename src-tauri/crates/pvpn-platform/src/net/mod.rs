// src-tauri/crates/pvpn-platform/src/net/mod.rs
//
// The TUN device and the routing around it.
//
// CONTRACT — every platform module (`macos`, `windows`, `linux`) exports
// exactly these items, and this file re-exports them one by one so that a
// missing or mistyped implementation fails here instead of at some call site
// in the GUI crate:
//
//   const DEVICE: &str                    name of the TUN device
//   fn  tun2socks_device_arg() -> String  value for tun2socks' `-device`
//   async physical_route() -> Result<PhysicalRoute>
//                                         how packets leave the machine
//                                         *outside* the tunnel; reads the
//                                         physical default route, which stays
//                                         in the table while our two halves
//                                         are installed
//   async add_host_route(dest, &PhysicalRoute) -> Result<()>
//   async delete_host_route(dest)         best effort
//   async host_route_ok(dest) -> bool     still pointing outside the tunnel?
//   async add_split_defaults() -> Result<()>
//   async delete_split_defaults()         best effort
//   async split_defaults_ok() -> bool
//   async wait_for_device(Duration) -> Result<()>
//                                         the device is created by tun2socks,
//                                         not by us; this waits for it
//   async configure_device() -> Result<()> address (+ platform extras)
//   async device_down()                   best effort
//   async engine_alive(stem) -> bool
//   async kill_stray(stem) / kill_stray_force(stem)
//   fn    sync_cleanup(&[Ipv4Addr])       blocking sweep for the exit path
//
// `stem` is always the plain sidecar name ("tun2socks", "xray", "hysteria");
// the platform module appends ".exe" where that is what the OS expects.

use std::net::Ipv4Addr;

pub mod plan;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as sys;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as sys;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as sys;

pub use sys::{
    add_host_route, add_split_defaults, configure_device, delete_host_route,
    delete_split_defaults, device_down, engine_alive, host_route_ok, kill_stray,
    kill_stray_force, physical_route, split_defaults_ok, sync_cleanup,
    tun2socks_device_arg, wait_for_device, DEVICE,
};

/// Address of the TUN device, identical on every platform.
pub use plan::DEVICE_ADDR;

/// How traffic leaves the machine without going through the tunnel.
///
/// `next_hop` is kept as the platform's own text (macOS hands back whatever
/// `route -n get default` printed, Windows a dotted IPv4) because it is passed
/// straight back to the platform's route command. `None` means the default is
/// on-link and the route must be bound to the interface instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalRoute {
    pub next_hop: Option<String>,
    pub if_name: Option<String>,
    pub if_index: Option<u32>,
}

impl std::fmt::Display for PhysicalRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.next_hop, &self.if_name, self.if_index) {
            (Some(gw), Some(name), _) => write!(f, "{} via {}", gw, name),
            (Some(gw), None, Some(idx)) => write!(f, "{} via if#{}", gw, idx),
            (Some(gw), None, None) => write!(f, "{}", gw),
            (None, Some(name), _) => write!(f, "on-link {}", name),
            (None, None, Some(idx)) => write!(f, "on-link if#{}", idx),
            (None, None, None) => write!(f, "unknown"),
        }
    }
}

/// Steps of a tunnel teardown, in the only order that is safe.
///
/// Rationale, in the order the variants appear:
///   * the split defaults go first, so the OS is already routing through the
///     physical link before anything else disappears — otherwise traffic
///     blackholes against a half-dismantled device;
///   * the host route to the node goes next: while the split defaults existed
///     it was the only thing keeping the node reachable, and removing it
///     earlier would cut the engines off mid-shutdown;
///   * then the device itself;
///   * then the engine we own, by handle;
///   * and only then a sweep for strays, which is the fallback for a sidecar
///     that outlived its parent (a crash, or a kill the handle never saw).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownStep {
    SplitDefaults,
    HostRoute,
    DeviceDown,
    KillOwnedEngine,
    KillStrayEngines,
}

pub const TEARDOWN_ORDER: [TeardownStep; 5] = [
    TeardownStep::SplitDefaults,
    TeardownStep::HostRoute,
    TeardownStep::DeviceDown,
    TeardownStep::KillOwnedEngine,
    TeardownStep::KillStrayEngines,
];

/// Sidecars that must not survive a teardown, in kill order.
pub const ENGINE_STEMS: [&str; 3] = ["tun2socks", "xray", "hysteria"];

/// Convenience for callers that only have the device name.
pub fn is_our_device(name: &str) -> bool {
    name == DEVICE
}

/// Parses a route-hint file written by an earlier run.
///
/// Format is the pre-split one (`pid=…` / `server_ip=…` lines), so an upgrade
/// still cleans up after a previous version that crashed.
pub fn parse_route_hint(contents: &str) -> Vec<Ipv4Addr> {
    contents
        .lines()
        .filter_map(|l| l.trim().strip_prefix("server_ip="))
        .filter_map(|ip| ip.trim().parse::<Ipv4Addr>().ok())
        .collect()
}

/// Serialises the route hint. Kept identical to the pre-split layout.
pub fn format_route_hint(pid: u32, server_ip: Ipv4Addr) -> String {
    format!("pid={}\nserver_ip={}\n", pid, server_ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_removes_routes_before_the_device_disappears() {
        let pos = |s: TeardownStep| {
            TEARDOWN_ORDER
                .iter()
                .position(|x| *x == s)
                .expect("step present")
        };
        assert!(pos(TeardownStep::SplitDefaults) < pos(TeardownStep::DeviceDown));
        assert!(pos(TeardownStep::HostRoute) < pos(TeardownStep::DeviceDown));
        // The node must stay reachable until the tunnel's defaults are gone.
        assert!(pos(TeardownStep::SplitDefaults) < pos(TeardownStep::HostRoute));
        // Our own child first, the blunt sweep last.
        assert!(pos(TeardownStep::KillOwnedEngine) < pos(TeardownStep::KillStrayEngines));
        assert!(pos(TeardownStep::DeviceDown) < pos(TeardownStep::KillOwnedEngine));
    }

    #[test]
    fn teardown_order_has_no_duplicates() {
        let mut seen = Vec::new();
        for step in TEARDOWN_ORDER {
            assert!(!seen.contains(&step), "duplicate step {:?}", step);
            seen.push(step);
        }
        assert_eq!(seen.len(), TEARDOWN_ORDER.len());
    }

    #[test]
    fn engine_stems_cover_every_sidecar() {
        assert!(ENGINE_STEMS.contains(&"tun2socks"));
        assert!(ENGINE_STEMS.contains(&"xray"));
        assert!(ENGINE_STEMS.contains(&"hysteria"));
    }

    #[test]
    fn route_hint_round_trips() {
        let ip: Ipv4Addr = "203.0.113.7".parse().expect("ip");
        let text = format_route_hint(4242, ip);
        assert_eq!(text, "pid=4242\nserver_ip=203.0.113.7\n");
        assert_eq!(parse_route_hint(&text), vec![ip]);
    }

    #[test]
    fn route_hint_ignores_junk() {
        assert!(parse_route_hint("").is_empty());
        assert!(parse_route_hint("pid=1\nserver_ip=not-an-ip\n").is_empty());
        assert!(parse_route_hint("server_ip=2001:db8::1\n").is_empty());
        assert_eq!(
            parse_route_hint("pid=1\nserver_ip=198.51.100.9\nserver_ip=203.0.113.7\n").len(),
            2
        );
    }

    #[test]
    fn physical_route_display_is_readable() {
        let r = PhysicalRoute {
            next_hop: Some("192.168.1.1".into()),
            if_name: Some("en0".into()),
            if_index: Some(11),
        };
        assert_eq!(r.to_string(), "192.168.1.1 via en0");
        let onlink = PhysicalRoute { next_hop: None, if_name: None, if_index: Some(7) };
        assert_eq!(onlink.to_string(), "on-link if#7");
    }

    #[test]
    fn device_name_is_recognised() {
        assert!(is_our_device(DEVICE));
        assert!(!is_our_device("en0"));
    }
}
