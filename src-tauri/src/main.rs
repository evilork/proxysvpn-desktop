// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "ios"))]
fn main() {
    proxysvpn_desktop_lib::run()
}

// On iOS the app is the staticlib that Xcode links as libapp.a, started
// through run()'s mobile_entry_point; this binary is never shipped. Cargo
// still builds it (`tauri ios xcode-script` runs a plain `cargo build`), and
// pulling run() in would need the pvpn_* symbols that only VpnBridge.swift
// defines at Xcode link time, so on iOS it stays empty.
#[cfg(target_os = "ios")]
fn main() {}
