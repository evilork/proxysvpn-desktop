// src-tauri/src/tun/sys/contract.rs
//! The contract every platform backend under `tun::sys` implements.
//!
//! There is no `dyn` trait here on purpose: exactly one backend is compiled
//! into any given build, so a trait would buy nothing and cost an
//! `async-trait` boxing layer. Instead `tun::sys::active` is a module alias and
//! the compiler checks the signatures at the call sites in `tun::mod`. What is
//! documented below is therefore normative — all three backends must match it
//! name for name:
//!
//! ```text
//! pub const TUN_NAME: &str;
//!
//! /// Fail fast before anything is changed: privileges, helper, sidecars.
//! pub async fn preflight() -> anyhow::Result<()>;
//!
//! /// Raise the tunnel. Must leave nothing behind on failure.
//! pub async fn up(plan: &TunPlan) -> anyhow::Result<()>;
//!
//! /// Supervisor tick (every 5 s): re-assert host route, split defaults, DNS.
//! pub async fn ensure(plan: &TunPlan) -> anyhow::Result<()>;
//!
//! /// Tear the tunnel down. Idempotent; safe to call when nothing is up.
//! pub async fn down(server_ip: Option<&str>) -> anyhow::Result<()>;
//!
//! /// Is the tun2socks engine still running?
//! pub async fn engine_alive() -> bool;
//!
//! /// Synchronous crash recovery, run at startup and from signal handlers.
//! pub fn purge_stale();
//! ```

use std::net::IpAddr;
use std::path::PathBuf;

/// Everything a backend needs to raise or re-assert the tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunPlan {
    /// Resolved IPv4 address of the VPN node. Never logged: the node addresses
    /// are not public information.
    pub server_ip: String,
    /// Loopback SOCKS5 port of the engine in front of tun2socks.
    pub socks_port: u16,
    /// Absolute path to the tun2socks sidecar.
    pub tun2socks: PathBuf,
    /// Resolvers to publish on the tunnel interface. Reachable only through
    /// the tunnel, which is the point.
    pub dns: Vec<IpAddr>,
}

impl TunPlan {
    /// DNS servers as strings, for the Linux helper's JSON protocol.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn dns_strings(&self) -> Vec<String> {
        self.dns.iter().map(|ip| ip.to_string()).collect()
    }
}

/// Compile-time check of the non-async part of the contract. The async
/// functions are checked by `tun::mod`, which calls all of them.
#[allow(dead_code)]
const _CONTRACT: fn() = || {
    let _: &'static str = super::active::TUN_NAME;
    let _: fn() = super::active::purge_stale;
};
