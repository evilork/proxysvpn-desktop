// src/sysdns.rs
//
// The system resolver, pointed into the tunnel for as long as it is up.
//
// ── Why (27.09.2026) ───────────────────────────────────
// With the tunnel up, nothing loaded on the owner's Mac on any transport.
// mDNSResponder's log showed why: the Mac's DNS is 8.8.8.8, macOS had upgraded
// it to encrypted DNS (DoH to dns.google, discovered through DDR) and bound
// that connection to Wi-Fi. When our routes came up the connection stalled —
// "Having stream problems with HTTPS server", queries resent #2…#5 with no
// answer — and the plain servers were marked unusable. Names did not resolve
// for the whole session and came back the second the tunnel went down.
//
// ── What ───────────────────────────────────────────────
// For the length of the session our tunnel is announced to the system as a
// network service of its own and made primary (`OverridePrimary`), with one
// DNS server: `TUNNEL_DNS`, an address inside the tunnel that xray answers
// (routing rule to `dns-out`, see subscription.rs). The system then asks us,
// through the tunnel, instead of keeping a Wi-Fi-bound connection alive. The
// same keys openconnect's vpnc-script writes with `scutil`.
//
// ── Why temporary values ───────────────────────────────
// `SCDynamicStoreAddTemporaryValue` ties the keys to our session with configd:
// they disappear when the store is released or the process dies, crash
// included. A persistent change (`networksetup -setdnsservers`) left behind by
// a crash would leave the Mac with a DNS server that answers only while our
// tunnel runs — no internet at all until someone fixes it by hand.
//
// macOS only; writing `State:` keys needs root, which the app already has.

use std::ffi::c_void;

use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

pub use crate::subscription::TUNNEL_DNS;

/// Our service id in the dynamic store. Fixed: there is one tunnel at a time,
/// and a fixed id means a second install replaces rather than stacks.
const SERVICE_ID: &str = "ProxysVPN-Tunnel";

type SCDynamicStoreRef = *const c_void;

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {
    fn SCDynamicStoreCreate(
        allocator: *const c_void,
        name: CFStringRef,
        callout: *const c_void,
        context: *const c_void,
    ) -> SCDynamicStoreRef;
    fn SCDynamicStoreAddTemporaryValue(
        store: SCDynamicStoreRef,
        key: CFStringRef,
        value: *const c_void,
    ) -> u8;
    fn SCDynamicStoreRemoveValue(store: SCDynamicStoreRef, key: CFStringRef) -> u8;
    fn SCError() -> i32;
}

/// One value of a service dictionary, before it becomes Core Foundation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlistValue {
    Str(String),
    Strs(Vec<String>),
    Int(i32),
}

/// The two keys and their contents. Pure, so the shape is testable without
/// root and without touching the machine.
pub fn service_entries(interface: &str, tun_addr: &str, dns: &str) -> Vec<(String, Vec<(&'static str, PlistValue)>)> {
    vec![
        (
            format!("State:/Network/Service/{SERVICE_ID}/IPv4"),
            vec![
                ("Addresses", PlistValue::Strs(vec![tun_addr.to_string()])),
                ("SubnetMasks", PlistValue::Strs(vec!["255.255.255.255".to_string()])),
                ("InterfaceName", PlistValue::Str(interface.to_string())),
                ("Router", PlistValue::Str(tun_addr.to_string())),
                // Makes this the primary service, so its DNS is the default
                // resolver and not a supplemental one nobody asks.
                ("OverridePrimary", PlistValue::Int(1)),
            ],
        ),
        (
            format!("State:/Network/Service/{SERVICE_ID}/DNS"),
            vec![("ServerAddresses", PlistValue::Strs(vec![dns.to_string()]))],
        ),
    ]
}

fn to_cf(value: &PlistValue) -> CFType {
    match value {
        PlistValue::Str(s) => CFString::new(s).as_CFType(),
        PlistValue::Strs(items) => {
            let strings: Vec<CFString> = items.iter().map(|s| CFString::new(s)).collect();
            CFArray::from_CFTypes(&strings).as_CFType()
        }
        PlistValue::Int(n) => CFNumber::from(*n).as_CFType(),
    }
}

/// Keys installed in the dynamic store; removed on drop.
pub struct TunnelDns {
    store: SCDynamicStoreRef,
    keys: Vec<CFString>,
}

// SAFETY: the store is a Core Foundation object; retain/release are
// thread-safe, and the only calls made on it (add/remove value) do not
// depend on a run loop or on the creating thread.
unsafe impl Send for TunnelDns {}
unsafe impl Sync for TunnelDns {}

impl TunnelDns {
    /// Announce the tunnel as the primary service with `TUNNEL_DNS`.
    ///
    /// Either both keys are in place or neither is: a primary service without
    /// DNS would take the system's resolver away and give nothing back.
    pub fn install(interface: &str, tun_addr: &str) -> Result<Self, String> {
        let name = CFString::new("ProxysVPN");
        // SAFETY: plain C call with a valid CFString; no callout, no context.
        let store = unsafe {
            SCDynamicStoreCreate(std::ptr::null(), name.as_concrete_TypeRef(), std::ptr::null(), std::ptr::null())
        };
        if store.is_null() {
            // SAFETY: reads the thread's last SystemConfiguration error.
            return Err(format!("SCDynamicStoreCreate failed ({})", unsafe { SCError() }));
        }
        let mut installed = TunnelDns { store, keys: Vec::new() };
        for (key, entries) in service_entries(interface, tun_addr, TUNNEL_DNS) {
            let pairs: Vec<(CFString, CFType)> = entries
                .iter()
                .map(|(k, v)| (CFString::new(k), to_cf(v)))
                .collect();
            let dict = CFDictionary::from_CFType_pairs(&pairs);
            let cf_key = CFString::new(&key);
            // SAFETY: store is live; key and value are valid CF objects that
            // outlive the call.
            let ok = unsafe {
                SCDynamicStoreAddTemporaryValue(
                    installed.store,
                    cf_key.as_concrete_TypeRef(),
                    dict.as_concrete_TypeRef() as *const c_void,
                )
            };
            if ok == 0 {
                // SAFETY: as above.
                let code = unsafe { SCError() };
                // Dropping `installed` removes whatever was already added.
                return Err(format!("{key}: SCDynamicStoreAddTemporaryValue failed ({code})"));
            }
            installed.keys.push(cf_key);
        }
        Ok(installed)
    }
}

impl Drop for TunnelDns {
    fn drop(&mut self) {
        // DNS first, then the service: never a primary service without DNS.
        for key in self.keys.iter().rev() {
            // SAFETY: store is live until the release below.
            unsafe {
                SCDynamicStoreRemoveValue(self.store, key.as_concrete_TypeRef());
            }
        }
        // SAFETY: we own the store's single reference from Create. Releasing
        // it would drop the temporary values anyway; the removals above just
        // make the change immediate and explicit.
        unsafe { CFRelease(self.store) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tunnel_becomes_the_primary_service_with_one_dns_server_inside_it() {
        let entries = service_entries("utun225", "198.18.0.1", TUNNEL_DNS);
        assert_eq!(entries.len(), 2);
        let (ipv4_key, ipv4) = &entries[0];
        assert_eq!(ipv4_key, "State:/Network/Service/ProxysVPN-Tunnel/IPv4");
        assert!(ipv4.contains(&("InterfaceName", PlistValue::Str("utun225".into()))));
        assert!(ipv4.contains(&("OverridePrimary", PlistValue::Int(1))));
        assert!(ipv4.contains(&("Addresses", PlistValue::Strs(vec!["198.18.0.1".into()]))));
        let (dns_key, dns) = &entries[1];
        assert_eq!(dns_key, "State:/Network/Service/ProxysVPN-Tunnel/DNS");
        assert_eq!(dns, &vec![("ServerAddresses", PlistValue::Strs(vec!["198.18.0.2".into()]))]);
    }

    #[test]
    fn the_dns_server_is_reached_through_the_tunnel_split_route() {
        // 128.0.0.0/1 is ours; the address must fall into it (or 0/1).
        let first: u8 = TUNNEL_DNS.split('.').next().unwrap().parse().unwrap();
        assert!(first >= 128, "{TUNNEL_DNS} would not be routed into utun");
    }

    #[test]
    fn without_root_nothing_is_installed_and_the_error_says_so() {
        if crate::tun::is_root() {
            return; // Would touch the real machine; the live run covers it.
        }
        let err = TunnelDns::install("utun225", "198.18.0.1").err().expect("needs root");
        assert!(err.contains("SCDynamicStoreAddTemporaryValue"), "{err}");
    }
}
