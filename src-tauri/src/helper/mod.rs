// src-tauri/src/helper/mod.rs
//! The privileged Linux helper.
//!
//! The same executable, restarted through `pkexec` with `--helper`. It owns the
//! TUN device, the routing table entries and DNS, and it is the only part of
//! the app that runs as root — the GUI and the webview stay unprivileged, which
//! is stricter than the macOS model where the whole process is root.
//!
//! Lifetime: it is a child of the GUI and its only input is the pipe on stdin.
//! When the GUI exits — cleanly, crashed or killed — the pipe reaches EOF and
//! the helper tears the tunnel down before exiting. A SIGKILL of the helper
//! itself is covered by `purge_stale_sync()` on the next start.

pub mod proto;

#[cfg(target_os = "linux")]
mod server;

#[cfg(target_os = "linux")]
pub use server::{emit_log, is_helper_invocation, run_helper};
