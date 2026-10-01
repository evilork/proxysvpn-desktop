// src-tauri/crates/pvpn-platform/src/net/windows.rs
// Windows implementation of the net contract.
//
// Division of labour, and why:
//
//   reads  -> IP Helper (the `windows` crate). `route print` and
//             `netsh … show …` are localised: on a Russian Windows the column
//             headers and the "no tasks" sentences are Russian, so parsing
//             them is a bug waiting for the first non-English user. The four
//             read calls used here (ConvertInterfaceAliasToLuid,
//             ConvertInterfaceLuidToIndex, GetBestRoute2, GetIpForwardTable2)
//             return structs, not text.
//   writes -> netsh. We never read its output, only the exit code, so locale
//             does not matter, and it keeps the unsafe surface small. Every
//             write uses `store=active`, so nothing survives a reboot — the
//             cheapest possible crash recovery.
//
// The device itself is created by tun2socks (wireguard-go's Wintun driver),
// exactly as utun225 is on macOS. Wintun publishes the requested name as the
// interface alias, which is how `device_index` finds it again, and the adapter
// disappears together with the process that owns it — so a hard kill of
// tun2socks takes the adapter and all of its routes with it. The only thing
// that can outlive a crash is the /32 host route to the node, which
// `sync_cleanup` removes from the route hint on the next start.
//
// UNVERIFIED on real hardware (no Windows machine on the dev box, see PR):
//   * that `-device ProxysVPN` makes tun2socks create a Wintun adapter whose
//     interface alias is exactly "ProxysVPN";
//   * that wintun.dll next to tun2socks.exe is picked up from the bundle;
//   * the exact netsh argument spelling accepted by every supported build.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToAlias,
    ConvertInterfaceLuidToIndex, FreeMibTable, GetBestRoute2, GetIfEntry2, GetIpForwardTable2,
    GetIpInterfaceEntry, MIB_IF_ROW2, MIB_IPFORWARD_ROW2, MIB_IPFORWARD_TABLE2,
    MIB_IPINTERFACE_ROW,
};
use windows::Win32::NetworkManagement::Ndis::{IF_MAX_STRING_SIZE, NET_LUID_LH};
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_INET};

use crate::log;
use crate::net::plan::{self, windows as p, SPLIT_HIGH, SPLIT_LOW};
use crate::net::{IfCounters, PhysicalRoute};

pub use plan::windows::DEVICE;

/// Probe addresses used to confirm that both halves of the default route point
/// into the tunnel: 1.0.0.1 falls inside 0.0.0.0/1, 200.0.0.1 inside
/// 128.0.0.0/1. These are route-table lookups only — no packet is sent.
const PROBE_LOW: Ipv4Addr = Ipv4Addr::new(1, 0, 0, 1);
const PROBE_HIGH: Ipv4Addr = Ipv4Addr::new(200, 0, 0, 1);

// --------------------------------------------------------------------- reads

fn sockaddr_v4(addr: Ipv4Addr) -> SOCKADDR_INET {
    let mut sa = SOCKADDR_INET::default();
    sa.Ipv4.sin_family = AF_INET;
    // S_addr holds the four octets in network order, which is exactly the
    // byte order Ipv4Addr::octets() produces.
    sa.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(addr.octets());
    sa
}

fn next_hop_of(row: &MIB_IPFORWARD_ROW2) -> Option<Ipv4Addr> {
    // SAFETY: the row came from IP Helper with an AF_INET filter, so the IPv4
    // arm of the union is the initialised one.
    let raw = unsafe { row.NextHop.Ipv4.sin_addr.S_un.S_addr };
    let addr = Ipv4Addr::from(raw.to_ne_bytes());
    if addr.is_unspecified() {
        None
    } else {
        Some(addr)
    }
}

/// Interface index of our TUN adapter, or an error while it does not exist.
fn device_index() -> Result<u32> {
    let alias = HSTRING::from(DEVICE);
    let mut luid = NET_LUID_LH::default();
    // SAFETY: both arguments are valid for the duration of the call.
    let err = unsafe { ConvertInterfaceAliasToLuid(&alias, &mut luid) };
    if err != NO_ERROR {
        return Err(anyhow!("adapter {} not found ({:?})", DEVICE, err));
    }
    let mut index = 0u32;
    // SAFETY: `luid` was filled in by the call above.
    let err = unsafe { ConvertInterfaceLuidToIndex(&luid, &mut index) };
    if err != NO_ERROR {
        return Err(anyhow!("no interface index for {} ({:?})", DEVICE, err));
    }
    Ok(index)
}

/// Interface the OS would currently use to reach `dest`.
fn best_route_index(dest: Ipv4Addr) -> Result<u32> {
    let destination = sockaddr_v4(dest);
    let mut row = MIB_IPFORWARD_ROW2::default();
    let mut source = SOCKADDR_INET::default();
    // SAFETY: all pointers are to live locals; the call only writes into
    // `row` and `source`.
    let err = unsafe {
        GetBestRoute2(
            None,
            0,
            None,
            &destination,
            0,
            &mut row,
            &mut source,
        )
    };
    if err != NO_ERROR {
        return Err(anyhow!("GetBestRoute2({}) failed: {:?}", dest, err));
    }
    Ok(row.InterfaceIndex)
}

/// Interface metric of the adapter a route row belongs to.
///
/// `MIB_IPFORWARD_ROW2::Metric` is only the route's own offset, and in practice
/// it is 0 on every adapter. What Windows actually ranks by is
/// `InterfaceMetric + Metric`, so without this a machine with more than one
/// 0.0.0.0/0 row — Ethernet up with the cable out next to Wi-Fi, or a
/// Hyper-V/VMware virtual NIC — had all candidates tie at 0 and the first row
/// encountered won. Pinning the node's /32 to the wrong adapter shows up as
/// "no route to the node" with no clue why.
fn interface_metric(row: &MIB_IPFORWARD_ROW2) -> Option<u32> {
    let mut iface = MIB_IPINTERFACE_ROW {
        Family: AF_INET,
        InterfaceLuid: row.InterfaceLuid,
        InterfaceIndex: row.InterfaceIndex,
        ..Default::default()
    };
    // SAFETY: `iface` is a live, zero-initialised row with the two key fields
    // filled in, which is what GetIpInterfaceEntry requires; it only writes
    // into that struct.
    let err = unsafe { GetIpInterfaceEntry(&mut iface) };
    if err != NO_ERROR {
        return None;
    }
    Some(iface.Metric)
}

/// How Windows ranks one default route: the interface metric plus the route's
/// own offset, saturating so a pathological pair cannot wrap.
fn route_rank(row: &MIB_IPFORWARD_ROW2) -> u32 {
    match interface_metric(row) {
        Some(iface) => iface.saturating_add(row.Metric),
        // The query failed: fall back to the route offset alone rather than
        // dropping an otherwise usable default.
        None => row.Metric,
    }
}

/// The machine's real IPv4 default route, read from the forwarding table.
///
/// Why the table and not `GetBestRoute2`: once our 0.0.0.0/1 and 128.0.0.0/1
/// are installed, a best-path lookup for any address answers "the tunnel". The
/// physical default is still its own 0.0.0.0/0 row, so read that row directly —
/// the same reason macOS asks `route get default` instead of `route get <ip>`.
/// Rows on our own adapter are skipped, and the lowest rank wins — see
/// [`route_rank`] for what "rank" has to mean here.
fn physical_default(exclude_index: Option<u32>) -> Result<PhysicalRoute> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    // SAFETY: `table` is an out-pointer that IP Helper allocates; it is freed
    // with FreeMibTable below on every path that reaches it.
    let err = unsafe { GetIpForwardTable2(AF_INET, &mut table) };
    if err != NO_ERROR {
        return Err(anyhow!("GetIpForwardTable2 failed: {:?}", err));
    }

    let mut best: Option<(PhysicalRoute, u32)> = None;
    // SAFETY: the table is non-null (NO_ERROR) and NumEntries describes the
    // length of the trailing Table array; nothing escapes the loop by pointer.
    unsafe {
        let entries = (*table).NumEntries as usize;
        let rows = (*table).Table.as_ptr();
        for i in 0..entries {
            let row = &*rows.add(i);
            if row.DestinationPrefix.PrefixLength != 0 {
                continue;
            }
            if row.DestinationPrefix.Prefix.si_family != AF_INET {
                continue;
            }
            if Some(row.InterfaceIndex) == exclude_index {
                continue;
            }
            let candidate = PhysicalRoute {
                next_hop: next_hop_of(row).map(|ip| ip.to_string()),
                if_name: None,
                if_index: Some(row.InterfaceIndex),
            };
            let rank = route_rank(row);
            if best.as_ref().map(|(_, m)| rank < *m).unwrap_or(true) {
                best = Some((candidate, rank));
            }
        }
        FreeMibTable(table as *const _);
    }

    best.map(|(route, _)| route).ok_or_else(|| {
        anyhow!("нет сетевого подключения — не найден шлюз по умолчанию")
    })
}

/// The interface alias ("Ethernet", "Wi-Fi") of an interface index.
///
/// The alias is the name Go's `net.InterfaceByName` matches on Windows, which
/// is how xray's `sockopt.interface` finds the adapter its sockets must leave
/// through. `None` when the index has gone away in the meantime.
fn interface_alias(index: u32) -> Option<String> {
    let mut luid = NET_LUID_LH::default();
    // SAFETY: `luid` is a live out-parameter for the duration of the call.
    if unsafe { ConvertInterfaceIndexToLuid(index, &mut luid) } != NO_ERROR {
        return None;
    }
    // IF_MAX_STRING_SIZE characters plus the terminating NUL.
    let mut buf = [0u16; IF_MAX_STRING_SIZE as usize + 1];
    // SAFETY: `luid` was filled in above; the call writes at most `buf.len()`
    // UTF-16 units into `buf`, including the terminator.
    if unsafe { ConvertInterfaceLuidToAlias(&luid, &mut buf) } != NO_ERROR {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let alias = String::from_utf16(&buf[..len]).ok()?;
    if alias.is_empty() {
        None
    } else {
        Some(alias)
    }
}

// ----------------------------------------------------------------- contract

/// The physical default, with the adapter's alias filled in so the GUI can
/// bind the engine's sockets to it (xray `sockopt.interface`).
pub async fn physical_route() -> Result<PhysicalRoute> {
    let ours = device_index().ok();
    let mut route = physical_default(ours)?;
    route.if_name = route.if_index.and_then(interface_alias);
    Ok(route)
}

/// Byte counters of the tunnel adapter, `None` while it does not exist.
pub fn device_counters() -> Option<IfCounters> {
    let alias = HSTRING::from(DEVICE);
    let mut row = MIB_IF_ROW2::default();
    // SAFETY: both arguments are valid for the duration of the call; the
    // second one is the LUID field of a live, zero-initialised row.
    if unsafe { ConvertInterfaceAliasToLuid(&alias, &mut row.InterfaceLuid) } != NO_ERROR {
        return None;
    }
    // SAFETY: `row` is a live MIB_IF_ROW2 whose key (InterfaceLuid) is set,
    // which is what GetIfEntry2 requires; it only writes into that struct.
    if unsafe { GetIfEntry2(&mut row) } != NO_ERROR {
        return None;
    }
    Some(IfCounters {
        rx_bytes: row.InOctets,
        tx_bytes: row.OutOctets,
    })
}

/// Does the machine have a way out at all? An IPv4 default route on an adapter
/// that is not ours is the same answer Windows' own "no internet" check starts
/// from, and it needs no packet.
pub fn has_usable_link() -> bool {
    physical_default(device_index().ok()).is_ok()
}

/// Is there a /32 route to `dest` in the table, on any adapter?
///
/// Asked before the pre-install delete below so that the delete runs only
/// when there is something to delete. Run blind, it failed on every connect
/// and location change — there normally is no leftover — and each failure
/// put a netsh command line naming the node into the log.
fn host_route_present(dest: Ipv4Addr) -> bool {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    // SAFETY: `table` is an out-pointer that IP Helper allocates; it is freed
    // with FreeMibTable below on every path that reaches it.
    let err = unsafe { GetIpForwardTable2(AF_INET, &mut table) };
    if err != NO_ERROR {
        // Unknown: let the caller delete as before rather than add over a
        // leftover and fail.
        return true;
    }
    let wanted = u32::from_ne_bytes(dest.octets());
    let mut found = false;
    // SAFETY: the table is non-null (NO_ERROR) and NumEntries describes the
    // length of the trailing Table array; nothing escapes the loop by pointer.
    unsafe {
        let entries = (*table).NumEntries as usize;
        let rows = (*table).Table.as_ptr();
        for i in 0..entries {
            let row = &*rows.add(i);
            if row.DestinationPrefix.PrefixLength == 32
                && row.DestinationPrefix.Prefix.si_family == AF_INET
                && row.DestinationPrefix.Prefix.Ipv4.sin_addr.S_un.S_addr == wanted
            {
                found = true;
                break;
            }
        }
        FreeMibTable(table as *const _);
    }
    found
}

pub async fn add_host_route(dest: Ipv4Addr, via: &PhysicalRoute) -> Result<()> {
    // Drop a leftover from an earlier run before installing ours, otherwise
    // netsh refuses the duplicate prefix. Only when there is one: see
    // `host_route_present`.
    if host_route_present(dest) {
        p::host_route_delete(dest).run_best_effort().await;
    }
    p::host_route_add(dest, via)?
        .run()
        .await
        .context("add host route for VPN server")
}

pub async fn delete_host_route(dest: Ipv4Addr) {
    p::host_route_delete(dest).run_best_effort().await;
}

pub async fn host_route_ok(dest: Ipv4Addr) -> bool {
    let ours = match device_index() {
        Ok(idx) => idx,
        // No adapter means no tunnel to loop through.
        Err(_) => return true,
    };
    match best_route_index(dest) {
        Ok(idx) => idx != ours,
        Err(_) => false,
    }
}

pub async fn add_split_defaults() -> Result<()> {
    p::split_default_add(SPLIT_LOW)
        .run()
        .await
        .context("add route 0.0.0.0/1")?;
    p::split_default_add(SPLIT_HIGH)
        .run()
        .await
        .context("add route 128.0.0.0/1")?;
    Ok(())
}

pub async fn delete_split_defaults() {
    p::split_default_delete(SPLIT_LOW).run_best_effort().await;
    p::split_default_delete(SPLIT_HIGH).run_best_effort().await;
}

pub async fn split_defaults_ok() -> bool {
    let ours = match device_index() {
        Ok(idx) => idx,
        Err(_) => return false,
    };
    let low = best_route_index(PROBE_LOW).map(|i| i == ours).unwrap_or(false);
    let high = best_route_index(PROBE_HIGH).map(|i| i == ours).unwrap_or(false);
    low && high
}

pub async fn wait_for_device(timeout: Duration) -> Result<()> {
    let step = Duration::from_millis(100);
    let attempts = (timeout.as_millis() / step.as_millis().max(1)).max(1) as u32;
    for _ in 0..attempts {
        tokio::time::sleep(step).await;
        if device_index().is_ok() {
            return Ok(());
        }
    }
    Err(anyhow!(
        "{} did not come up within {}s — проверьте, что wintun.dll лежит рядом с tun2socks.exe",
        DEVICE,
        timeout.as_secs()
    ))
}

pub async fn configure_device() -> Result<()> {
    p::device_set_address()
        .run()
        .await
        .with_context(|| format!("assign IP to {}", DEVICE))?;
    // Not fatal: the split routes are more specific than any physical default,
    // so a failed metric change degrades predictability, not connectivity.
    if let Err(e) = p::device_set_metric().run().await {
        log::warn("net", &format!("could not set interface metric: {}", e));
    }
    // Must not be skipped silently: with a DNS server on the tunnel adapter
    // Windows would query into the tunnel, where nothing answers.
    p::device_clear_dns()
        .run()
        .await
        .context("clear DNS servers on the tunnel adapter")?;
    Ok(())
}

pub async fn device_down() {
    // Nothing to do: the Wintun adapter belongs to tun2socks and is destroyed
    // with it, taking its addresses and routes along. Kept as a contract step
    // so the teardown order is identical on every platform.
    log::info("net", "device teardown is implicit on Windows (Wintun)");
}

/// Nothing to sweep, and deliberately so.
///
/// `taskkill /IM tun2socks.exe` stopped every process of that name on the
/// machine — another VPN client's included, the same mistake the macOS app
/// made with `pkill -x` until 28.09.2026. Our own engines cannot outlive us
/// here: each one joins a kill-on-close job object at spawn
/// (`process::tie_to_app`), so a crash or an "End task" takes them along, and
/// a normal stop kills them by handle. The step stays in `TEARDOWN_ORDER` so
/// the order is the same on every platform.
pub async fn kill_stray(stem: &str) {
    log::info("net", &format!("no {} sweep on Windows: engines die with the app", stem));
}

/// Synchronous sweep for the exit path, where no runtime is available.
fn sync_cleanup_inner(stale_hosts: &[Ipv4Addr]) {
    use std::process::Command;

    let run = |argv: crate::Argv| {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new(&argv.program);
        cmd.args(&argv.args);
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = cmd.status();
    };

    // Routes first: while an engine is still alive the tunnel keeps working,
    // so there is no window where traffic has nowhere to go.
    run(p::split_default_delete(SPLIT_LOW));
    run(p::split_default_delete(SPLIT_HIGH));
    for host in stale_hosts {
        run(p::host_route_delete(*host));
    }
    // No engine sweep by image name: see `kill_stray`. An engine of ours that
    // existed when the previous run died went with it (job object).
    log::info("net", "cleanup done");
}

// ===========================================================================
// The contract
// ===========================================================================
//
// Windows is a privileged-GUI platform like macOS: the exe carries a manifest
// with requestedExecutionLevel="requireAdministrator" (see build.rs and
// windows-app.manifest), so there is one UAC prompt at launch and the whole
// process tree — including the sidecars — is elevated. The sequence therefore
// runs in-process, and `net::local` holds it (shared with macOS).
//
// The cost is documented in docs/WINDOWS.md: the WebView runs elevated too. An
// unprivileged GUI plus a small elevated service is the next step, and it is
// cheap from here — the contract in net/mod.rs is already the narrow,
// three-operation shape that Linux puts behind IPC, so moving Windows behind
// the same boundary does not touch any caller.

use std::net::IpAddr;
use std::path::Path;

use crate::net::local::{self, Privileged};
use crate::net::TunPlan;
use crate::process::Argv;

struct Windows;

impl Privileged for Windows {
    const DEVICE: &'static str = DEVICE;

    fn tun2socks_argv(bin: &Path, socks_port: u16) -> Argv {
        p::tun2socks(bin, socks_port)
    }

    async fn physical_route() -> Result<PhysicalRoute> {
        physical_route().await
    }

    async fn add_host_route(dest: Ipv4Addr, via: &PhysicalRoute) -> Result<()> {
        add_host_route(dest, via).await
    }

    async fn delete_host_route(dest: Ipv4Addr) {
        delete_host_route(dest).await
    }

    async fn host_route_ok(dest: Ipv4Addr) -> bool {
        host_route_ok(dest).await
    }

    async fn add_split_defaults() -> Result<()> {
        add_split_defaults().await
    }

    async fn delete_split_defaults() {
        delete_split_defaults().await
    }

    async fn split_defaults_ok() -> bool {
        split_defaults_ok().await
    }

    async fn wait_for_device(timeout: Duration) -> Result<()> {
        wait_for_device(timeout).await
    }

    async fn configure_device() -> Result<()> {
        configure_device().await
    }

    async fn device_down() {
        device_down().await
    }

    /// Deliberately nothing — and note that `configure_device` does the
    /// *opposite*: it clears the resolver list on the tunnel adapter.
    ///
    /// Windows queries every adapter that has a resolver configured, in
    /// parallel, and takes the first answer, so publishing a resolver on the
    /// tunnel alone would not keep lookups inside it. With no resolver on the
    /// adapter, lookups go to the network's resolver: through the tunnel when
    /// that resolver is a public address (0.0.0.0/1 and 128.0.0.0/1 cover it),
    /// past it when it sits on the local network, whose route is more
    /// specific. The tunnel is IPv4 only, so IPv6 traffic passes it as well.
    ///
    /// This is NOT what 0.3.1 does on macOS, where sysdns.rs points the
    /// system at `subscription::TUNNEL_DNS` and xray answers it through
    /// dns-out. The window says so on Windows (src/platformCopy.ts) until the
    /// same is done here: TUNNEL_DNS on the adapter plus an NRPT rule for "."
    /// so Windows asks nothing else, and IPv6 carried or blocked.
    async fn configure_dns(_servers: &[IpAddr]) -> Result<()> {
        Ok(())
    }

    async fn restore_dns() {}

    async fn kill_stray(stem: &str) {
        kill_stray(stem).await
    }

}

/// netsh and the Wintun driver both need elevation. The manifest should have
/// got it for us at launch; check rather than fail halfway through.
pub async fn preflight() -> Result<()> {
    if crate::privilege::is_elevated() {
        return Ok(());
    }
    Err(anyhow!("{}", crate::privilege::missing_privileges_message()))
}

pub async fn up(plan: &TunPlan) -> Result<()> {
    local::up::<Windows>(plan).await
}

pub async fn ensure(plan: &TunPlan) -> Result<()> {
    local::ensure::<Windows>(plan).await
}

pub async fn retarget(old: Option<Ipv4Addr>, plan: &TunPlan) -> Result<()> {
    local::retarget::<Windows>(old, plan).await
}

pub async fn down(server_ip: Option<Ipv4Addr>) -> Result<()> {
    local::down::<Windows>(server_ip).await
}

pub async fn engine_alive() -> bool {
    local::engine_alive().await
}

pub fn purge_stale(stale_hosts: &[Ipv4Addr]) {
    sync_cleanup_inner(stale_hosts)
}
