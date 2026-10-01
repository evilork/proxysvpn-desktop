// src-tauri/crates/pvpn-platform/src/net/macos.rs
// macOS implementation of the net contract.
//
// Every command and every parse is the pre-split tun.rs, moved verbatim. The
// argv itself comes from net::plan::macos, which is pinned by golden tests —
// this file only decides *when* to run them.
//
// Note on the device: tun2socks creates utun225 itself (wireguard-go's utun
// driver), so we wait for it to appear instead of opening it, then assign the
// address and the two half-default routes.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use crate::log;
use crate::net::plan::{self, macos as p, SPLIT_HIGH, SPLIT_LOW};
use crate::net::PhysicalRoute;

pub use plan::macos::DEVICE;

pub async fn physical_route() -> Result<PhysicalRoute> {
    let text = p::default_route_query()
        .stdout_lossy()
        .await
        .ok_or_else(|| anyhow!("could not run route -n get default"))?;
    p::parse_default_route(&text)
}

pub async fn add_host_route(dest: Ipv4Addr, via: &PhysicalRoute) -> Result<()> {
    // Remove a stale entry first: a previous crash can leave a host route that
    // points at a gateway which no longer exists.
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
    match p::host_route_query(dest).stdout_lossy().await {
        Some(text) => p::parse_host_route_ok(&text),
        None => false,
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
    match p::routing_table_query().stdout_lossy().await {
        Some(text) => p::parse_split_defaults_ok(&text),
        None => false,
    }
}

pub async fn wait_for_device(timeout: Duration) -> Result<()> {
    let step = Duration::from_millis(100);
    let attempts = (timeout.as_millis() / step.as_millis().max(1)).max(1) as u32;
    for _ in 0..attempts {
        tokio::time::sleep(step).await;
        if p::device_probe().succeeds().await {
            return Ok(());
        }
    }
    Err(anyhow!(
        "{} did not come up within {}s",
        DEVICE,
        timeout.as_secs()
    ))
}

pub async fn configure_device() -> Result<()> {
    p::device_configure()
        .run()
        .await
        .with_context(|| format!("assign IP to {}", DEVICE))
}

pub async fn device_down() {
    p::device_down().run_best_effort().await;
}

pub async fn kill_stray(stem: &str) {
    p::kill_stray(stem).run_best_effort().await;
}

/// Synchronous sweep for the exit/signal path, where there is no runtime to
/// await on. Mirrors the pre-split `sync_cleanup`.
fn sync_cleanup_inner(stale_hosts: &[Ipv4Addr]) {
    use std::process::Command;

    let run = |argv: crate::Argv| {
        let _ = Command::new(&argv.program).args(&argv.args).status();
    };

    for stem in crate::net::ENGINE_STEMS {
        run(p::kill_stray_force(stem));
    }
    run(p::split_default_delete(SPLIT_LOW));
    run(p::split_default_delete(SPLIT_HIGH));
    run(p::device_down());
    for host in stale_hosts {
        run(p::host_route_delete(*host));
    }
    log::info("net", "cleanup done");
}

// ===========================================================================
// The contract
// ===========================================================================
//
// macOS is a privileged-GUI platform: scripts/launcher.sh asks for the password
// once via osascript and re-execs us through `launchctl asuser`, so the whole
// process is root while staying inside the user's GUI session. The sequence
// therefore runs in-process, and `net::local` holds it (shared with Windows).

use std::net::IpAddr;
use std::path::Path;

use crate::net::local::{self, Privileged};
use crate::net::TunPlan;
use crate::process::Argv;

struct MacOs;

impl Privileged for MacOs {
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

    /// Nothing to do, and that is deliberate.
    ///
    /// The macOS resolver keeps using the physical link's servers and those
    /// queries are simply proxied, because their addresses fall under
    /// 0.0.0.0/1 like all other traffic. Pointing the system resolver at an
    /// address inside the tunnel would need `networksetup -setdnsservers` per
    /// service plus a restore on every exit path, and would break name
    /// resolution outright if we ever failed to restore it. The leak this
    /// leaves — the physical resolver sees the names — is the same one the
    /// shipping build has, and fixing it belongs with the `dns` section of the
    /// generated xray config, not here.
    async fn configure_dns(_servers: &[IpAddr]) -> Result<()> {
        Ok(())
    }

    async fn restore_dns() {}

    async fn kill_stray(stem: &str) {
        kill_stray(stem).await
    }

}

/// macOS needs root for `route` and `ifconfig`. The launcher provides it; if we
/// are running without it, say so before touching anything.
pub async fn preflight() -> Result<()> {
    if crate::privilege::is_elevated() {
        return Ok(());
    }
    Err(anyhow!("{}", crate::privilege::missing_privileges_message()))
}

pub async fn up(plan: &TunPlan) -> Result<()> {
    local::up::<MacOs>(plan).await
}

pub async fn ensure(plan: &TunPlan) -> Result<()> {
    local::ensure::<MacOs>(plan).await
}

pub async fn retarget(old: Option<Ipv4Addr>, plan: &TunPlan) -> Result<()> {
    local::retarget::<MacOs>(old, plan).await
}

pub async fn down(server_ip: Option<Ipv4Addr>) -> Result<()> {
    local::down::<MacOs>(server_ip).await
}

pub async fn engine_alive() -> bool {
    local::engine_alive().await
}

pub fn purge_stale(stale_hosts: &[Ipv4Addr]) {
    sync_cleanup_inner(stale_hosts)
}
