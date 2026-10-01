// src-tauri/src/tun/sys/mod.rs
//! Platform backends for the tunnel. Exactly one is compiled; `active` is the
//! alias the portable code in `tun::mod` talks to. See `contract.rs`.

pub mod contract;

/// Host-independent Linux logic (route-table parsers, resolver files). Built on
/// every platform so its unit tests run on the macOS developer machine, which
/// is the only one available for this port.
pub mod linux_logic;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos as active;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux as active;

// The Windows backend lands with feat/desktop-windows; the arm is declared here
// so that branch only has to add the file.
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows as active;
