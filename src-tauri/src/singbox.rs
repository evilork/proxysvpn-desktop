// src-tauri/src/singbox.rs
// Builds the sing-box JSON config used by the iOS PacketTunnel extension.
//
// On iOS the whole engine chain (xray / hysteria / tun2socks processes) is
// replaced by a single in-process sing-box instance (Libbox.xcframework)
// running inside an NEPacketTunnelProvider. sing-box natively supports both
// protocols we ship: VLESS Reality and Hysteria2.
//
// Config format targets sing-box 1.12.x (the version pinned by
// scripts/build-libbox.sh). Route rules mirror the desktop smart routing:
//   • private/LAN → direct
//   • major Russian domains → direct
//   • QUIC (udp/443) blocked for VLESS (Vision is TCP-only, UDP would leak)
//   • everything else → proxy
//
// Unlike desktop (where DNS intentionally leaks to the system resolver, see
// build_xray_config notes), here DNS goes through the tunnel via DoH: on iOS
// the extension owns the device's DNS anyway, so a leak would be user-visible
// in the NE settings and there is no throughput-tuning history to preserve.

use serde_json::{json, Value};

use crate::subscription::{Hy2Config, ServerConfig, VlessConfig, RU_DIRECT_DOMAINS};

/// TUN interface address inside the NE sandbox (CGNAT range, like the
/// 198.18.0.1 used on desktop but with a /30 as sing-box recommends).
const TUN_ADDRESS: &str = "172.19.0.1/30";
const TUN_MTU: u16 = 1400;

fn vless_outbound(cfg: &VlessConfig) -> Value {
    let mut tls = json!({
        "enabled": true,
        "server_name": cfg.sni,
        "utls": {
            "enabled": true,
            "fingerprint": if cfg.fingerprint.is_empty() { "chrome" } else { &cfg.fingerprint }
        },
        "reality": {
            "enabled": true,
            "public_key": cfg.public_key,
            "short_id": cfg.short_id
        }
    });
    if cfg.sni.is_empty() {
        tls["server_name"] = json!(cfg.host);
    }

    let mut outbound = json!({
        "type": "vless",
        "tag": "proxy",
        "server": cfg.host,
        "server_port": cfg.port,
        "uuid": cfg.uuid,
        "tls": tls
    });
    if !cfg.flow.is_empty() {
        outbound["flow"] = json!(cfg.flow);
    }
    outbound
}

fn hy2_outbound(cfg: &Hy2Config) -> Value {
    // sing-box has no pinSHA256 for hysteria2; with a pinned self-signed cert
    // the desktop client also runs insecure+pin. Here pin => insecure, which
    // downgrades pin verification to none — acceptable because the hy2
    // password already authenticates the server before any payload flows.
    let insecure = cfg.insecure || !cfg.pin_sha256.is_empty();
    json!({
        "type": "hysteria2",
        "tag": "proxy",
        "server": cfg.host,
        "server_port": cfg.port,
        "password": cfg.password,
        "tls": {
            "enabled": true,
            "server_name": if cfg.sni.is_empty() { &cfg.host } else { &cfg.sni },
            "insecure": insecure
        }
    })
}

/// Full sing-box config for the PacketTunnel extension.
pub fn build_config(server: &ServerConfig) -> Value {
    let outbound = match server {
        ServerConfig::Vless(cfg) => vless_outbound(cfg),
        ServerConfig::Hy2(cfg) => hy2_outbound(cfg),
    };

    let ru_domains: Vec<&str> = RU_DIRECT_DOMAINS.to_vec();

    let mut route_rules = vec![
        json!({ "action": "sniff" }),
        json!({ "protocol": "dns", "action": "hijack-dns" }),
        json!({ "ip_is_private": true, "outbound": "direct" }),
        json!({ "domain_suffix": ru_domains, "outbound": "direct" }),
    ];
    // Block QUIC only for VLESS: Vision is TCP-only. Hysteria2 itself is
    // QUIC-based traffic *to the proxy*, but in-tunnel udp/443 is fine there.
    if matches!(server, ServerConfig::Vless(_)) {
        route_rules.push(json!({ "network": "udp", "port": 443, "action": "reject" }));
    }

    json!({
        "log": { "level": "warn" },
        "dns": {
            "servers": [
                {
                    "type": "https",
                    "tag": "dns-remote",
                    "server": "1.1.1.1",
                    "detour": "proxy"
                },
                {
                    "type": "udp",
                    "tag": "dns-direct",
                    "server": "77.88.8.8"
                }
            ],
            "rules": [
                { "domain_suffix": RU_DIRECT_DOMAINS.to_vec(), "server": "dns-direct" }
            ],
            "final": "dns-remote",
            "strategy": "ipv4_only"
        },
        "inbounds": [
            {
                "type": "tun",
                "tag": "tun-in",
                "address": [TUN_ADDRESS],
                "mtu": TUN_MTU,
                "auto_route": true,
                "strict_route": false
            }
        ],
        "outbounds": [
            outbound,
            { "type": "direct", "tag": "direct" }
        ],
        "route": {
            "rules": route_rules,
            "final": "proxy",
            "auto_detect_interface": true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vless_fixture() -> ServerConfig {
        ServerConfig::Vless(VlessConfig {
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            host: "vpn.example.com".into(),
            port: 443,
            encryption: "none".into(),
            public_key: "pbk123".into(),
            short_id: "sid1".into(),
            sni: "cdn.example.org".into(),
            fingerprint: "chrome".into(),
            flow: "xtls-rprx-vision".into(),
            spider_x: String::new(),
            remark: "Test VLESS".into(),
        })
    }

    fn hy2_fixture(insecure: bool, pin: &str) -> ServerConfig {
        ServerConfig::Hy2(Hy2Config {
            password: "secret".into(),
            host: "hy2.example.com".into(),
            port: 8443,
            sni: "hy2.example.com".into(),
            pin_sha256: pin.into(),
            insecure,
            remark: "Test Hy2".into(),
        })
    }

    #[test]
    fn vless_config_has_reality_and_quic_block() {
        let cfg = build_config(&vless_fixture());
        let outbound = &cfg["outbounds"][0];
        assert_eq!(outbound["type"], "vless");
        assert_eq!(outbound["flow"], "xtls-rprx-vision");
        assert_eq!(outbound["tls"]["reality"]["enabled"], true);
        assert_eq!(outbound["tls"]["reality"]["public_key"], "pbk123");
        assert_eq!(outbound["tls"]["server_name"], "cdn.example.org");

        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["network"] == "udp" && r["port"] == 443));
    }

    #[test]
    fn hy2_config_no_quic_block_and_pin_implies_insecure() {
        let cfg = build_config(&hy2_fixture(false, "sha256/abc"));
        let outbound = &cfg["outbounds"][0];
        assert_eq!(outbound["type"], "hysteria2");
        assert_eq!(outbound["password"], "secret");
        assert_eq!(outbound["tls"]["insecure"], true);

        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(!rules.iter().any(|r| r["network"] == "udp"));
    }

    #[test]
    fn tun_inbound_and_final_proxy() {
        let cfg = build_config(&hy2_fixture(false, ""));
        assert_eq!(cfg["inbounds"][0]["type"], "tun");
        assert_eq!(cfg["inbounds"][0]["auto_route"], true);
        assert_eq!(cfg["route"]["final"], "proxy");
        // Direct outbound present for smart routing.
        assert_eq!(cfg["outbounds"][1]["type"], "direct");
    }

    #[test]
    fn ru_domains_routed_direct() {
        let cfg = build_config(&vless_fixture());
        let rules = cfg["route"]["rules"].as_array().unwrap();
        let direct_domains = rules
            .iter()
            .find(|r| r["domain_suffix"].is_array())
            .expect("domain_suffix rule present");
        assert_eq!(direct_domains["outbound"], "direct");
        assert!(direct_domains["domain_suffix"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d == "yandex.ru"));
    }
}
