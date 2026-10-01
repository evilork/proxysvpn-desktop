// src-tauri/src/tun/sys/mod.rs
//! Platform backends for the tunnel. Exactly one is compiled; `active` is the
//! alias the portable code in `tun::mod` talks to. See `contract.rs`.

pub mod contract;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos as active;

// The Windows backend lands with feat/desktop-windows; the arm is declared here
// so that branch only has to add the file.
#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows as active;
