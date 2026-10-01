// src-tauri/crates/pvpn-platform/src/net/mod.rs
//
// The TUN device and everything around it: routes, DNS, the engine process and
// the privileges needed to touch them.
//
// ---------------------------------------------------------------------------
// THE CONTRACT
// ---------------------------------------------------------------------------
// Every platform module (`macos`, `windows`, `linux`) exports exactly these
// items, and this file re-exports them one by one, so a missing or mistyped
// implementation fails *here* rather than at a call site in the GUI crate:
//
//   const DEVICE: &str                     name of the TUN device
//   async preflight() -> Result<()>        fail before anything is changed:
//                                          privileges, helper, authorization
//   async up(&TunPlan) -> Result<()>       raise the tunnel; on failure it must
//                                          leave nothing behind
//   async ensure(&TunPlan) -> Result<()>   supervisor tick: re-assert routes
//                                          and DNS that the OS dropped
//   async retarget(Option<Ipv4Addr>, &TunPlan) -> Result<()>
//                                          move the live tunnel to another
//                                          node: the new host route goes in
//                                          before the old one goes out, and
//                                          nothing else is touched
//   async down(Option<Ipv4Addr>) -> Result<()>
//                                          tear down; idempotent, safe when
//                                          nothing is up
//   async engine_alive() -> bool           is tun2socks still running?
//   async physical_route() -> Result<PhysicalRoute>
//                                          how the machine reaches the
//                                          internet outside the tunnel, with
//                                          the interface name the engine binds
//                                          its sockets to
//   fn    purge_stale(&[Ipv4Addr])         blocking crash recovery, for the
//                                          startup and exit paths where there
//                                          is no runtime to await on
//
// Windows and Linux also export two read-only facts the GUI's supervisor
// samples every tick, which the macOS GUI reads itself from getifaddrs
// (src/probe.rs) exactly as it did before this layer existed:
//
//   fn    device_counters() -> Option<IfCounters>
//                                          byte counters of the tunnel device
//   fn    has_usable_link() -> bool        is there any way out at all
//
// ---------------------------------------------------------------------------
// WHY THE CONTRACT IS COARSE
// ---------------------------------------------------------------------------
// An earlier draft of this layer exposed the individual steps (add_host_route,
// add_split_defaults, configure_device, …) and let the GUI sequence them. That
// is the right shape only while the GUI is the privileged process, which is
// true on macOS (root via launchctl) and on Windows (requireAdministrator) but
// *cannot* be true on Linux: a root process cannot connect to a Wayland
// compositor, so the GUI stays unprivileged there and a small root helper owns
// the device. Two consequences decide the granularity:
//
//   1. On Linux every step would be an IPC call across a privilege boundary.
//      Three coarse operations keep that boundary narrow and auditable — the
//      helper is told "raise a tunnel to this address", never "add this route
//      to that interface", so a compromised GUI cannot aim root's `ip` at an
//      arbitrary target. See helper/proto.rs for the validation.
//   2. `up` has to be atomic: whatever it changed must be undone when a later
//      step fails. That is only expressible where the steps live together.
//
// So the fine-grained steps still exist — they are each platform's private
// vocabulary, in `macos.rs`, `windows.rs` and (behind the helper) `linux.rs`,
// with their exact argv produced by `plan` and pinned by golden tests.
//
// ---------------------------------------------------------------------------
// INVARIANTS THAT MUST SURVIVE REFACTORS
// ---------------------------------------------------------------------------
//   * macOS runs byte-for-byte the commands it ran before this layer existed.
//     `plan::macos` produces them and golden tests compare the argv, so a typo
//     cannot silently change behaviour for the users who are on it today.
//   * This crate has no tauri/reqwest/rustls dependency and nothing in it needs
//     a C compiler, so the Windows and Linux code type-checks from a Mac:
//         cargo check -p pvpn-platform --target x86_64-pc-windows-msvc
//         cargo check -p pvpn-platform --target x86_64-unknown-linux-gnu
//     That is the only verification those two ports can get here, so it must
//     not be given up for convenience.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

pub mod linux_logic;
pub mod plan;

/// tun2socks plumbing shared by the platforms that spawn it themselves
/// (macOS and Windows). On Linux the root helper owns the process instead.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod engine;

/// The tunnel sequence for the platforms whose GUI is itself privileged, shared
/// by macOS and Windows so the order of operations cannot drift between them.
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod local;

/// The privileged half of the Linux backend: what the root helper runs. Split
/// from `linux` (the unprivileged client) because only one of the two is ever
/// the right thing to call, and mixing them is how a GUI ends up trying to
/// write a routing table it has no rights to.
#[cfg(target_os = "linux")]
pub mod linux_priv;

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
    down, engine_alive, ensure, physical_route, preflight, purge_stale, retarget, up, DEVICE,
};

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub use sys::{device_counters, has_usable_link};

/// Cumulative byte counters of one interface, as the OS reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IfCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Address of the TUN device, identical on every platform.
pub use plan::DEVICE_ADDR;

/// Everything a platform needs to raise or re-assert the tunnel.
///
/// `server_ip` is an `Ipv4Addr` and not a string on purpose: it is resolved and
/// validated once, at the edge, and can therefore never reach a command line as
/// anything but four octets. The Linux wire protocol re-validates it anyway,
/// because the helper must not trust its peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunPlan {
    /// Resolved IPv4 address of the VPN node. Never logged: node addresses are
    /// not public information.
    pub server_ip: Ipv4Addr,
    /// Loopback SOCKS5 port of the engine (xray or hysteria) in front of
    /// tun2socks.
    pub socks_port: u16,
    /// Absolute path to the tun2socks sidecar.
    pub tun2socks: PathBuf,
    /// Resolvers to publish on the tunnel interface, where the platform needs
    /// them. Linux does; macOS and Windows leave the system resolver alone (see
    /// `configure_dns` in each module for why).
    pub dns: Vec<IpAddr>,
}

impl TunPlan {
    /// Resolvers as strings, for the Linux helper's JSON protocol.
    pub fn dns_strings(&self) -> Vec<String> {
        self.dns.iter().map(|ip| ip.to_string()).collect()
    }
}

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
///   * then DNS, which must not be left pointing into a tunnel that is going
///     away, or every name lookup on the machine stops working;
///   * then the device itself;
///   * then the engine we own, by handle;
///   * and only then a sweep for strays, which is the fallback for a sidecar
///     that outlived its parent (a crash, or a kill the handle never saw).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownStep {
    SplitDefaults,
    HostRoute,
    RestoreDns,
    DeviceDown,
    KillOwnedEngine,
    KillStrayEngines,
}

pub const TEARDOWN_ORDER: [TeardownStep; 6] = [
    TeardownStep::SplitDefaults,
    TeardownStep::HostRoute,
    TeardownStep::RestoreDns,
    TeardownStep::DeviceDown,
    TeardownStep::KillOwnedEngine,
    TeardownStep::KillStrayEngines,
];

/// Sidecars that must not survive a teardown, in kill order.
pub const ENGINE_STEMS: [&str; 3] = ["tun2socks", "xray", "hysteria"];

/// The engine the tunnel itself depends on; the other two carry traffic into it.
pub const TUNNEL_ENGINE: &str = "tun2socks";

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
        // A resolver on a device that no longer exists breaks every lookup.
        assert!(pos(TeardownStep::RestoreDns) < pos(TeardownStep::DeviceDown));
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
        assert!(ENGINE_STEMS.contains(&TUNNEL_ENGINE));
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
        // Anything that could become an extra argument must not survive.
        assert!(parse_route_hint("server_ip=; rm -rf /\n").is_empty());
        assert!(parse_route_hint("server_ip=-net default\n").is_empty());
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
        let onlink = PhysicalRoute {
            next_hop: None,
            if_name: None,
            if_index: Some(7),
        };
        assert_eq!(onlink.to_string(), "on-link if#7");
    }

    #[test]
    fn device_name_is_recognised() {
        assert!(is_our_device(DEVICE));
        assert!(!is_our_device("en0"));
    }

    #[test]
    fn device_name_fits_the_kernel_limit() {
        // IFNAMSIZ is 16 including the NUL on Linux; utun names are shorter.
        assert!(!DEVICE.is_empty());
        assert!(DEVICE.len() < 16, "{} is too long for an interface", DEVICE);
        assert!(DEVICE.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn plan_renders_dns_for_the_wire() {
        let plan = TunPlan {
            server_ip: Ipv4Addr::new(203, 0, 113, 7),
            socks_port: 10808,
            tun2socks: PathBuf::from("/opt/proxysvpn/tun2socks"),
            dns: vec![
                "1.1.1.1".parse().expect("ip"),
                "2606:4700:4700::1111".parse().expect("ip"),
            ],
        };
        assert_eq!(
            plan.dns_strings(),
            vec!["1.1.1.1".to_string(), "2606:4700:4700::1111".to_string()]
        );
    }
}
