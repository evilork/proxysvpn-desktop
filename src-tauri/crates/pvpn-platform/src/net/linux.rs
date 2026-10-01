// src-tauri/crates/pvpn-platform/src/net/linux.rs
//
// PLACEHOLDER — the Linux tunnel lands on its own branch (feat/desktop-linux)
// and replaces this file wholesale. It exists so that the contract in
// net/mod.rs is already final and a Linux build fails loudly at runtime
// instead of silently doing half of the work.
//
// What the real implementation has to add (all of it already sketched in
// net::plan::linux):
//   * /dev/net/tun via tun2socks `-device tun://proxysvpn0`, CAP_NET_ADMIN;
//   * `ip addr add 198.18.0.1/24`, `ip link set … up`, both halves of the
//     default route, host route to the node via /proc/net/route;
//   * DNS: `resolvectl dns proxysvpn0 …` where systemd-resolved runs, a
//     backed-up /etc/resolv.conf where it does not;
//   * privileges: xray's sockopt.interface needs CAP_NET_RAW, and file
//     capabilities are not inherited by children, so the engines must be
//     started by a privileged helper (pkexec, one prompt per session) rather
//     than by a root GUI — a root GUI cannot talk to a Wayland compositor.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::net::plan;
use crate::net::PhysicalRoute;

pub use plan::linux::DEVICE;

fn not_implemented<T>() -> Result<T> {
    Err(anyhow!(
        "поддержка Linux ещё не готова (ветка feat/desktop-linux)"
    ))
}

pub fn tun2socks_device_arg() -> String {
    format!("tun://{}", DEVICE)
}

pub async fn physical_route() -> Result<PhysicalRoute> {
    not_implemented()
}

pub async fn add_host_route(_dest: Ipv4Addr, _via: &PhysicalRoute) -> Result<()> {
    not_implemented()
}

pub async fn delete_host_route(_dest: Ipv4Addr) {}

pub async fn host_route_ok(_dest: Ipv4Addr) -> bool {
    false
}

pub async fn add_split_defaults() -> Result<()> {
    not_implemented()
}

pub async fn delete_split_defaults() {}

pub async fn split_defaults_ok() -> bool {
    false
}

pub async fn wait_for_device(_timeout: Duration) -> Result<()> {
    not_implemented()
}

pub async fn configure_device() -> Result<()> {
    not_implemented()
}

pub async fn device_down() {}

pub async fn engine_alive(_stem: &str) -> bool {
    false
}

pub async fn kill_stray(_stem: &str) {}

pub async fn kill_stray_force(_stem: &str) {}

pub fn sync_cleanup(_stale_hosts: &[Ipv4Addr]) {}
