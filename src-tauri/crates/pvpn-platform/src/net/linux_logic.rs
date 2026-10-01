// src-tauri/crates/pvpn-platform/src/net/linux_logic.rs
//! Host-independent half of the Linux tunnel backend: table parsers, resolver
//! file rendering and the `/etc/resolv.conf` backup dance.
//!
//! Compiled on **every** host, not only Linux, so that `cargo test` on the
//! developer's Mac — the only machine available for this port — exercises the
//! parts that are easy to get wrong. The command-running half lives in
//! `net/linux.rs` and only builds on Linux; the argv it runs comes from
//! `net::plan::linux`.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
// Reason for the allow: on macOS and Windows nothing calls these, but the tests
// must run — this Mac is the only machine available for the Linux port.

use std::net::{IpAddr, Ipv4Addr};

/// The physical default route we must pin the node's address to, so that the
/// tunnel's own packets do not re-enter the tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxDefaultRoute {
    pub iface: String,
    /// `None` for a link-scoped default (`default dev ppp0` with no gateway).
    pub gateway: Option<Ipv4Addr>,
    pub metric: u32,
}

impl From<&LinuxDefaultRoute> for crate::net::PhysicalRoute {
    /// The contract's route type, for logging and for the uniform
    /// "original gateway: …" line every platform prints.
    fn from(r: &LinuxDefaultRoute) -> Self {
        crate::net::PhysicalRoute {
            next_hop: r.gateway.map(|gw| gw.to_string()),
            if_name: Some(r.iface.clone()),
            if_index: None,
        }
    }
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
pub fn parse_physical_default(text: &str) -> Option<LinuxDefaultRoute> {
    let mut best: Option<LinuxDefaultRoute> = None;
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
        let candidate = LinuxDefaultRoute {
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

/// Is there an IPv6 default route on a physical interface in
/// `/proc/net/ipv6_route`?
///
/// Columns: destination (32 hex digits), prefix length (hex), source, source
/// prefix, next hop, metric, refcount, use, flags, device. `::/0` on anything
/// but loopback or a tunnel means the machine reaches the internet over IPv6.
/// The kernel lists unreachable defaults on `lo`, which is why that one is
/// skipped explicitly.
pub fn parse_ipv6_physical_default(text: &str) -> bool {
    text.lines().any(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 {
            return false;
        }
        let (dest, prefix, iface) = (cols[0], cols[1], cols[9]);
        dest.len() == 32
            && dest.bytes().all(|b| b == b'0')
            && prefix == "00"
            && iface != "lo"
            && !is_tunnel_iface(iface)
    })
}

/// TCP state LISTEN in `/proc/net/tcp{,6}`.
const TCP_LISTEN: &str = "0A";

/// Decode a `/proc/net/tcp6` address: 32 hex digits, four 32-bit words, each
/// in host byte order (little-endian on every platform we ship).
fn hex_le_to_ipv6(field: &str) -> Option<std::net::Ipv6Addr> {
    if field.len() != 32 || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut octets = [0u8; 16];
    let (words, _) = octets.as_chunks_mut::<4>();
    for (word, chunk) in words.iter_mut().enumerate() {
        let hex = field.get(word * 8..word * 8 + 8)?;
        let raw = u32::from_str_radix(hex, 16).ok()?;
        *chunk = raw.to_le_bytes();
    }
    Some(std::net::Ipv6Addr::from(octets))
}

/// The owners (uids) of every listening TCP socket that a connection to
/// 127.0.0.1:`port` could reach, read from `/proc/net/tcp` and
/// `/proc/net/tcp6`.
///
/// This is what tun2socks connects to, and through it every packet of the
/// machine once the split defaults are in: the root helper asks it before it
/// points the machine at the port and on every supervisor tick, so a program
/// of another user that takes the port while xray restarts gets nothing.
///
/// Columns: slot, local address:port (hex), remote address:port, state, queues,
/// timer, retransmits, uid, … Unparsable lines are skipped.
pub fn loopback_listener_uids(tcp: &str, tcp6: &str, port: u16) -> Vec<u32> {
    let mut owners = Vec::new();
    for (text, v6) in [(tcp, false), (tcp6, true)] {
        for line in text.lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 8 || cols[3] != TCP_LISTEN {
                continue;
            }
            let Some((addr_hex, port_hex)) = cols[1].split_once(':') else {
                continue;
            };
            if u16::from_str_radix(port_hex, 16).ok() != Some(port) {
                continue;
            }
            let addr = if v6 {
                hex_le_to_ipv6(addr_hex).map(IpAddr::V6)
            } else {
                hex_le_to_ipv4(addr_hex).map(IpAddr::V4)
            };
            let Some(addr) = addr else { continue };
            if !crate::net::takes_loopback_v4(addr) {
                continue;
            }
            if let Ok(uid) = cols[7].parse::<u32>() {
                owners.push(uid);
            }
        }
    }
    owners
}

/// The first listener owner that is neither root nor the person who started
/// the helper (`PKEXEC_UID`; `None` when a root GUI started it directly, and
/// then its engines are root's too).
pub fn foreign_listener(owners: &[u32], invoking_uid: Option<u32>) -> Option<u32> {
    owners
        .iter()
        .copied()
        .find(|&uid| uid != 0 && Some(uid) != invoking_uid)
}

/// May `name` be used as one path component under `/sys/class/net`?
///
/// The name comes from our own constants today; the check keeps it that way —
/// a slash or a dot-dot would turn a counter read into a read of somewhere else.
pub fn is_plain_iface_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 16
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// One `/sys/class/net/<iface>/statistics/*_bytes` value.
pub fn parse_counter(text: &str) -> Option<u64> {
    text.trim().parse().ok()
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

/// What `resolv_restore` did with a backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvRestore {
    /// No marker: we never touched the file.
    NoBackup,
    /// Our file (or nothing at all) was in place, and the original is back.
    Restored,
    /// Something else — NetworkManager, resolvconf, resolved — rewrote the
    /// file after us. Theirs describes the network the machine is on now and
    /// is newer than any backup, so it was kept and the backup dropped.
    KeptNewer,
}

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

    pub fn marker_path(backup_dir: &Path) -> PathBuf {
        backup_dir.join("resolv-backup")
    }

    /// Is this file one we wrote ourselves?
    fn file_is_ours(resolv: &Path) -> bool {
        match fs::read_to_string(resolv) {
            Ok(text) => text.contains(RESOLV_MARKER),
            Err(_) => false,
        }
    }

    /// Replace `resolv` with our own resolver list, remembering what was there.
    ///
    /// `backup_dir` must be a directory that survives a reboot — see
    /// `paths::linux_persistent_dir` for why that is not the same place as the
    /// route hint.
    ///
    /// Idempotent: if a marker already exists we do **not** overwrite it, so a
    /// second `up` (or an `ensure` tick) cannot lose the distro's original.
    pub fn resolv_apply(backup_dir: &Path, resolv: &Path, servers: &[IpAddr]) -> io::Result<()> {
        fs::create_dir_all(backup_dir)?;
        fs::set_permissions(backup_dir, fs::Permissions::from_mode(0o700))?;

        let marker = marker_path(backup_dir);
        if !marker.exists() {
            let backup = match fs::symlink_metadata(resolv) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let target = fs::read_link(resolv)?;
                    ResolvBackup::Symlink(target.to_string_lossy().to_string())
                }
                // Our own file, with no marker beside it: a previous run was
                // killed in a way that lost the backup (a power cut with the
                // backup in a tmpfs used to do exactly that). Recording it as
                // "the original" would make the loss permanent and silent, so
                // record `Absent` instead — on restore we remove our file and
                // let NetworkManager, resolvconf or resolved regenerate one.
                Ok(_) if file_is_ours(resolv) => ResolvBackup::Absent,
                Ok(_) => {
                    fs::copy(resolv, backup_dir.join(BACKUP_COPY_NAME))?;
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

    /// Is `resolv` still the regular file `resolv_apply` wrote? A symlink is
    /// never ours (we always write a plain file), whatever it points at.
    fn regular_file_is_ours(resolv: &Path) -> bool {
        match fs::symlink_metadata(resolv) {
            Ok(meta) if meta.file_type().is_file() => file_is_ours(resolv),
            _ => false,
        }
    }

    /// Drop the marker and the copy without touching `resolv`.
    fn forget_backup(backup_dir: &Path) -> io::Result<()> {
        let _ = fs::remove_file(backup_dir.join(BACKUP_COPY_NAME));
        match fs::remove_file(marker_path(backup_dir)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Put back whatever `resolv_apply` found — but only over our own file.
    ///
    /// The backup is the resolver of the network the machine was on at
    /// connect time. When NetworkManager (or resolvconf, or resolved) has
    /// written the file since — after a reboot that followed a power cut, on
    /// a new network, or inside the few seconds before the supervisor's next
    /// tick — theirs is the one that works here and now, and putting the old
    /// copy over it left every lookup on the machine failing until they
    /// happened to rewrite it again. So, like the .deb's postrm: our file (or
    /// no file at all) is replaced by the original; anything else is kept and
    /// only the stale backup goes.
    pub fn resolv_restore(backup_dir: &Path, resolv: &Path) -> io::Result<ResolvRestore> {
        let run_dir = backup_dir;
        let marker = marker_path(backup_dir);
        let text = match fs::read_to_string(&marker) {
            Ok(t) => t,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(ResolvRestore::NoBackup),
            Err(e) => return Err(e),
        };

        let present = match fs::symlink_metadata(resolv) {
            Ok(_) => true,
            Err(e) if e.kind() == ErrorKind::NotFound => false,
            Err(e) => return Err(e),
        };
        if present && !regular_file_is_ours(resolv) {
            forget_backup(backup_dir)?;
            return Ok(ResolvRestore::KeptNewer);
        }

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
        Ok(ResolvRestore::Restored)
    }

    /// Did a previous run leave a backup behind (i.e. did we crash)?
    pub fn resolv_backup_exists(backup_dir: &Path) -> bool {
        marker_path(backup_dir).exists()
    }

    /// Put ours back after something else rewrote `resolv` mid-session.
    ///
    /// `resolv_apply` keeps the backup taken at connect time, which is right
    /// for a second `up` but wrong here: NetworkManager rewrites the file when
    /// the network changes (home Wi-Fi to office Wi-Fi after a suspend), and
    /// what it wrote is the resolver of the network the machine is on NOW.
    /// Keeping the connect-time copy meant Disconnect restored the previous
    /// network's resolver, unreachable here, and every lookup on the machine
    /// failed until NetworkManager happened to rewrite the file again. So the
    /// backup is retaken from the file that replaced ours — unless that file
    /// is gone, in which case the old copy is still the best there is.
    pub fn resolv_reapply(backup_dir: &Path, resolv: &Path, servers: &[IpAddr]) -> io::Result<()> {
        let replaced = fs::symlink_metadata(resolv).is_ok() && !file_is_ours(resolv);
        if replaced {
            forget_backup(backup_dir)?;
        }
        resolv_apply(backup_dir, resolv, servers)
    }
}

#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(unused_imports))]
pub use files::{resolv_apply, resolv_backup_exists, resolv_reapply, resolv_restore};

#[cfg(test)]
mod tests {
    use super::*;

    // `/proc/net/tcp` and `/proc/net/tcp6` in the kernel's own layout. Port
    // 10808 is 2A38. Rows: our xray on 127.0.0.1 (uid 1000), a stranger's
    // wildcard listener on another port, an established connection to 10808
    // (not a listener), and in tcp6 a stranger's dual-stack [::]:10808.
    const TCP: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:2A38 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41001 1 0000000000000000 100 0 0 10 0
   1: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1001        0 41002 1 0000000000000000 100 0 0 10 0
   2: 0100007F:2A38 0100007F:D431 01 00000000:00000000 00:00000000 00000000  1002        0 41003 1 0000000000000000 20 4 30 10 -1
";
    const TCP6_STRANGER: &str = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:2A38 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1001        0 52001 1 0000000000000000 100 0 0 10 0
";
    const TCP6_LOOPBACK_V6_ONLY: &str = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:2A38 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1001        0 52002 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn listeners_a_tunnel_connection_could_reach_are_found_with_their_owner() {
        assert_eq!(loopback_listener_uids(TCP, "", 10808), vec![1000]);
        // A dual-stack [::] listener takes 127.0.0.1 connections too.
        assert_eq!(loopback_listener_uids(TCP, TCP6_STRANGER, 10808), vec![1000, 1001]);
        // ::1 is not where tun2socks connects.
        assert_eq!(loopback_listener_uids("", TCP6_LOOPBACK_V6_ONLY, 10808), Vec::<u32>::new());
        // Another port, and an established connection, are not listeners on ours.
        assert_eq!(loopback_listener_uids(TCP, "", 8080), vec![1001]);
        assert_eq!(loopback_listener_uids("", "", 10808), Vec::<u32>::new());
        assert_eq!(loopback_listener_uids("garbage\n0: zz", "", 10808), Vec::<u32>::new());
    }

    /// Only the person who started the helper and root may hold the port the
    /// whole machine's traffic is pointed at. No listener at all is fine: an
    /// engine restarting leaves the port empty for a moment.
    #[test]
    fn a_listener_of_another_user_is_foreign() {
        assert_eq!(foreign_listener(&[1000], Some(1000)), None);
        assert_eq!(foreign_listener(&[0], Some(1000)), None);
        assert_eq!(foreign_listener(&[], Some(1000)), None);
        assert_eq!(foreign_listener(&[1000, 1001], Some(1000)), Some(1001));
        // Started by a root GUI without pkexec: only root's own listener.
        assert_eq!(foreign_listener(&[0], None), None);
        assert_eq!(foreign_listener(&[1000], None), Some(1000));
    }

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

    // `cat /proc/net/ipv6_route` shape: an unreachable default on lo (the
    // kernel always lists one), a real default via the wired link, and a
    // default on another VPN's tunnel.
    const IPV6_SAMPLE: &str = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003   enp0s3
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200       lo
fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001   enp0s3
";

    #[test]
    fn an_ipv6_default_on_a_real_link_counts() {
        assert!(parse_ipv6_physical_default(IPV6_SAMPLE));
    }

    #[test]
    fn ipv6_defaults_on_loopback_or_a_tunnel_do_not_count() {
        let only_lo_and_tunnel = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200       lo
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 00000400 00000001 00000000 00000001      wg0
";
        assert!(!parse_ipv6_physical_default(only_lo_and_tunnel));
        assert!(!parse_ipv6_physical_default(""));
        assert!(!parse_ipv6_physical_default("short line\n"));
    }

    #[test]
    fn only_a_plain_interface_name_reaches_sysfs() {
        assert!(is_plain_iface_name("proxysvpn0"));
        assert!(is_plain_iface_name("enp0s3"));
        assert!(is_plain_iface_name("wlan0.100"));
        assert!(!is_plain_iface_name(""));
        assert!(!is_plain_iface_name(".."));
        assert!(!is_plain_iface_name("."));
        assert!(!is_plain_iface_name("../../etc"));
        assert!(!is_plain_iface_name("eth0/statistics"));
        assert!(!is_plain_iface_name("averyveryverylongname"));
    }

    #[test]
    fn sysfs_counters_parse_with_their_newline() {
        assert_eq!(parse_counter("123456\n"), Some(123_456));
        assert_eq!(parse_counter("0"), Some(0));
        assert_eq!(parse_counter("x\n"), None);
        assert_eq!(parse_counter(""), None);
    }

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

            assert_eq!(resolv_restore(&run, &resolv).expect("restore"), ResolvRestore::Restored);
            assert_eq!(
                fs::read_to_string(&resolv).expect("read"),
                "nameserver 192.168.1.1\n"
            );
            assert!(!resolv_backup_exists(&run));
        }

        /// The case a tmpfs backup directory used to create: the machine comes
        /// back up with our `/etc/resolv.conf` still on disk and the marker
        /// gone. Recording our own file as "the original" would make the loss
        /// permanent and invisible, so it must be recorded as absent instead,
        /// and the restore must clear the way for the distro to regenerate one.
        #[test]
        fn our_own_resolv_conf_is_never_mistaken_for_the_original() {
            let s = Scratch::new("resolv-ours");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, render_resolv_conf(&servers())).expect("seed");

            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            let marker = fs::read_to_string(crate::net::linux_logic::files::marker_path(&backup))
                .expect("marker");
            assert_eq!(
                parse_backup_marker(&marker).expect("parse"),
                ResolvBackup::Absent,
                "our own file must not be remembered as the distro's"
            );
            assert!(
                !backup.join("resolv.conf.orig").exists(),
                "and it must not be copied either"
            );

            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::Restored);
            assert!(
                !resolv.exists(),
                "nothing to put back, so the path is left for the distro to recreate"
            );
        }

        /// The .deb's postrm, run as `apt remove` would run it.
        fn run_postrm(s: &Scratch, action: &str) {
            let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../linux/postrm");
            let status = std::process::Command::new("sh")
                .arg(&script)
                .arg(action)
                .env("PROXYSVPN_STATE_DIR", s.path("backup"))
                .env("PROXYSVPN_RESOLV_CONF", s.path("resolv.conf"))
                .env("PROXYSVPN_RUNTIME_DIR", s.path("run"))
                .status()
                .expect("sh runs");
            assert!(status.success(), "postrm must never fail a removal");
        }

        /// A power cut while connected, then `apt remove` instead of a
        /// relaunch: nothing else would ever put the resolver back.
        #[test]
        fn removing_the_package_puts_back_the_original_resolver() {
            let s = Scratch::new("postrm-file");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("seed");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");

            run_postrm(&s, "remove");
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 192.168.1.1\n");
            assert!(!resolv_backup_exists(&backup), "a leftover backup would be restored over a newer file later");
        }

        #[test]
        fn removing_the_package_restores_a_symlink_and_an_absent_file() {
            let s = Scratch::new("postrm-link");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            let stub = s.path("stub-resolv.conf");
            fs::write(&stub, "nameserver 127.0.0.53\n").expect("stub");
            std::os::unix::fs::symlink(&stub, &resolv).expect("link");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            run_postrm(&s, "remove");
            assert_eq!(fs::read_link(&resolv).expect("a link again"), stub);

            let s = Scratch::new("postrm-absent");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            run_postrm(&s, "purge");
            assert!(!resolv.exists());
            assert!(!backup.exists(), "purge removes the state folder");
        }

        /// NetworkManager already rewrote the file: it is newer than any
        /// backup and is left alone; the stale backup goes.
        #[test]
        fn removing_the_package_leaves_a_rewritten_resolver_alone() {
            let s = Scratch::new("postrm-nm");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("seed");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            fs::write(&resolv, "nameserver 10.0.0.1\n").expect("NM");

            run_postrm(&s, "remove");
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 10.0.0.1\n");
            assert!(!resolv_backup_exists(&backup));
        }

        #[test]
        fn the_postrm_knows_our_marker_line() {
            let script = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../linux/postrm"))
                .expect("postrm");
            assert!(script.contains(RESOLV_MARKER), "the script must recognise the file the helper writes");
        }

        /// Connect at home, NetworkManager rewrites the file on the office
        /// network, the supervisor puts ours back, Disconnect: the machine
        /// must get the office resolver, not the unreachable home one.
        #[test]
        fn a_rewrite_mid_session_becomes_the_new_original() {
            let s = Scratch::new("resolv-moved");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("home");

            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            fs::write(&resolv, "nameserver 10.0.0.1\n").expect("office, written by NM");
            resolv_reapply(&backup, &resolv, &servers()).expect("reapply");
            assert!(fs::read_to_string(&resolv).expect("read").contains("nameserver 1.1.1.1"));

            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::Restored);
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 10.0.0.1\n");
        }

        /// Connect at home, power cut, the machine boots on the office
        /// network and NetworkManager rewrites the file, then the helper's
        /// crash recovery (or a Disconnect inside the supervisor's 5 s tick)
        /// restores: NM's file must stay, and the stale home copy must go so
        /// the next connect backs up the office resolver instead.
        #[test]
        fn a_restore_never_puts_an_old_backup_over_a_newer_file() {
            let s = Scratch::new("resolv-crash-nm");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("home");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            fs::write(&resolv, "nameserver 10.0.0.1\n").expect("office, written by NM at boot");

            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::KeptNewer);
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 10.0.0.1\n");
            assert!(!resolv_backup_exists(&backup), "the stale backup must not survive");
            assert!(!backup.join("resolv.conf.orig").exists());

            // The next connect takes the office file as the original.
            resolv_apply(&backup, &resolv, &servers()).expect("apply again");
            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::Restored);
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 10.0.0.1\n");
        }

        /// resolved or resolvconf put its symlink back after us: keep it,
        /// even when it points at a file that happens to carry our marker.
        #[test]
        fn a_restore_keeps_a_symlink_someone_else_put_back() {
            let s = Scratch::new("resolv-relinked");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            let stub = s.path("stub-resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("home");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");

            fs::write(&stub, render_resolv_conf(&servers())).expect("stub");
            fs::remove_file(&resolv).expect("unlink");
            std::os::unix::fs::symlink(&stub, &resolv).expect("relink");

            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::KeptNewer);
            assert_eq!(fs::read_link(&resolv).expect("still a link"), stub);
            assert!(!resolv_backup_exists(&backup));
        }

        /// Our file was deleted and nothing replaced it yet: the original is
        /// still better than no resolver at all.
        #[test]
        fn a_restore_over_a_missing_file_puts_the_original_back() {
            let s = Scratch::new("resolv-gone");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("home");
            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            fs::remove_file(&resolv).expect("gone");

            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::Restored);
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 192.168.1.1\n");
        }

        /// Reapplying over our own file (nothing rewrote it) keeps the
        /// connect-time original.
        #[test]
        fn a_reapply_over_our_own_file_keeps_the_original() {
            let s = Scratch::new("resolv-same");
            let backup = s.path("backup");
            let resolv = s.path("resolv.conf");
            fs::write(&resolv, "nameserver 192.168.1.1\n").expect("home");

            resolv_apply(&backup, &resolv, &servers()).expect("apply");
            resolv_reapply(&backup, &resolv, &servers()).expect("reapply");
            assert_eq!(resolv_restore(&backup, &resolv).expect("restore"), ResolvRestore::Restored);
            assert_eq!(fs::read_to_string(&resolv).expect("read"), "nameserver 192.168.1.1\n");
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

            assert_eq!(resolv_restore(&run, &resolv).expect("restore"), ResolvRestore::Restored);
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
            assert_eq!(resolv_restore(&run, &resolv).expect("restore"), ResolvRestore::Restored);
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
            assert_eq!(resolv_restore(&run, &resolv).expect("restore"), ResolvRestore::Restored);
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
            assert_eq!(resolv_restore(&run, &resolv).expect("restore"), ResolvRestore::NoBackup);
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
