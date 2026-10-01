// src-tauri/src/tun/sys/linux_logic.rs
//! Host-independent half of the Linux tunnel backend: table parsers, resolver
//! file rendering and the `/etc/resolv.conf` backup dance.
//!
//! Compiled on **every** host, not only Linux, so that `cargo test` on the
//! developer's Mac — the only machine available for this port — exercises the
//! parts that are easy to get wrong. The command-running half lives in
//! `linux/net.rs` and only builds on Linux.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
// Reason for the allow: on macOS nothing calls these, but the tests must run.

use std::net::{IpAddr, Ipv4Addr};

/// The physical default route we must pin the node's address to, so that the
/// tunnel's own packets do not re-enter the tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalRoute {
    pub iface: String,
    /// `None` for a link-scoped default (`default dev ppp0` with no gateway).
    pub gateway: Option<Ipv4Addr>,
    pub metric: u32,
}

/// Interfaces that are never a physical exit: our own device, other tunnels.
/// `ppp*` is deliberately absent — a mobile-broadband link is a real exit.
pub fn is_tunnel_iface(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "proxysvpn", "tun", "tap", "wg", "utun", "tailscale", "zt", "nordlynx", "ipsec",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Decode one address field of `/proc/net/route`: 8 hex digits, host byte
/// order, which on every Linux platform we ship is little-endian.
pub fn hex_le_to_ipv4(field: &str) -> Option<Ipv4Addr> {
    if field.len() != 8 || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let raw = u32::from_str_radix(field, 16).ok()?;
    Some(Ipv4Addr::from(raw.to_le_bytes()))
}

const RTF_UP: u32 = 0x0001;

struct RouteRow<'a> {
    iface: &'a str,
    dest: Ipv4Addr,
    gateway: Ipv4Addr,
    flags: u32,
    metric: u32,
    mask: Ipv4Addr,
}

fn parse_row<'a>(line: &'a str) -> Option<RouteRow<'a>> {
    let mut cols = line.split_whitespace();
    let iface = cols.next()?;
    let dest = hex_le_to_ipv4(cols.next()?)?;
    let gateway = hex_le_to_ipv4(cols.next()?)?;
    let flags = u32::from_str_radix(cols.next()?, 16).ok()?;
    let _refcnt = cols.next()?;
    let _use = cols.next()?;
    let metric: u32 = cols.next()?.parse().ok()?;
    let mask = hex_le_to_ipv4(cols.next()?)?;
    Some(RouteRow {
        iface,
        dest,
        gateway,
        flags,
        metric,
        mask,
    })
}

/// Pick the physical default route out of `/proc/net/route`.
///
/// `/proc/net/route` is a kernel ABI: no localisation, no iproute2 version
/// drift. We read it instead of `ip route show default` for the same reason the
/// macOS backend reads `netstat -rn` instead of `route get default`: once the
/// tunnel is up, our own `0.0.0.0/1` half-default is *more* specific than
/// `0.0.0.0/0`, and a "what would you do with this packet" query answers with
/// our own device.
///
/// Returns the up default route with the lowest metric that is not a tunnel.
pub fn parse_physical_default(text: &str) -> Option<PhysicalRoute> {
    let mut best: Option<PhysicalRoute> = None;
    for line in text.lines().skip(1) {
        let Some(row) = parse_row(line) else { continue };
        if row.flags & RTF_UP == 0 {
            continue;
        }
        if !row.dest.is_unspecified() || !row.mask.is_unspecified() {
            continue;
        }
        if row.iface == "lo" || is_tunnel_iface(row.iface) {
            continue;
        }
        let candidate = PhysicalRoute {
            iface: row.iface.to_string(),
            gateway: if row.gateway.is_unspecified() {
                None
            } else {
                Some(row.gateway)
            },
            metric: row.metric,
        };
        match &best {
            Some(current) if current.metric <= candidate.metric => {}
            _ => best = Some(candidate),
        }
    }
    best
}

/// Are both halves of the split default present on `iface`?
pub fn parse_split_defaults(text: &str, iface: &str) -> (bool, bool) {
    let half = Ipv4Addr::new(128, 0, 0, 0);
    let (mut low, mut high) = (false, false);
    for line in text.lines().skip(1) {
        let Some(row) = parse_row(line) else { continue };
        if row.iface != iface || row.mask != half || row.flags & RTF_UP == 0 {
            continue;
        }
        if row.dest.is_unspecified() {
            low = true;
        } else if row.dest == half {
            high = true;
        }
    }
    (low, high)
}

/// Extract the `dev <name>` token from `ip -4 route get <ip>` output.
/// iproute2 is not translated, so this is locale-safe.
pub fn parse_ip_route_get_dev(text: &str) -> Option<String> {
    let mut it = text.split_whitespace();
    while let Some(token) = it.next() {
        if token == "dev" {
            return it.next().map(|s| s.to_string());
        }
    }
    None
}

/// Argument list for `ip` that pins one host to the physical link.
///
/// A link-scoped default (no gateway, e.g. `ppp0`) must be expressed as
/// `dev <iface>` with no `via`, or `ip` rejects it.
pub fn host_route_args(ip: &Ipv4Addr, route: &PhysicalRoute) -> Vec<String> {
    let mut args = vec![
        "-4".to_string(),
        "route".to_string(),
        "replace".to_string(),
        format!("{}/32", ip),
    ];
    if let Some(gw) = route.gateway {
        args.push("via".to_string());
        args.push(gw.to_string());
    }
    args.push("dev".to_string());
    args.push(route.iface.clone());
    args
}

/// Which DNS mechanism to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsBackend {
    /// systemd-resolved: per-link servers, reverted with the link.
    Resolved,
    /// Everything else: rewrite `/etc/resolv.conf`, restore it afterwards.
    ResolvConf,
}

/// `resolvectl` present *and* resolved's runtime directory present. Checking
/// only for the binary is not enough: it ships in systemd packages even on
/// systems where resolved is masked.
pub fn dns_backend(resolvectl_present: bool, resolved_runtime_dir: bool) -> DnsBackend {
    if resolvectl_present && resolved_runtime_dir {
        DnsBackend::Resolved
    } else {
        DnsBackend::ResolvConf
    }
}

/// Marker line that tells our own `/etc/resolv.conf` apart from the distro's.
pub const RESOLV_MARKER: &str = "# ProxysVPN: tunnel resolvers, original backed up by the helper";

/// Render the `/etc/resolv.conf` we install while the tunnel is up.
pub fn render_resolv_conf(servers: &[IpAddr]) -> String {
    let mut out = String::from(RESOLV_MARKER);
    out.push('\n');
    for ip in servers {
        out.push_str("nameserver ");
        out.push_str(&ip.to_string());
        out.push('\n');
    }
    out
}

/// What we found at `/etc/resolv.conf` before replacing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvBackup {
    /// It was a symlink to this target (systemd-resolved stub, resolvconf, …).
    Symlink(String),
    /// It was a regular file, copied to `<run_dir>/<name>`.
    File(String),
    /// It did not exist at all.
    Absent,
}

const BACKUP_COPY_NAME: &str = "resolv.conf.orig";

pub fn render_backup_marker(backup: &ResolvBackup) -> String {
    match backup {
        ResolvBackup::Symlink(target) => format!("symlink {}\n", target),
        ResolvBackup::File(name) => format!("file {}\n", name),
        ResolvBackup::Absent => "absent\n".to_string(),
    }
}

pub fn parse_backup_marker(text: &str) -> Result<ResolvBackup, String> {
    let line = text.lines().next().unwrap_or("").trim();
    if line == "absent" {
        return Ok(ResolvBackup::Absent);
    }
    if let Some(rest) = line.strip_prefix("symlink ") {
        let target = rest.trim();
        if target.is_empty() {
            return Err("empty symlink target in backup marker".to_string());
        }
        return Ok(ResolvBackup::Symlink(target.to_string()));
    }
    if let Some(rest) = line.strip_prefix("file ") {
        let name = rest.trim();
        if name.is_empty() || name.contains('/') {
            return Err(format!("unsafe backup file name: {:?}", name));
        }
        return Ok(ResolvBackup::File(name.to_string()));
    }
    Err(format!("unrecognised backup marker: {:?}", line))
}

#[cfg(unix)]
mod files {
    use super::*;
    use std::fs;
    use std::io::{self, ErrorKind};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    pub fn marker_path(run_dir: &Path) -> PathBuf {
        run_dir.join("resolv-backup")
    }

    /// Replace `resolv` with our own resolver list, remembering what was there.
    ///
    /// Idempotent: if a marker already exists we do **not** overwrite it, so a
    /// second `up` (or an `ensure` tick) cannot lose the distro's original.
    pub fn resolv_apply(run_dir: &Path, resolv: &Path, servers: &[IpAddr]) -> io::Result<()> {
        fs::create_dir_all(run_dir)?;
        fs::set_permissions(run_dir, fs::Permissions::from_mode(0o700))?;

        let marker = marker_path(run_dir);
        if !marker.exists() {
            let backup = match fs::symlink_metadata(resolv) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let target = fs::read_link(resolv)?;
                    ResolvBackup::Symlink(target.to_string_lossy().to_string())
                }
                Ok(_) => {
                    fs::copy(resolv, run_dir.join(BACKUP_COPY_NAME))?;
                    ResolvBackup::File(BACKUP_COPY_NAME.to_string())
                }
                Err(e) if e.kind() == ErrorKind::NotFound => ResolvBackup::Absent,
                Err(e) => return Err(e),
            };
            fs::write(&marker, render_backup_marker(&backup))?;
        }

        // Unlink first: writing through a symlink would edit the *target*
        // (/run/systemd/resolve/stub-resolv.conf), which resolved owns.
        match fs::remove_file(resolv) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        fs::write(resolv, render_resolv_conf(servers))?;
        fs::set_permissions(resolv, fs::Permissions::from_mode(0o644))?;
        Ok(())
    }

    /// Put back whatever `resolv_apply` found. Returns `false` when there was
    /// nothing to restore (no marker — so we never touched the file).
    pub fn resolv_restore(run_dir: &Path, resolv: &Path) -> io::Result<bool> {
        let marker = marker_path(run_dir);
        let text = match fs::read_to_string(&marker) {
            Ok(t) => t,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let backup = parse_backup_marker(&text)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;

        match fs::remove_file(resolv) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match &backup {
            ResolvBackup::Symlink(target) => {
                std::os::unix::fs::symlink(target, resolv)?;
            }
            ResolvBackup::File(name) => {
                let copy = run_dir.join(name);
                fs::copy(&copy, resolv)?;
                fs::set_permissions(resolv, fs::Permissions::from_mode(0o644))?;
                let _ = fs::remove_file(&copy);
            }
            ResolvBackup::Absent => {}
        }
        fs::remove_file(&marker)?;
        Ok(true)
    }

    /// Did a previous run leave a backup behind (i.e. did we crash)?
    pub fn resolv_backup_exists(run_dir: &Path) -> bool {
        marker_path(run_dir).exists()
    }
}

#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(unused_imports))]
pub use files::{resolv_apply, resolv_backup_exists, resolv_restore};

#[cfg(test)]
mod tests {
    use super::*;

    // A real `cat /proc/net/route` from an Ubuntu 22.04 box with one wired
    // link, plus the two half-defaults our tunnel installs.
    const SAMPLE: &str = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
enp0s3\t00000000\t0202000A\t0003\t0\t0\t100\t00000000\t0\t0\t0
enp0s3\t0002000A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
proxysvpn0\t00000000\t00000000\t0001\t0\t0\t0\t00000080\t0\t0\t0
proxysvpn0\t00000080\t00000000\t0001\t0\t0\t0\t00000080\t0\t0\t0
proxysvpn0\t000012C6\t00000000\t0001\t0\t0\t0\t0000FEFF\t0\t0\t0
";

    #[test]
    fn hex_fields_decode_little_endian() {
        assert_eq!(hex_le_to_ipv4("0202000A"), Some(Ipv4Addr::new(10, 0, 2, 2)));
        assert_eq!(hex_le_to_ipv4("00000000"), Some(Ipv4Addr::new(0, 0, 0, 0)));
        assert_eq!(hex_le_to_ipv4("00000080"), Some(Ipv4Addr::new(128, 0, 0, 0)));
        assert_eq!(hex_le_to_ipv4("00FFFFFF"), Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(hex_le_to_ipv4("0101A8C0"), Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn hex_fields_reject_junk() {
        assert_eq!(hex_le_to_ipv4(""), None);
        assert_eq!(hex_le_to_ipv4("0202000"), None);
        assert_eq!(hex_le_to_ipv4("0202000AA"), None);
        assert_eq!(hex_le_to_ipv4("ZZZZZZZZ"), None);
        assert_eq!(hex_le_to_ipv4("Destination"), None);
    }

    #[test]
    fn physical_default_skips_our_own_half_routes() {
        let got = parse_physical_default(SAMPLE).expect("a default route");
        assert_eq!(got.iface, "enp0s3");
        assert_eq!(got.gateway, Some(Ipv4Addr::new(10, 0, 2, 2)));
        assert_eq!(got.metric, 100);
    }

    #[test]
    fn physical_default_prefers_lowest_metric() {
        let text = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlp3s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0
enp0s3\t00000000\t0202000A\t0003\t0\t0\t100\t00000000\t0\t0\t0
";
        let got = parse_physical_default(text).expect("a default route");
        assert_eq!(got.iface, "enp0s3");
        assert_eq!(got.metric, 100);
    }

    #[test]
    fn physical_default_handles_link_scoped_default() {
        let text = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
ppp0\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0
";
        let got = parse_physical_default(text).expect("ppp0 is a real exit");
        assert_eq!(got.iface, "ppp0");
        assert_eq!(got.gateway, None);
    }

    #[test]
    fn physical_default_ignores_down_routes_and_other_vpns() {
        let text = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
enp0s3\t00000000\t0202000A\t0002\t0\t0\t100\t00000000\t0\t0\t0
wg0\t00000000\t0101A8C0\t0003\t0\t0\t50\t00000000\t0\t0\t0
";
        assert_eq!(parse_physical_default(text), None);
    }

    #[test]
    fn physical_default_on_empty_input() {
        assert_eq!(parse_physical_default(""), None);
        assert_eq!(parse_physical_default("Iface\tDestination\n"), None);
        assert_eq!(parse_physical_default("garbage\nlines\n"), None);
    }

    #[test]
    fn split_defaults_detected_for_our_device_only() {
        assert_eq!(parse_split_defaults(SAMPLE, "proxysvpn0"), (true, true));
        assert_eq!(parse_split_defaults(SAMPLE, "enp0s3"), (false, false));
    }

    #[test]
    fn split_defaults_report_half_installed_state() {
        let text = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
proxysvpn0\t00000000\t00000000\t0001\t0\t0\t0\t00000080\t0\t0\t0
";
        assert_eq!(parse_split_defaults(text, "proxysvpn0"), (true, false));
    }

    #[test]
    fn route_get_dev_is_extracted() {
        let via = "203.0.113.7 via 10.0.2.2 dev enp0s3 src 10.0.2.15 uid 1000 \n    cache \n";
        assert_eq!(parse_ip_route_get_dev(via).as_deref(), Some("enp0s3"));
        let leaked = "203.0.113.7 dev proxysvpn0 src 198.18.0.1 uid 0 \n    cache \n";
        assert_eq!(parse_ip_route_get_dev(leaked).as_deref(), Some("proxysvpn0"));
        assert_eq!(parse_ip_route_get_dev(""), None);
        assert_eq!(parse_ip_route_get_dev("RTNETLINK answers: Network is unreachable"), None);
        assert_eq!(parse_ip_route_get_dev("1.2.3.4 dev"), None);
    }

    #[test]
    fn tunnel_ifaces_recognised() {
        for name in ["proxysvpn0", "tun0", "tap3", "wg0", "utun4", "tailscale0"] {
            assert!(is_tunnel_iface(name), "{name}");
        }
        for name in ["enp0s3", "eth0", "wlan0", "wlp3s0", "ppp0", "lo", "enx001122"] {
            assert!(!is_tunnel_iface(name), "{name}");
        }
    }

    #[test]
    fn host_route_args_use_via_when_a_gateway_exists() {
        let route = PhysicalRoute {
            iface: "enp0s3".to_string(),
            gateway: Some(Ipv4Addr::new(10, 0, 2, 2)),
            metric: 100,
        };
        assert_eq!(
            host_route_args(&Ipv4Addr::new(203, 0, 113, 7), &route),
            vec!["-4", "route", "replace", "203.0.113.7/32", "via", "10.0.2.2", "dev", "enp0s3"]
        );
    }

    #[test]
    fn host_route_args_fall_back_to_the_device_for_link_scoped_defaults() {
        let route = PhysicalRoute {
            iface: "ppp0".to_string(),
            gateway: None,
            metric: 0,
        };
        assert_eq!(
            host_route_args(&Ipv4Addr::new(203, 0, 113, 7), &route),
            vec!["-4", "route", "replace", "203.0.113.7/32", "dev", "ppp0"]
        );
    }

    #[test]
    fn dns_backend_requires_both_signals() {
        assert_eq!(dns_backend(true, true), DnsBackend::Resolved);
        assert_eq!(dns_backend(true, false), DnsBackend::ResolvConf);
        assert_eq!(dns_backend(false, true), DnsBackend::ResolvConf);
        assert_eq!(dns_backend(false, false), DnsBackend::ResolvConf);
    }

    #[test]
    fn resolv_conf_is_rendered_with_marker() {
        let out = render_resolv_conf(&["1.1.1.1".parse().unwrap(), "2606:4700:4700::1111".parse().unwrap()]);
        assert!(out.starts_with(RESOLV_MARKER));
        assert!(out.contains("\nnameserver 1.1.1.1\n"));
        assert!(out.contains("\nnameserver 2606:4700:4700::1111\n"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn backup_markers_round_trip() {
        let cases = [
            ResolvBackup::Symlink("/run/systemd/resolve/stub-resolv.conf".to_string()),
            ResolvBackup::File("resolv.conf.orig".to_string()),
            ResolvBackup::Absent,
        ];
        for case in cases {
            let text = render_backup_marker(&case);
            assert_eq!(parse_backup_marker(&text).expect("parse"), case);
        }
    }

    #[test]
    fn backup_markers_reject_junk_and_path_traversal() {
        assert!(parse_backup_marker("").is_err());
        assert!(parse_backup_marker("nonsense\n").is_err());
        assert!(parse_backup_marker("symlink \n").is_err());
        assert!(parse_backup_marker("file ../../etc/shadow\n").is_err());
        assert!(parse_backup_marker("file \n").is_err());
    }

    #[cfg(unix)]
    mod unix_files {
        use super::*;
        use std::fs;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU32, Ordering};

        static SEQ: AtomicU32 = AtomicU32::new(0);

        struct Scratch(PathBuf);

        impl Scratch {
            fn new(tag: &str) -> Self {
                let dir = std::env::temp_dir().join(format!(
                    "proxysvpn-{}-{}-{}",
                    tag,
                    std::process::id(),
                    SEQ.fetch_add(1, Ordering::SeqCst)
                ));
                let _ = fs::remove_dir_all(&dir);
                fs::create_dir_all(&dir).expect("scratch dir");
                Scratch(dir)
            }
            fn path(&self, name: &str) -> PathBuf {
                self.0.join(name)
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn servers() -> Vec<IpAddr> {
            vec!["1.1.1.1".parse().expect("ip")]
        }

        #[test]
        fn plain_file_is_backed_up_and_restored() {
            let s = Scratch::new("resolv-file");
            let run = s.path("run");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("seed");

            resolv_apply(&run, &resolv, &servers()).expect("apply");
            let now = fs::read_to_string(&resolv).expect("read");
            assert!(now.contains("nameserver 1.1.1.1"));
            assert!(resolv_backup_exists(&run));

            assert!(resolv_restore(&run, &resolv).expect("restore"));
            assert_eq!(
                fs::read_to_string(&resolv).expect("read"),
                "nameserver 192.168.1.1\n"
            );
            assert!(!resolv_backup_exists(&run));
        }

        #[test]
        fn symlink_target_survives_and_is_not_overwritten() {
            let s = Scratch::new("resolv-link");
            let run = s.path("run");
            let resolv = s.path("resolv.conf");
            let stub = s.path("stub-resolv.conf");
            fs::write(&stub, "nameserver 127.0.0.53\n").expect("seed");
            std::os::unix::fs::symlink(&stub, &resolv).expect("symlink");

            resolv_apply(&run, &resolv, &servers()).expect("apply");
            assert!(
                fs::symlink_metadata(&resolv).expect("meta").file_type().is_file(),
                "the symlink must be replaced, not followed"
            );
            assert_eq!(
                fs::read_to_string(&stub).expect("read stub"),
                "nameserver 127.0.0.53\n",
                "resolved's own stub file must stay untouched"
            );

            assert!(resolv_restore(&run, &resolv).expect("restore"));
            let meta = fs::symlink_metadata(&resolv).expect("meta");
            assert!(meta.file_type().is_symlink());
            assert_eq!(fs::read_link(&resolv).expect("link"), stub);
        }

        #[test]
        fn missing_resolv_conf_is_recreated_as_missing() {
            let s = Scratch::new("resolv-absent");
            let run = s.path("run");
            let resolv = s.path("resolv.conf");

            resolv_apply(&run, &resolv, &servers()).expect("apply");
            assert!(resolv.exists());
            assert!(resolv_restore(&run, &resolv).expect("restore"));
            assert!(!resolv.exists(), "we must not leave a file we invented");
        }

        #[test]
        fn second_apply_keeps_the_first_backup() {
            let s = Scratch::new("resolv-twice");
            let run = s.path("run");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 10.0.0.1\n").expect("seed");

            resolv_apply(&run, &resolv, &servers()).expect("apply 1");
            resolv_apply(&run, &resolv, &servers()).expect("apply 2");
            assert!(resolv_restore(&run, &resolv).expect("restore"));
            assert_eq!(
                fs::read_to_string(&resolv).expect("read"),
                "nameserver 10.0.0.1\n",
                "the original must survive a double apply"
            );
        }

        #[test]
        fn restore_without_backup_is_a_noop() {
            let s = Scratch::new("resolv-noop");
            let run = s.path("run");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 10.0.0.1\n").expect("seed");
            assert!(!resolv_restore(&run, &resolv).expect("restore"));
            assert_eq!(
                fs::read_to_string(&resolv).expect("read"),
                "nameserver 10.0.0.1\n"
            );
        }

        #[test]
        fn run_dir_is_private() {
            use std::os::unix::fs::PermissionsExt;
            let s = Scratch::new("resolv-mode");
            let run = s.path("run");
            let resolv = s.path("resolv.conf");
            resolv_apply(&run, &resolv, &servers()).expect("apply");
            let mode = fs::metadata(&run).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "helper state must not be world readable");
        }
    }
}
