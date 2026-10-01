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
    ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToIndex, FreeMibTable, GetBestRoute2,
    GetIpForwardTable2, MIB_IPFORWARD_ROW2, MIB_IPFORWARD_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_INET};

use crate::log;
use crate::net::plan::{self, windows as p, SPLIT_HIGH, SPLIT_LOW};
use crate::net::PhysicalRoute;

pub use plan::windows::DEVICE;

/// Probe addresses used to confirm that both halves of the default route point
/// into the tunnel: 1.0.0.1 falls inside 0.0.0.0/1, 200.0.0.1 inside
/// 128.0.0.0/1. These are route-table lookups only — no packet is sent.
const PROBE_LOW: Ipv4Addr = Ipv4Addr::new(1, 0, 0, 1);
const PROBE_HIGH: Ipv4Addr = Ipv4Addr::new(200, 0, 0, 1);

/// tun2socks takes `[driver://]name`; the bare name selects the platform's
/// default driver, which is Wintun on Windows.
pub fn tun2socks_device_arg() -> String {
    DEVICE.to_string()
}

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

/// The machine's real IPv4 default route, read from the forwarding table.
///
/// Why the table and not `GetBestRoute2`: once our 0.0.0.0/1 and 128.0.0.0/1
/// are installed, a best-path lookup for any address answers "the tunnel". The
/// physical default is still its own 0.0.0.0/0 row, so read that row directly —
/// the same reason macOS asks `route get default` instead of `route get <ip>`.
/// Rows on our own adapter are skipped, and the lowest metric wins, which is
/// how Windows itself picks between several defaults.
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
            let metric = row.Metric;
            if best.as_ref().map(|(_, m)| metric < *m).unwrap_or(true) {
                best = Some((candidate, metric));
            }
        }
        FreeMibTable(table as *const _);
    }

    best.map(|(route, _)| route).ok_or_else(|| {
        anyhow!("нет сетевого подключения — не найден шлюз по умолчанию")
    })
}

// ----------------------------------------------------------------- contract

pub async fn physical_route() -> Result<PhysicalRoute> {
    let ours = device_index().ok();
    physical_default(ours)
}

pub async fn add_host_route(dest: Ipv4Addr, via: &PhysicalRoute) -> Result<()> {
    // Drop a leftover from an earlier run before installing ours, otherwise
    // netsh refuses the duplicate prefix.
    p::host_route_delete(dest).run_best_effort().await;
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

pub async fn engine_alive(stem: &str) -> bool {
    let image = p::image_name(stem);
    match p::process_alive(&image).stdout_lossy().await {
        Some(text) => p::parse_tasklist_alive(&text, &image),
        None => false,
    }
}

pub async fn kill_stray(stem: &str) {
    p::kill_stray(&p::image_name(stem)).run_best_effort().await;
}

pub async fn kill_stray_force(stem: &str) {
    // taskkill /F is already a hard kill; there is no gentler variant to
    // escalate from, so both entry points do the same thing.
    kill_stray(stem).await;
}

/// Synchronous sweep for the exit path, where no runtime is available.
pub fn sync_cleanup(stale_hosts: &[Ipv4Addr]) {
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
    for stem in crate::net::ENGINE_STEMS {
        run(p::kill_stray(&p::image_name(stem)));
    }
    log::info("net", "cleanup done");
}
