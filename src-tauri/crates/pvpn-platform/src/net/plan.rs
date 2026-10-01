// src-tauri/crates/pvpn-platform/src/net/plan.rs
//
// Pure half of the platform layer: what each OS *would* run, and how each OS's
// query output is read. No process is started and no syscall is made here, so
// all three platforms' plans are compiled and unit-tested from any host —
// including the Windows plan, which cannot otherwise be exercised on a Mac.
//
// The macOS plan is a transcription of the pre-split tun.rs. Its golden tests
// are the proof that the refactor did not change a single argument.

use std::net::{IpAddr, Ipv4Addr};

use crate::net::PhysicalRoute;
use crate::process::Argv;

/// The two halves of the default route. More specific than 0.0.0.0/0, so they
/// win over whatever the physical link installed without deleting it.
pub const SPLIT_LOW: &str = "0.0.0.0/1";
pub const SPLIT_HIGH: &str = "128.0.0.0/1";

/// Address assigned to the TUN device on every platform.
pub const DEVICE_ADDR: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);

// ===========================================================================
// macOS
// ===========================================================================

pub mod macos {
    use super::*;
    use anyhow::{anyhow, Result};

    pub const DEVICE: &str = "utun225";
    const ROUTE: &str = "/sbin/route";
    const IFCONFIG: &str = "/sbin/ifconfig";
    const PKILL: &str = "/usr/bin/pkill";

    /// What tun2socks is told to open. On macOS the bare device name, exactly
    /// as the pre-split build passed it.
    pub const DEVICE_ARG: &str = DEVICE;

    /// Argv for the engine, unchanged from the pre-split build: no `-mtu`, and
    /// `-loglevel info`. tun2socks creates the device itself from `-device`.
    pub fn tun2socks(bin: &std::path::Path, socks_port: u16) -> Argv {
        Argv::new(
            bin.to_string_lossy().to_string(),
            [
                "-device".to_string(),
                DEVICE_ARG.to_string(),
                "-proxy".to_string(),
                format!("socks5://127.0.0.1:{}", socks_port),
                "-loglevel".to_string(),
                "info".to_string(),
            ],
        )
    }

    pub fn default_route_query() -> Argv {
        // Deliberately `route` without a path: unchanged from the pre-split code.
        Argv::new("route", ["-n", "get", "default"])
    }

    pub fn host_route_add(ip: Ipv4Addr, via: &PhysicalRoute) -> Result<Argv> {
        match (&via.next_hop, &via.if_name) {
            (Some(gw), _) => Ok(Argv::new(
                ROUTE,
                ["-n", "add", "-host", &ip.to_string(), gw],
            )),
            (None, Some(iface)) => Ok(Argv::new(
                ROUTE,
                ["-n", "add", "-host", &ip.to_string(), "-interface", iface],
            )),
            (None, None) => Err(anyhow!(
                "no gateway and no interface for the host route to {}",
                ip
            )),
        }
    }

    pub fn host_route_delete(ip: Ipv4Addr) -> Argv {
        Argv::new(ROUTE, ["-n", "delete", "-host", &ip.to_string()])
    }

    pub fn host_route_query(ip: Ipv4Addr) -> Argv {
        Argv::new(ROUTE, ["-n", "get", "-host", &ip.to_string()])
    }

    pub fn split_default_add(half: &str) -> Argv {
        Argv::new(ROUTE, ["-n", "add", "-net", half, "-interface", DEVICE])
    }

    pub fn split_default_delete(half: &str) -> Argv {
        Argv::new(ROUTE, ["-n", "delete", "-net", half])
    }

    pub fn routing_table_query() -> Argv {
        Argv::new("netstat", ["-rn", "-f", "inet"])
    }

    pub fn device_probe() -> Argv {
        Argv::new(IFCONFIG, [DEVICE])
    }

    pub fn device_configure() -> Argv {
        let addr = DEVICE_ADDR.to_string();
        // Point-to-point: local and remote address are the same, as before.
        Argv::new(IFCONFIG, [DEVICE, &addr, &addr, "up"])
    }

    pub fn device_down() -> Argv {
        Argv::new(IFCONFIG, [DEVICE, "down"])
    }

    pub fn process_alive(name: &str) -> Argv {
        Argv::new("pgrep", ["-x", name])
    }

    pub fn kill_stray(name: &str) -> Argv {
        Argv::new(PKILL, ["-x", name])
    }

    pub fn kill_stray_force(name: &str) -> Argv {
        Argv::new(PKILL, ["-9", "-x", name])
    }

    /// Reads `route -n get default`.
    ///
    /// Semantics preserved exactly: the first `gateway:` line wins; a
    /// gateway-less default over another VPN's utun gets its own message,
    /// because that is a user-fixable situation.
    pub fn parse_default_route(text: &str) -> Result<PhysicalRoute> {
        let mut iface: Option<String> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("gateway:") {
                return Ok(PhysicalRoute {
                    next_hop: Some(rest.trim().to_string()),
                    if_name: iface,
                    if_index: None,
                });
            }
            if let Some(rest) = trimmed.strip_prefix("interface:") {
                iface = Some(rest.trim().to_string());
            }
        }
        if let Some(i) = iface {
            if i.starts_with("utun") {
                return Err(anyhow!(
                    "default route goes through {} — отключите другой VPN",
                    i
                ));
            }
        }
        Err(anyhow!("could not parse default gateway"))
    }

    /// Reads `route -n get -host <ip>`: the host route survives as long as the
    /// answer does not point back into our own tunnel.
    pub fn parse_host_route_ok(text: &str) -> bool {
        for line in text.lines() {
            if line.trim().starts_with("interface:") && line.contains(DEVICE) {
                return false;
            }
        }
        text.contains("gateway:") || text.contains("interface:")
    }

    /// Reads `netstat -rn -f inet`: both halves must still point at the device.
    /// netstat abbreviates the prefixes, hence "0/1" and "128.0/1".
    pub fn parse_split_defaults_ok(text: &str) -> bool {
        let has_low = text
            .lines()
            .any(|l| l.starts_with("0/1") && l.contains(DEVICE));
        let has_high = text
            .lines()
            .any(|l| l.starts_with("128.0/1") && l.contains(DEVICE));
        has_low && has_high
    }
}

// ===========================================================================
// Windows
// ===========================================================================

pub mod windows {
    use super::*;

    /// Adapter name requested from tun2socks. Wintun publishes it as the
    /// interface alias, which is how we find the adapter again (see
    /// net::windows::device_index). Chosen by us, so never localised.
    pub const DEVICE: &str = "ProxysVPN";
    pub const DEVICE_MASK: &str = "255.255.255.0";
    const NETSH: &str = "netsh";

    /// Writes go through netsh rather than IP Helper: we never parse its
    /// output, only check the exit code, so localisation is irrelevant, and it
    /// keeps the unsafe FFI surface down to the four read-only calls in
    /// net::windows. Reads go through IP Helper, because `route print` and
    /// `netsh ... show` are localised and cannot be parsed reliably.
    ///
    /// `store=active` keeps every change volatile: a reboot after a crash
    /// leaves no trace of the tunnel in the persistent route table.
    /// tun2socks takes `[driver://]name`; the bare name selects the platform
    /// default driver, which is Wintun here.
    pub const DEVICE_ARG: &str = DEVICE;

    /// Argv for the engine, unchanged from the pre-split build: no `-mtu`, and
    /// `-loglevel info`. tun2socks creates the device itself from `-device`.
    pub fn tun2socks(bin: &std::path::Path, socks_port: u16) -> Argv {
        Argv::new(
            bin.to_string_lossy().to_string(),
            [
                "-device".to_string(),
                DEVICE_ARG.to_string(),
                "-proxy".to_string(),
                format!("socks5://127.0.0.1:{}", socks_port),
                "-loglevel".to_string(),
                "info".to_string(),
            ],
        )
    }

    pub fn host_route_add(ip: Ipv4Addr, via: &PhysicalRoute) -> anyhow::Result<Argv> {
        let index = via.if_index.ok_or_else(|| {
            anyhow::anyhow!("no interface index for the host route to {}", ip)
        })?;
        let mut args = vec![
            "interface".to_string(),
            "ipv4".to_string(),
            "add".to_string(),
            "route".to_string(),
            format!("prefix={}/32", ip),
            format!("interface={}", index),
        ];
        // No next hop = the default is on-link (PPP, some cellular links):
        // route through the interface only, like macOS's `-interface`.
        if let Some(gw) = &via.next_hop {
            args.push(format!("nexthop={}", gw));
        }
        args.push("store=active".to_string());
        Ok(Argv::new(NETSH, args))
    }

    pub fn host_route_delete(ip: Ipv4Addr) -> Argv {
        Argv::new(
            NETSH,
            [
                "interface",
                "ipv4",
                "delete",
                "route",
                &format!("prefix={}/32", ip),
                "store=active",
            ],
        )
    }

    pub fn split_default_add(half: &str) -> Argv {
        Argv::new(
            NETSH,
            [
                "interface",
                "ipv4",
                "add",
                "route",
                &format!("prefix={}", half),
                &format!("interface={}", DEVICE),
                "store=active",
            ],
        )
    }

    pub fn split_default_delete(half: &str) -> Argv {
        Argv::new(
            NETSH,
            [
                "interface",
                "ipv4",
                "delete",
                "route",
                &format!("prefix={}", half),
                &format!("interface={}", DEVICE),
                "store=active",
            ],
        )
    }

    pub fn device_set_address() -> Argv {
        Argv::new(
            NETSH,
            [
                "interface",
                "ipv4",
                "set",
                "address",
                &format!("name={}", DEVICE),
                "source=static",
                &format!("address={}", DEVICE_ADDR),
                &format!("mask={}", DEVICE_MASK),
            ],
        )
    }

    /// Lowest interface metric: Windows breaks ties by metric when several
    /// adapters offer a route of the same length, and prefers the tunnel's
    /// source address for traffic it sends over it.
    pub fn device_set_metric() -> Argv {
        Argv::new(
            NETSH,
            [
                "interface",
                "ipv4",
                "set",
                "interface",
                &format!("interface={}", DEVICE),
                "metric=1",
                "store=active",
            ],
        )
    }

    /// Explicitly leaves the tunnel adapter without DNS servers.
    ///
    /// This is not a leak fix, it prevents one: the Windows DNS client queries
    /// every interface that has a server configured and takes the first answer.
    /// Nothing in this build listens for DNS inside the tunnel (xray has no
    /// `dns` section and no port-53 rule here), so pointing the adapter at
    /// 198.18.0.2 would break name resolution outright. Names keep resolving
    /// through the physical adapter's servers — exactly what macOS does today.
    /// Closing that leak needs a resolver inside the tunnel plus NRPT rules;
    /// it is deliberately out of scope and called out in the PR.
    pub fn device_clear_dns() -> Argv {
        Argv::new(
            NETSH,
            [
                "interface",
                "ipv4",
                "set",
                "dnsservers",
                &format!("name={}", DEVICE),
                "source=static",
                "address=none",
                "register=none",
            ],
        )
    }

    pub fn process_alive(image: &str) -> Argv {
        Argv::new(
            "tasklist",
            [
                "/FI",
                &format!("IMAGENAME eq {}", image),
                "/NH",
            ],
        )
    }

    pub fn kill_stray(image: &str) -> Argv {
        // /T also kills children: a sidecar that spawned helpers must not
        // outlive the kill, since nothing else reaps it on Windows.
        Argv::new("taskkill", ["/F", "/T", "/IM", image])
    }

    pub fn image_name(stem: &str) -> String {
        format!("{}.exe", stem)
    }

    /// `tasklist /NH` prints the image name in the first column when the
    /// process exists and a localised "no tasks" sentence when it does not,
    /// so look for the image name instead of interpreting the message.
    pub fn parse_tasklist_alive(text: &str, image: &str) -> bool {
        let needle = image.to_ascii_lowercase();
        text.lines()
            .any(|l| l.to_ascii_lowercase().contains(&needle))
    }
}

// ===========================================================================
// Linux
// ===========================================================================

/// Everything the root helper runs, as pure values.
///
/// `ip` and `resolvectl` are looked up as absolute paths at call time (pkexec
/// resets PATH, and distros disagree on /sbin vs /usr/sbin), so the programs
/// here are bare names and `net::linux_priv::tool` resolves them. The argument
/// lists are what matters and what these tests pin.
///
/// Routing shape is the same as macOS and Windows: two half-defaults on the
/// tunnel plus a /32 host route to the node over the physical link. The halves
/// beat the real 0.0.0.0/0 on prefix length without deleting it, so another
/// VPN's default and the ISP's survive untouched and come back when we leave.
pub mod linux {
    use super::*;
    use crate::net::linux_logic::LinuxDefaultRoute;

    pub const DEVICE: &str = "proxysvpn0";

    /// 198.18.0.0/15 (RFC 2544 benchmark range) — the subnet tun2socks' own
    /// documentation uses, and not routed on the public internet.
    pub const PREFIX: u8 = 15;

    /// tun2socks terminates TCP locally and re-opens it towards the SOCKS
    /// proxy, so the device is not an encapsulating tunnel and does not need a
    /// reduced MTU.
    pub const MTU: u32 = 1500;

    const IP: &str = "ip";
    const RESOLVECTL: &str = "resolvectl";

    /// `ip -4 route replace <ip>/32 [via <gw>] dev <iface>`.
    ///
    /// A link-scoped default (no gateway, e.g. `ppp0`) must be expressed as
    /// `dev <iface>` with no `via`, or `ip` rejects it.
    pub fn host_route_add(ip: Ipv4Addr, via: &LinuxDefaultRoute) -> Argv {
        let mut args = vec![
            "-4".to_string(),
            "route".to_string(),
            "replace".to_string(),
            format!("{}/32", ip),
        ];
        if let Some(gw) = via.gateway {
            args.push("via".to_string());
            args.push(gw.to_string());
        }
        args.push("dev".to_string());
        args.push(via.iface.clone());
        Argv::new(IP, args)
    }

    pub fn host_route_delete(ip: Ipv4Addr) -> Argv {
        Argv::new(
            IP,
            ["-4", "route", "del", &format!("{}/32", ip)],
        )
    }

    /// Asks the kernel which device would carry a packet to `ip`. Used to tell
    /// "the host route is still outside the tunnel" from "it got swallowed".
    pub fn host_route_query(ip: Ipv4Addr) -> Argv {
        Argv::new(IP, ["-4", "route", "get", &ip.to_string()])
    }

    /// `replace`, not `add`: a half left over from a crashed run must not make
    /// the whole connect fail.
    pub fn split_default_add(half: &str) -> Argv {
        Argv::new(IP, ["-4", "route", "replace", half, "dev", DEVICE])
    }

    pub fn split_default_delete(half: &str) -> Argv {
        Argv::new(IP, ["-4", "route", "del", half, "dev", DEVICE])
    }

    pub fn device_set_address() -> Argv {
        Argv::new(
            IP,
            [
                "-4",
                "addr",
                "replace",
                &format!("{}/{}", DEVICE_ADDR, PREFIX),
                "dev",
                DEVICE,
            ],
        )
    }

    pub fn device_up() -> Argv {
        Argv::new(
            IP,
            ["link", "set", "dev", DEVICE, "up", "mtu", &MTU.to_string()],
        )
    }

    /// Deletes the device outright rather than downing it: it belongs to
    /// tun2socks, and a leftover `proxysvpn0` would make the next run's
    /// `wait_for_device` succeed before the new engine is actually listening.
    pub fn device_delete() -> Argv {
        Argv::new(IP, ["link", "del", DEVICE])
    }

    pub fn tun2socks(bin: &std::path::Path, socks_port: u16) -> Argv {
        Argv::new(
            bin.to_string_lossy().to_string(),
            [
                "-device".to_string(),
                format!("tun://{}", DEVICE),
                "-proxy".to_string(),
                format!("socks5://127.0.0.1:{}", socks_port),
                "-mtu".to_string(),
                MTU.to_string(),
                // `warn`, never `warning`: the long form makes tun2socks exit.
                "-loglevel".to_string(),
                "warn".to_string(),
            ],
        )
    }

    // ------------------------------------------------------------------ DNS

    /// `resolvectl dns proxysvpn0 <servers…>`.
    pub fn resolved_set_servers(servers: &[IpAddr]) -> Argv {
        let mut args = vec!["dns".to_string(), DEVICE.to_string()];
        args.extend(servers.iter().map(|ip| ip.to_string()));
        Argv::new(RESOLVECTL, args)
    }

    /// `~.` makes the tunnel the resolver of last resort for every name, which
    /// is what stops systemd-resolved from asking the LAN router in parallel.
    pub fn resolved_set_domain() -> Argv {
        Argv::new(RESOLVECTL, ["domain", DEVICE, "~."])
    }

    pub fn resolved_set_default_route() -> Argv {
        Argv::new(RESOLVECTL, ["default-route", DEVICE, "yes"])
    }

    pub fn resolved_revert() -> Argv {
        Argv::new(RESOLVECTL, ["revert", DEVICE])
    }

    pub fn resolved_flush() -> Argv {
        Argv::new(RESOLVECTL, ["flush-caches"])
    }

    /// Crash sweep for a tun2socks the helper no longer has a handle for.
    /// `-x` so it cannot match an unrelated command line.
    pub fn kill_stray(stem: &str) -> Argv {
        Argv::new("pkill", ["-9", "-x", stem])
    }
}

// ===========================================================================
// Tests — all three plans, from any host
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn gw_route() -> PhysicalRoute {
        PhysicalRoute {
            next_hop: Some("192.168.1.1".into()),
            if_name: Some("en0".into()),
            if_index: Some(11),
        }
    }

    fn onlink_route() -> PhysicalRoute {
        PhysicalRoute {
            next_hop: None,
            if_name: Some("ppp0".into()),
            if_index: Some(7),
        }
    }

    const NODE: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 7);

    // ---------------------------------------------------------------- macOS
    // Golden argv. These literals are the pre-split behaviour; changing one
    // changes what ships to users on macOS.

    #[test]
    fn macos_argv_is_unchanged() {
        assert_eq!(
            macos::default_route_query().to_string(),
            "route -n get default"
        );
        assert_eq!(
            macos::host_route_add(NODE, &gw_route()).expect("plan").to_string(),
            "/sbin/route -n add -host 203.0.113.7 192.168.1.1"
        );
        assert_eq!(
            macos::host_route_delete(NODE).to_string(),
            "/sbin/route -n delete -host 203.0.113.7"
        );
        assert_eq!(
            macos::host_route_query(NODE).to_string(),
            "/sbin/route -n get -host 203.0.113.7"
        );
        assert_eq!(
            macos::split_default_add(SPLIT_LOW).to_string(),
            "/sbin/route -n add -net 0.0.0.0/1 -interface utun225"
        );
        assert_eq!(
            macos::split_default_add(SPLIT_HIGH).to_string(),
            "/sbin/route -n add -net 128.0.0.0/1 -interface utun225"
        );
        assert_eq!(
            macos::split_default_delete(SPLIT_LOW).to_string(),
            "/sbin/route -n delete -net 0.0.0.0/1"
        );
        assert_eq!(
            macos::routing_table_query().to_string(),
            "netstat -rn -f inet"
        );
        assert_eq!(macos::device_probe().to_string(), "/sbin/ifconfig utun225");
        assert_eq!(
            macos::device_configure().to_string(),
            "/sbin/ifconfig utun225 198.18.0.1 198.18.0.1 up"
        );
        assert_eq!(macos::device_down().to_string(), "/sbin/ifconfig utun225 down");
        assert_eq!(macos::process_alive("tun2socks").to_string(), "pgrep -x tun2socks");
        assert_eq!(
            macos::kill_stray("tun2socks").to_string(),
            "/usr/bin/pkill -x tun2socks"
        );
        assert_eq!(
            macos::kill_stray_force("xray").to_string(),
            "/usr/bin/pkill -9 -x xray"
        );
    }

    #[test]
    fn macos_host_route_falls_back_to_interface_without_gateway() {
        assert_eq!(
            macos::host_route_add(NODE, &onlink_route()).expect("plan").to_string(),
            "/sbin/route -n add -host 203.0.113.7 -interface ppp0"
        );
        let nothing = PhysicalRoute { next_hop: None, if_name: None, if_index: None };
        assert!(macos::host_route_add(NODE, &nothing).is_err());
    }

    #[test]
    fn macos_parses_default_route_with_gateway() {
        let text = "   route to: default\ndestination: default\n       mask: default\n    gateway: 192.168.1.1\n  interface: en0\n      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING,GLOBAL>\n";
        let r = macos::parse_default_route(text).expect("parse");
        assert_eq!(r.next_hop.as_deref(), Some("192.168.1.1"));
    }

    #[test]
    fn macos_rejects_default_route_through_another_vpn() {
        let text = "   route to: default\ndestination: default\n  interface: utun4\n";
        let err = macos::parse_default_route(text).expect_err("must refuse");
        assert!(err.to_string().contains("utun4"), "{}", err);
        assert!(err.to_string().contains("отключите другой VPN"), "{}", err);
    }

    #[test]
    fn macos_rejects_unparsable_default_route() {
        let err = macos::parse_default_route("").expect_err("must refuse");
        assert_eq!(err.to_string(), "could not parse default gateway");
    }

    #[test]
    fn macos_host_route_check_detects_loop_into_tunnel() {
        let good = "    gateway: 192.168.1.1\n  interface: en0\n";
        let looped = "  interface: utun225\n";
        assert!(macos::parse_host_route_ok(good));
        assert!(!macos::parse_host_route_ok(looped));
        assert!(!macos::parse_host_route_ok(""));
    }

    #[test]
    fn macos_split_default_check_needs_both_halves() {
        let both = "0/1                utun225            USc          utun225\n128.0/1            utun225            USc          utun225\n";
        let one = "0/1                utun225            USc          utun225\n";
        let other_dev = "0/1                utun9              USc          utun9\n128.0/1            utun9              USc          utun9\n";
        assert!(macos::parse_split_defaults_ok(both));
        assert!(!macos::parse_split_defaults_ok(one));
        assert!(!macos::parse_split_defaults_ok(other_dev));
    }

    // -------------------------------------------------------------- Windows

    #[test]
    fn windows_host_route_uses_index_and_next_hop() {
        assert_eq!(
            windows::host_route_add(NODE, &gw_route()).expect("plan").to_string(),
            "netsh interface ipv4 add route prefix=203.0.113.7/32 interface=11 nexthop=192.168.1.1 store=active"
        );
    }

    #[test]
    fn windows_host_route_omits_next_hop_on_link() {
        assert_eq!(
            windows::host_route_add(NODE, &onlink_route()).expect("plan").to_string(),
            "netsh interface ipv4 add route prefix=203.0.113.7/32 interface=7 store=active"
        );
    }

    #[test]
    fn windows_host_route_needs_an_interface_index() {
        let no_index = PhysicalRoute {
            next_hop: Some("192.168.1.1".into()),
            if_name: Some("Ethernet".into()),
            if_index: None,
        };
        assert!(windows::host_route_add(NODE, &no_index).is_err());
    }

    #[test]
    fn windows_route_and_device_argv() {
        assert_eq!(
            windows::host_route_delete(NODE).to_string(),
            "netsh interface ipv4 delete route prefix=203.0.113.7/32 store=active"
        );
        assert_eq!(
            windows::split_default_add(SPLIT_LOW).to_string(),
            "netsh interface ipv4 add route prefix=0.0.0.0/1 interface=ProxysVPN store=active"
        );
        assert_eq!(
            windows::split_default_delete(SPLIT_HIGH).to_string(),
            "netsh interface ipv4 delete route prefix=128.0.0.0/1 interface=ProxysVPN store=active"
        );
        assert_eq!(
            windows::device_set_address().to_string(),
            "netsh interface ipv4 set address name=ProxysVPN source=static address=198.18.0.1 mask=255.255.255.0"
        );
        assert_eq!(
            windows::device_set_metric().to_string(),
            "netsh interface ipv4 set interface interface=ProxysVPN metric=1 store=active"
        );
        assert_eq!(
            windows::device_clear_dns().to_string(),
            "netsh interface ipv4 set dnsservers name=ProxysVPN source=static address=none register=none"
        );
        assert_eq!(
            windows::kill_stray("tun2socks.exe").to_string(),
            "taskkill /F /T /IM tun2socks.exe"
        );
        assert_eq!(windows::image_name("xray"), "xray.exe");
    }

    /// Every write goes to the volatile store, so a crash cannot leave a
    /// persistent route behind.
    #[test]
    fn windows_route_writes_are_volatile() {
        for argv in [
            windows::host_route_add(NODE, &gw_route()).expect("plan"),
            windows::host_route_delete(NODE),
            windows::split_default_add(SPLIT_LOW),
            windows::split_default_delete(SPLIT_LOW),
        ] {
            assert!(
                argv.args.iter().any(|a| a == "store=active"),
                "missing store=active in {}",
                argv
            );
        }
    }

    #[test]
    fn windows_tasklist_parsing_ignores_localised_text() {
        let running = "tun2socks.exe                 7420 Console                    1     18 236 КБ\r\n";
        let absent = "ИНФОРМАЦИЯ: нет запущенных задач, соответствующих указанным критериям.\r\n";
        assert!(windows::parse_tasklist_alive(running, "tun2socks.exe"));
        assert!(!windows::parse_tasklist_alive(absent, "tun2socks.exe"));
        assert!(!windows::parse_tasklist_alive("", "tun2socks.exe"));
        // Case differences in the image column must not hide a live process.
        assert!(windows::parse_tasklist_alive("TUN2SOCKS.EXE 1 Console", "tun2socks.exe"));
    }

    // ---------------------------------------------------------------- Linux

    fn linux_route(gw: Option<[u8; 4]>, iface: &str) -> crate::net::linux_logic::LinuxDefaultRoute {
        crate::net::linux_logic::LinuxDefaultRoute {
            iface: iface.to_string(),
            gateway: gw.map(Ipv4Addr::from),
            metric: 100,
        }
    }

    #[test]
    fn linux_routes_have_the_expected_shape() {
        assert_eq!(
            linux::split_default_add(SPLIT_LOW).to_string(),
            "ip -4 route replace 0.0.0.0/1 dev proxysvpn0"
        );
        assert_eq!(
            linux::split_default_delete(SPLIT_HIGH).to_string(),
            "ip -4 route del 128.0.0.0/1 dev proxysvpn0"
        );
        assert_eq!(
            linux::device_set_address().to_string(),
            "ip -4 addr replace 198.18.0.1/15 dev proxysvpn0"
        );
        assert_eq!(
            linux::device_up().to_string(),
            "ip link set dev proxysvpn0 up mtu 1500"
        );
        assert_eq!(linux::device_delete().to_string(), "ip link del proxysvpn0");
    }

    /// `replace` and not `add`: a half-default or address left over from a
    /// crashed run must not make the next connect fail.
    #[test]
    fn linux_route_writes_are_idempotent() {
        for argv in [
            linux::split_default_add(SPLIT_LOW),
            linux::device_set_address(),
            linux::host_route_add(Ipv4Addr::new(203, 0, 113, 7), &linux_route(None, "eth0")),
        ] {
            assert!(
                argv.args.contains(&"replace".to_string()),
                "{} must be idempotent",
                argv
            );
            assert!(!argv.args.contains(&"add".to_string()), "{}", argv);
        }
    }

    #[test]
    fn linux_host_route_uses_via_when_a_gateway_exists() {
        assert_eq!(
            linux::host_route_add(
                Ipv4Addr::new(203, 0, 113, 7),
                &linux_route(Some([10, 0, 2, 2]), "enp0s3")
            )
            .to_string(),
            "ip -4 route replace 203.0.113.7/32 via 10.0.2.2 dev enp0s3"
        );
    }

    /// A link-scoped default (mobile broadband, `default dev ppp0`) has no
    /// gateway, and `ip` rejects a `via` with an empty argument.
    #[test]
    fn linux_host_route_falls_back_to_the_device_for_link_scoped_defaults() {
        let argv = linux::host_route_add(
            Ipv4Addr::new(203, 0, 113, 7),
            &linux_route(None, "ppp0"),
        );
        assert_eq!(
            argv.to_string(),
            "ip -4 route replace 203.0.113.7/32 dev ppp0"
        );
        assert!(!argv.args.contains(&"via".to_string()));
    }

    #[test]
    fn linux_host_route_delete_names_a_single_address() {
        assert_eq!(
            linux::host_route_delete(Ipv4Addr::new(203, 0, 113, 7)).to_string(),
            "ip -4 route del 203.0.113.7/32"
        );
        assert_eq!(
            linux::host_route_query(Ipv4Addr::new(203, 0, 113, 7)).to_string(),
            "ip -4 route get 203.0.113.7"
        );
    }

    #[test]
    fn linux_tun2socks_argv_matches_the_device_and_port() {
        let argv = linux::tun2socks(std::path::Path::new("/opt/pvpn/tun2socks"), 10808);
        assert_eq!(argv.program, "/opt/pvpn/tun2socks");
        assert_eq!(
            argv.args,
            vec![
                "-device",
                "tun://proxysvpn0",
                "-proxy",
                "socks5://127.0.0.1:10808",
                "-mtu",
                "1500",
                "-loglevel",
                "warn",
            ]
        );
        // `warning` would make tun2socks exit immediately; only `warn` works.
        assert!(!argv.args.contains(&"warning".to_string()));
    }

    #[test]
    fn linux_dns_argv_covers_servers_domain_and_revert() {
        let servers: Vec<IpAddr> = vec![
            "1.1.1.1".parse().expect("ip"),
            "1.0.0.1".parse().expect("ip"),
        ];
        assert_eq!(
            linux::resolved_set_servers(&servers).to_string(),
            "resolvectl dns proxysvpn0 1.1.1.1 1.0.0.1"
        );
        assert_eq!(
            linux::resolved_set_domain().to_string(),
            "resolvectl domain proxysvpn0 ~."
        );
        assert_eq!(
            linux::resolved_revert().to_string(),
            "resolvectl revert proxysvpn0"
        );
    }

    #[test]
    fn linux_kill_stray_is_exact_match_only() {
        let argv = linux::kill_stray("tun2socks");
        assert_eq!(argv.to_string(), "pkill -9 -x tun2socks");
        // Without -x this would also kill, say, "tun2socks-wrapper".
        assert!(argv.args.contains(&"-x".to_string()));
    }

    // ------------------------------------------------- cross-platform shape

    /// Both halves of the default route, on every platform, and nothing wider:
    /// the physical default must stay in the table so the host route to the
    /// node keeps working.
    #[test]
    fn split_halves_never_touch_the_default_route() {
        for half in [SPLIT_LOW, SPLIT_HIGH] {
            assert!(half.ends_with("/1"), "{}", half);
        }
        assert_ne!(SPLIT_LOW, SPLIT_HIGH);
    }

    #[test]
    fn device_names_differ_per_platform_but_address_is_shared() {
        assert_eq!(DEVICE_ADDR.to_string(), "198.18.0.1");
        assert_ne!(macos::DEVICE, windows::DEVICE);
        assert_ne!(windows::DEVICE, linux::DEVICE);
    }
}
