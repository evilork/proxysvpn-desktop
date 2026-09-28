// src-tauri/src/ios_vpn.rs
// iOS VPN control path.
//
// On iOS the app process cannot spawn engines or touch the routing table.
// The tunnel runs in a separate Network Extension (see
// gen/apple/PacketTunnel/), and the app talks to it through
// NETunnelProviderManager. That API is Swift-only, so this module calls into
// gen/apple/Sources/proxysvpn-desktop/VpnBridge.swift via C FFI — both sides
// are linked into the same app binary (@_cdecl on the Swift side).
//
// Flow: fetch subscription (reqwest works fine on iOS) → build the Xray config
// and the tunnel addresses (xray_apple.rs) → hand the JSON to Swift → Swift
// saves the NE profile and starts the tunnel → we poll the session status
// until connected or timed out.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::subscription::ServerConfig;

// Mirrors NEVPNStatus raw values.
const STATUS_DISCONNECTED: i32 = 1;
const STATUS_CONNECTED: i32 = 3;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

extern "C" {
    // Implemented in VpnBridge.swift.
    fn pvpn_tunnel_start(config_json: *const c_char);
    fn pvpn_tunnel_stop();
    /// NEVPNStatus raw value; -1 when the manager failed to load.
    fn pvpn_tunnel_status() -> i32;
    /// Last error message (heap copy, caller frees via pvpn_string_free) or null.
    fn pvpn_tunnel_last_error() -> *mut c_char;
    fn pvpn_string_free(ptr: *mut c_char);
}

fn last_error() -> Option<String> {
    // SAFETY: the bridge returns either null or a strdup'ed C string that we
    // free exactly once via pvpn_string_free.
    unsafe {
        let ptr = pvpn_tunnel_last_error();
        if ptr.is_null() {
            return None;
        }
        let msg = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        pvpn_string_free(ptr);
        Some(msg)
    }
}

fn status_raw() -> i32 {
    unsafe { pvpn_tunnel_status() }
}

/// True when NE reports "connected"; waits out the initial NE-preferences load
/// (bridge reports -2 while loading). Used by `vpn_status` so the UI shows
/// the right state right after app launch.
pub async fn is_connected_wait() -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let status = status_raw();
        if status != -2 {
            return status == STATUS_CONNECTED;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Starts the tunnel for `server` and waits until NE reports "connected".
///
/// The routing profile and the person's tunnel settings are read here, like
/// the desktop's `build_xray_config` does, so the extension runs the same
/// rules as the Mac.
pub async fn connect(server: &ServerConfig) -> Result<()> {
    if server.host().parse::<std::net::Ipv4Addr>().is_ok() {
        // Not an error: it works on every network with IPv4. On an IPv6-only
        // one (NAT64, App Review's own) only a NAME gets a synthesized IPv6
        // address, so say why that network would fail, without the address.
        crate::logger::log("warn", "ios-vpn", "node is an IPv4 literal: unreachable on IPv6-only (NAT64) networks");
    }
    let config_str = crate::xray_apple::provider_configuration(
        server,
        crate::subscription::last_routing().as_ref(),
        &crate::tunnel_prefs::load(),
    )?;
    let config_c = CString::new(config_str)
        .map_err(|_| anyhow!("config contains interior NUL byte"))?;

    crate::logger::log(
        "info",
        "ios-vpn",
        &format!("starting tunnel to {} ({})", server.host(), server.proto()),
    );

    unsafe { pvpn_tunnel_start(config_c.as_ptr()) };

    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        if let Some(err) = last_error() {
            return Err(anyhow!("{}", err));
        }
        match status_raw() {
            STATUS_CONNECTED => {
                crate::logger::log("info", "ios-vpn", "tunnel connected");
                return Ok(());
            }
            // Invalid (0) right after start just means prefs are still
            // saving; keep polling until the deadline.
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            unsafe { pvpn_tunnel_stop() };
            return Err(anyhow!(
                "VPN did not connect within {}s (status {})",
                CONNECT_TIMEOUT.as_secs(),
                status_raw()
            ));
        }
    }
}

/// Stops the tunnel; waits briefly for a clean shutdown (best effort).
pub async fn disconnect() {
    crate::logger::log("info", "ios-vpn", "stopping tunnel");
    unsafe { pvpn_tunnel_stop() };

    let deadline = tokio::time::Instant::now() + DISCONNECT_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if matches!(status_raw(), STATUS_DISCONNECTED | 0 | -1) {
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
