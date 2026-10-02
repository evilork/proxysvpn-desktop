// src-tauri/crates/pvpn-platform/src/helper/mod.rs
//! The privileged Linux helper.
//!
//! The same executable, restarted through `pkexec` with `--helper`. It owns the
//! TUN device, the routing table entries and DNS, and it is the only part of the
//! app that runs as root — the GUI and the webview stay unprivileged, which is
//! stricter than the macOS and Windows model where the whole process is
//! elevated.
//!
//! Lifetime: it is a child of the GUI and its only input is the pipe on stdin.
//! When the GUI exits — cleanly, crashed or killed — the pipe reaches EOF and
//! the helper tears the tunnel down before exiting. A SIGKILL of the helper
//! itself is covered by `purge_stale_sync()` on the next start.
//!
//! The GUI crate has one job here: call `is_helper_invocation()` first thing in
//! `main`, and if it is true call `run_helper()` instead of starting Tauri. See
//! docs/LINUX.md.
//!
//! An AppImage cannot do that — root may not enter its mount — so it ships the
//! same helper as a binary of its own (src/bin/proxysvpn-helper.rs) and runs a
//! root-owned copy of it: `install`, and docs/STEAMDECK.md.

pub mod install;
pub mod proto;

/// Argument that turns this executable into the privileged helper.
pub const HELPER_FLAG: &str = "--helper";
/// Extra argument that relaxes the sidecar ownership check for `cargo run`.
pub const HELPER_DEV_FLAG: &str = "--dev";
/// Set in a developer shell only; a bundle never has it.
pub const DEV_ENV: &str = "PROXYSVPN_HELPER_DEV";

#[cfg(target_os = "linux")]
mod server;

#[cfg(target_os = "linux")]
pub use server::{emit_log, is_helper_invocation, run_helper};

/// On macOS and Windows the app is privileged itself, so there is no helper to
/// become and these two keep every call site free of `#[cfg]`.
#[cfg(not(target_os = "linux"))]
pub fn is_helper_invocation() -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn emit_log(level: &str, source: &str, message: &str) {
    crate::log::log(level, source, message);
}
