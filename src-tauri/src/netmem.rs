// src/netmem.rs
//
// Which location worked on WHICH network (Watafast, phase 1).
//
// `last_good` remembered one location for the whole app. That is right at
// home and wrong everywhere else: the location that gets through on the home
// Wi-Fi may be the one a mobile operator or an office filter kills first, and
// the app would start every connect there with the wrong one. Now the last
// confirmed location is remembered per network, and a known network starts on
// its own winner.
//
// ── How a network is recognised ────────────────────────
// By the hardware address of its default gateway (the router), hashed with a
// per-install salt, plus the interface it is reached through. No Wi-Fi name:
// reading the SSID on macOS needs the location permission, and a prompt for
// "location" in a VPN app is a question nobody should have to answer. Where the
// gateway has no hardware address to read (cellular, some tethering) the
// gateway address stands in. Nothing identifying is stored: only the salted
// hash, never the address, and the file never leaves the machine.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How long a network's winner is trusted without being confirmed again.
pub const ENTRY_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// Networks kept at most; the oldest is forgotten first.
pub const MAX_NETWORKS: usize = 32;

/// A network's fingerprint: 16 hex characters of SHA-256 over salt,
/// interface and the gateway's hardware address (or its IP when there is none).
/// Pure, so it is testable; `None` when there is nothing to recognise it by.
pub fn fingerprint(
    salt: &[u8],
    interface: &str,
    gateway_mac: Option<&str>,
    gateway_ip: Option<&str>,
) -> Option<String> {
    let anchor = gateway_mac
        .map(|m| format!("mac:{}", m.trim().to_ascii_lowercase()))
        .or_else(|| gateway_ip.map(|ip| format!("ip:{}", ip.trim())))?;
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(b"\0");
    hasher.update(interface.trim().as_bytes());
    hasher.update(b"\0");
    hasher.update(anchor.as_bytes());
    let digest = hasher.finalize();
    Some(digest.iter().take(8).map(|b| format!("{b:02x}")).collect())
}

/// Read the gateway's hardware address out of `arp -n <ip>` output:
/// "? (192.168.1.1) at a4:2b:b0:12:34:56 on en0 ifscope [ethernet]".
/// `(incomplete)` and anything unparseable give `None`.
pub fn parse_arp_mac(output: &str) -> Option<String> {
    let after = output.split(" at ").nth(1)?;
    let mac = after.split_whitespace().next()?;
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() == 6 && parts.iter().all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_hexdigit())) {
        Some(mac.to_ascii_lowercase())
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    location: String,
    at_ms: u64,
}

/// The per-network winners, as stored on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkMemory {
    networks: HashMap<String, Entry>,
}

impl NetworkMemory {
    /// The location that last worked on `network`, if still fresh.
    pub fn winner(&self, network: &str, now_ms: u64) -> Option<&str> {
        self.networks
            .get(network)
            .filter(|e| now_ms.saturating_sub(e.at_ms) < ENTRY_TTL_MS)
            .map(|e| e.location.as_str())
    }

    /// Remember that `location` worked on `network` just now.
    pub fn remember(&mut self, network: &str, location: &str, now_ms: u64) {
        self.networks.insert(
            network.to_string(),
            Entry { location: location.to_string(), at_ms: now_ms },
        );
        self.prune(now_ms);
    }

    /// Forget `network`'s winner — it stopped getting through there.
    pub fn forget(&mut self, network: &str) {
        self.networks.remove(network);
    }

    /// Drop stale entries and keep the newest MAX_NETWORKS. O(n log n).
    fn prune(&mut self, now_ms: u64) {
        self.networks
            .retain(|_, e| now_ms.saturating_sub(e.at_ms) < ENTRY_TTL_MS);
        if self.networks.len() > MAX_NETWORKS {
            let mut by_age: Vec<(String, u64)> =
                self.networks.iter().map(|(k, e)| (k.clone(), e.at_ms)).collect();
            by_age.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
            for (key, _) in by_age.into_iter().skip(MAX_NETWORKS) {
                self.networks.remove(&key);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.networks.len()
    }
}

// ── Storage: beside the subscription link, same two-location rule ─────────

fn dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    out.push(PathBuf::from("/Library/Application Support/ProxysVPN"));
    if let Ok(home) = std::env::var("HOME") {
        let mut base = PathBuf::from(home).join("Library/Application Support");
        if cfg!(target_os = "macos") {
            base = base.join("com.proxysvpn.desktop");
        }
        out.push(base);
    }
    out
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> bool {
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    if std::fs::write(path, bytes).is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    true
}

/// The per-install salt; created on first use from the system's randomness.
pub fn salt() -> Option<Vec<u8>> {
    for dir in dir_candidates() {
        if let Ok(bytes) = std::fs::read(dir.join("netmem-salt")) {
            if bytes.len() == 16 {
                return Some(bytes);
            }
        }
    }
    let mut fresh = [0u8; 16];
    use std::io::Read;
    std::fs::File::open("/dev/urandom").ok()?.read_exact(&mut fresh).ok()?;
    for dir in dir_candidates() {
        if write_private(&dir.join("netmem-salt"), &fresh) {
            return Some(fresh.to_vec());
        }
    }
    // Could not store it: a salt that changes every run would make every
    // network new every time, which is merely the old behaviour.
    Some(fresh.to_vec())
}

/// Load the memory; any trouble gives an empty one (the old behaviour).
pub fn load() -> NetworkMemory {
    for dir in dir_candidates() {
        if let Ok(text) = std::fs::read_to_string(dir.join("network-memory.json")) {
            if let Ok(mem) = serde_json::from_str::<NetworkMemory>(&text) {
                return mem;
            }
        }
    }
    NetworkMemory::default()
}

/// Store the memory. Best effort: losing it costs one slower connect.
pub fn store(mem: &NetworkMemory) -> bool {
    let Ok(text) = serde_json::to_string(mem) else {
        return false;
    };
    dir_candidates()
        .into_iter()
        .any(|dir| write_private(&dir.join("network-memory.json"), text.as_bytes()))
}

/// The current network's id, read before the tunnel comes up (macOS).
#[cfg(target_os = "macos")]
pub async fn current_network() -> Option<String> {
    let route = crate::tun::physical_default().await.ok()?;
    let mac = match route.gateway.as_deref() {
        Some(gw) => {
            let out = tokio::process::Command::new("/usr/sbin/arp")
                .args(["-n", gw])
                .output()
                .await
                .ok()?;
            parse_arp_mac(&String::from_utf8_lossy(&out.stdout))
        }
        None => None,
    };
    fingerprint(&salt()?, &route.interface, mac.as_deref(), route.gateway.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: &[u8] = b"0123456789abcdef";

    #[test]
    fn the_same_router_is_the_same_network_and_the_salt_hides_it() {
        let a = fingerprint(SALT, "en0", Some("A4:2B:B0:12:34:56"), Some("192.168.1.1")).unwrap();
        let b = fingerprint(SALT, "en0", Some("a4:2b:b0:12:34:56"), Some("10.0.0.1")).unwrap();
        assert_eq!(a, b, "the router's hardware address decides, not its IP");
        assert_eq!(a.len(), 16);
        assert!(!a.contains("a4"), "no piece of the address in the id");
        let other_salt = fingerprint(b"fedcba9876543210", "en0", Some("a4:2b:b0:12:34:56"), None).unwrap();
        assert_ne!(a, other_salt, "another install cannot match ids");
    }

    #[test]
    fn another_router_or_interface_is_another_network() {
        let home = fingerprint(SALT, "en0", Some("a4:2b:b0:12:34:56"), None).unwrap();
        assert_ne!(home, fingerprint(SALT, "en0", Some("a4:2b:b0:12:34:57"), None).unwrap());
        assert_ne!(home, fingerprint(SALT, "en7", Some("a4:2b:b0:12:34:56"), None).unwrap());
    }

    #[test]
    fn without_a_hardware_address_the_gateway_ip_stands_in_and_nothing_means_none() {
        let cell = fingerprint(SALT, "pdp_ip0", None, Some("10.64.0.1")).unwrap();
        assert_eq!(cell.len(), 16);
        assert_eq!(fingerprint(SALT, "pdp_ip0", None, None), None);
    }

    #[test]
    fn arp_output_is_read_and_incomplete_entries_are_not() {
        let out = "? (192.168.1.1) at a4:2b:b0:12:34:56 on en0 ifscope [ethernet]\n";
        assert_eq!(parse_arp_mac(out).as_deref(), Some("a4:2b:b0:12:34:56"));
        assert_eq!(parse_arp_mac("? (192.168.1.1) at 0:1b:2c:3:4:5 on en0 [ethernet]").as_deref(), Some("0:1b:2c:3:4:5"));
        assert_eq!(parse_arp_mac("? (192.168.1.1) at (incomplete) on en0 ifscope [ethernet]"), None);
        assert_eq!(parse_arp_mac("192.168.1.1 (192.168.1.1) -- no entry"), None);
    }

    #[test]
    fn a_winner_is_per_network_and_goes_stale() {
        let mut mem = NetworkMemory::default();
        mem.remember("home", "loc-de", 1_000);
        mem.remember("office", "loc-uk-x", 2_000);
        assert_eq!(mem.winner("home", 5_000), Some("loc-de"));
        assert_eq!(mem.winner("office", 5_000), Some("loc-uk-x"));
        assert_eq!(mem.winner("cafe", 5_000), None);
        assert_eq!(mem.winner("home", 1_000 + ENTRY_TTL_MS), None, "30 days without a confirmation");
        mem.remember("home", "loc-fr", 6_000);
        assert_eq!(mem.winner("home", 7_000), Some("loc-fr"), "the newest confirmation wins");
        mem.forget("home");
        assert_eq!(mem.winner("home", 7_000), None);
    }

    #[test]
    fn at_most_max_networks_are_kept_newest_first() {
        let mut mem = NetworkMemory::default();
        for i in 0..(MAX_NETWORKS as u64 + 5) {
            mem.remember(&format!("net{i}"), "loc", 10_000 + i);
        }
        assert_eq!(mem.len(), MAX_NETWORKS);
        assert_eq!(mem.winner("net0", 20_000), None, "the oldest went first");
        assert!(mem.winner(&format!("net{}", MAX_NETWORKS as u64 + 4), 20_000).is_some());
    }

    #[test]
    fn it_survives_a_round_trip_through_json() {
        let mut mem = NetworkMemory::default();
        mem.remember("home", "loc-de", 1_000);
        let text = serde_json::to_string(&mem).unwrap();
        let back: NetworkMemory = serde_json::from_str(&text).unwrap();
        assert_eq!(back, mem);
    }
}
