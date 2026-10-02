// src-tauri/crates/pvpn-platform/src/bin/proxysvpn-helper.rs
//! The Linux root helper as a program of its own, for the AppImage.
//!
//! The .deb runs the GUI's own executable with `--helper`. The AppImage cannot
//! (helper/install.rs): root may not enter its mount, so the helper is copied
//! into a root-owned folder and run from there. The GUI executable would not
//! do as that copy — it links webkit2gtk, GTK and everything else the AppImage
//! carries in `usr/lib`, so outside the AppImage it would not even start on a
//! system without them. This one is the same `run_helper` without Tauri around
//! it, and links nothing of the desktop.
//!
//! Built by the `beforeBundleCommand` in tauri.linux.conf.json and put into
//! the AppImage's `usr/bin` by `bundle.linux.appimage.files`; the .deb does
//! not carry it.

fn main() {
    #[cfg(target_os = "linux")]
    if pvpn_platform::helper::is_helper_invocation() {
        pvpn_platform::helper::run_helper();
    }
    eprintln!("proxysvpn-helper: the ProxysVPN tunnel helper; the app starts it with --helper through pkexec, on Linux only");
    std::process::exit(2);
}
