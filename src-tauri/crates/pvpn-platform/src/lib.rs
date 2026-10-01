// src-tauri/crates/pvpn-platform/src/lib.rs
//
// Platform layer of ProxysVPN Desktop.
//
// Everything that differs between macOS, Windows and Linux lives here:
// elevation checks, on-disk locations, the TUN device and the routing /DNS
// plumbing around it. The GUI crate (`proxysvpn-desktop`) keeps the portable
// orchestration and calls into `net` through one fixed contract — see
// `net::CONTRACT` in src/net/mod.rs.
//
// Two properties are load-bearing and must survive refactors:
//
//   1. macOS behaviour is byte-for-byte what shipped before the split. The
//      exact argv of every command it runs is produced by `net::plan::macos`
//      and pinned by golden tests, so a typo cannot silently change it.
//   2. The crate has no tauri/reqwest/rustls dependency and nothing in it needs
//      a C compiler, so the Windows and Linux implementations — including the
//      Linux root helper — can be type-checked and linted from a Mac:
//          cargo check -p pvpn-platform --target x86_64-pc-windows-msvc
//          cargo check -p pvpn-platform --target x86_64-unknown-linux-gnu
//      That is the only verification those two ports get on the machine this
//      was written on, so the dependency list must stay this short.

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("pvpn-platform supports macOS, Windows and Linux only");

pub mod helper;
pub mod log;
pub mod net;
pub mod paths;
pub mod privilege;
pub mod process;
pub mod triple;

pub use process::Argv;
