// src-tauri/src/xray_apple.rs
//
// The configuration of the Apple tunnel engine: Xray-core, loaded from
// LibXray.xcframework (scripts/build-libxray.sh) inside the PacketTunnel
// Network Extension.
//
// ── Why Xray, and why this is not a second generator ───────────────────────
// Until 28.09.2026 the extension ran sing-box. It has no XHTTP, so the UK, US
// and France locations (XHTTP only since 27.09) were missing on iPhone; it
// fails REALITY against Xray >= 26.9.8; and it is GPL-3.0, which the App Store
// build cannot carry. The desktop app already runs Xray, so the config here
// IS the desktop one (`subscription::build_xray_config_around`): the same
// VLESS outbound with REALITY, Vision or XHTTP, the same routing rules from
// the service's profile, the same custom rules. What differs is only what
// the platform forces:
//
//   • input: a `tun` inbound on the utun file descriptor the extension hands
//     over (`env["xray.tun.fd"]`, added in PacketTunnelProvider.swift, the
//     only place that knows the number), instead of a SOCKS port;
//   • DNS: the resolver policy the in-app data notice describes: Cloudflare
//     DoH through the tunnel, Russian and other "direct" names through Yandex
//     (77.88.8.8) outside it, and the node's own name through the system
//     resolver of the extension, which is the one that knows NAT64/DNS64;
//   • no geo files: `geoip:private` becomes its literal networks and other
//     `geoip:` categories are dropped. The extension lives in ~50 MB and a
//     geoip.dat alone is 20; the service sends its Russian networks inline
//     anyway (`?routing=inline`, subscription.rs);
//   • Hysteria2 runs in Xray too (its built-in client), with the certificate
//     pin enforced (`pinnedPeerCertSha256`), never "insecure";
//   • nothing that Xray 26.9 refuses: no `proxySettings`, no `allowInsecure`.

use std::net::IpAddr;

use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::subscription::{self, Hy2Config, RoutingRules, ServerConfig, TUNNEL_DNS};
use crate::tunnel_prefs::TunnelPrefs;

/// Where Russian and other "direct" names are resolved: Yandex, outside the
/// tunnel. The same resolver the sing-box build used and the data notice names.
const DIRECT_DNS: &str = "77.88.8.8";
/// Every other name: Cloudflare over HTTPS, through the node.
const TUNNEL_DOH: &str = "https://1.1.1.1/dns-query";

/// Addresses of the utun interface. The IPv4 /30 holds `TUNNEL_DNS` (.2), the
/// resolver address the system is given; the IPv6 ULA exists so that IPv6
/// can be routed INTO the tunnel at all, which is what keeps it from leaking
/// past it on a dual-stack network.
const TUN_IPV4: &str = "198.18.0.1";
const TUN_IPV4_MASK: &str = "255.255.255.252";
const TUN_IPV6: &str = "fd66:7076:706e::1";
const TUN_IPV6_PREFIX: u8 = 126;
/// 1500 keeps every packet inside one pooled Xray buffer (8 KiB) with room to
/// spare. The tunnel carries no outer encapsulation, so it has no reason to
/// be smaller.
const TUN_MTU: u16 = 1500;

/// What `geoip:private` stands for, as literal networks: the list Xray-core
/// 26.9.9 itself hard-codes for private addresses (common/geodata/consts.go).
/// Without geoip.dat the category would stop the core from starting.
const PRIVATE_NETWORKS: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.88.99.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/3",
    "::/127",
    "fc00::/7",
    "fe80::/10",
    "ff00::/8",
];

/// Networks the system keeps OUT of the tunnel altogether: the LAN, link-local
/// and multicast, so AirPlay, a printer or the router page never touch the
/// engine. Anything else private still reaches Xray and goes "direct" there.
const EXCLUDED_IPV4: &[(&str, &str)] = &[
    ("10.0.0.0", "255.0.0.0"),
    ("172.16.0.0", "255.240.0.0"),
    ("192.168.0.0", "255.255.0.0"),
    ("169.254.0.0", "255.255.0.0"),
    ("224.0.0.0", "240.0.0.0"),
    ("255.255.255.255", "255.255.255.255"),
];
const EXCLUDED_IPV6: &[(&str, u8)] = &[("fc00::", 7), ("fe80::", 10), ("ff00::", 8)];

/// `NEPacketTunnelNetworkSettings` for the extension, written next to the Xray
/// config so the resolver address the system is told and the rule that
/// catches it in Xray come from the same constants.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TunnelSettings {
    pub ipv4_address: String,
    pub ipv4_mask: String,
    pub ipv4_excluded: Vec<Ipv4Route>,
    pub ipv6_address: String,
    pub ipv6_prefix: u8,
    pub ipv6_excluded: Vec<Ipv6Route>,
    pub dns_servers: Vec<String>,
    pub mtu: u16,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Ipv4Route {
    pub address: String,
    pub mask: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Ipv6Route {
    pub address: String,
    pub prefix: u8,
}

pub fn tunnel_settings() -> TunnelSettings {
    TunnelSettings {
        ipv4_address: TUN_IPV4.to_string(),
        ipv4_mask: TUN_IPV4_MASK.to_string(),
        ipv4_excluded: EXCLUDED_IPV4
            .iter()
            .map(|(address, mask)| Ipv4Route { address: (*address).into(), mask: (*mask).into() })
            .collect(),
        ipv6_address: TUN_IPV6.to_string(),
        ipv6_prefix: TUN_IPV6_PREFIX,
        ipv6_excluded: EXCLUDED_IPV6
            .iter()
            .map(|(address, prefix)| Ipv6Route { address: (*address).into(), prefix: *prefix })
            .collect(),
        dns_servers: vec![TUNNEL_DNS.to_string()],
        mtu: TUN_MTU,
    }
}

/// The string VpnBridge.swift stores as the provider configuration and the
/// extension reads back: `{"version":1,"xray":{…},"tunnel":{…}}`.
pub fn provider_configuration(
    server: &ServerConfig,
    routing: Option<&RoutingRules>,
    prefs: &TunnelPrefs,
) -> Result<String> {
    let envelope = json!({
        "version": 1,
        "xray": build_config(server, routing, prefs)?,
        "tunnel": tunnel_settings(),
    });
    Ok(serde_json::to_string(&envelope)?)
}

/// The Xray config for `server`. Errors never name the node: they reach the
/// log, and the log is shared with support.
pub fn build_config(server: &ServerConfig, routing: Option<&RoutingRules>, prefs: &TunnelPrefs) -> Result<Value> {
    let mut config = match server {
        ServerConfig::Vless(cfg) => subscription::build_xray_config_with_routing_and_prefs(cfg, routing, prefs),
        ServerConfig::Hy2(cfg) => {
            // Fragmenting cuts a TLS ClientHello carried over TCP; Hysteria2's
            // handshake is QUIC, so the helper outbound would only sit idle.
            let prefs = TunnelPrefs { fragment: false, ..prefs.clone() };
            // No QUIC block: this node carries UDP itself.
            subscription::build_xray_config_around(hysteria_outbound(cfg)?, false, routing, &prefs)
        }
    };
    adapt(&mut config, server.host())?;
    Ok(config)
}

/// Hysteria2 through Xray's own client (proxy/hysteria).
fn hysteria_outbound(cfg: &Hy2Config) -> Result<Value> {
    let mut tls = Map::new();
    tls.insert("serverName".into(), json!(if cfg.sni.is_empty() { &cfg.host } else { &cfg.sni }));
    tls.insert("alpn".into(), json!(["h3"]));
    if !cfg.pin_sha256.is_empty() {
        tls.insert("pinnedPeerCertSha256".into(), json!(normalize_pins(&cfg.pin_sha256)?));
    } else if cfg.insecure {
        // A link that asks for no verification at all. Xray 26.9 no longer
        // has such a switch, and silently verifying against the system roots
        // would fail on our self-signed nodes anyway, so say what it is.
        bail!("the Hysteria2 entry disables certificate checks and carries no pin; the Apple engine verifies every certificate");
    }
    Ok(json!({
        "tag": "proxy",
        "protocol": "hysteria",
        "settings": { "version": 2, "address": cfg.host, "port": cfg.port },
        "streamSettings": {
            "network": "hysteria",
            "security": "tls",
            "tlsSettings": Value::Object(tls),
            "hysteriaSettings": { "version": 2, "auth": cfg.password }
        }
    }))
}

/// A pin as Hysteria2 links carry it (hex, with or without colons, or
/// `sha256/<base64>`, several separated by commas) in the form Xray checks:
/// lower-case hex of a 32-byte SHA-256, comma-separated.
fn normalize_pins(raw: &str) -> Result<String> {
    let pins = raw
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(normalize_pin)
        .collect::<Result<Vec<_>>>()?;
    if pins.is_empty() {
        bail!("the Hysteria2 certificate pin is empty");
    }
    Ok(pins.join(","))
}

fn normalize_pin(pin: &str) -> Result<String> {
    let invalid = || anyhow!("the Hysteria2 certificate pin is not a SHA-256 fingerprint");
    let bytes = if let Some(b64) = pin.strip_prefix("sha256/") {
        B64.decode(b64).map_err(|_| invalid())?
    } else {
        let hex: String = pin.chars().filter(|c| *c != ':').collect();
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        (0..32)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| invalid()))
            .collect::<Result<Vec<u8>>>()?
    };
    if bytes.len() != 32 {
        return Err(invalid());
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Turns the desktop config into the extension's one; see the header.
fn adapt(config: &mut Value, node_host: &str) -> Result<()> {
    let root = config.as_object_mut().ok_or_else(|| anyhow!("xray config is not an object"))?;

    root.insert("inbounds".into(), json!([tun_inbound()]));

    let rules = root
        .get_mut("routing")
        .and_then(|r| r.get_mut("rules"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("xray config has no routing rules"))?;
    inline_geoip(rules);
    route_direct_dns(rules);
    // Last: IPv6 that nothing sent "direct" would go to the node, and no node
    // has an IPv6 exit (fleet check 07.09.2026). Refusing it at once lets the
    // app fall back to IPv4 instead of waiting on a connection that the tun
    // stack has already "accepted" and that will never carry a byte.
    rules.push(json!({ "type": "field", "ip": ["::/0"], "outboundTag": "block" }));
    let (direct_names, proxy_names) = names_by_outbound(rules);

    root.insert("dns".into(), dns_section(node_host, &direct_names, &proxy_names));

    let outbounds = root
        .get_mut("outbounds")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("xray config has no outbounds"))?;
    for outbound in outbounds.iter_mut() {
        match outbound.get("tag").and_then(Value::as_str) {
            Some("proxy") => resolve_node_through_system(outbound)?,
            Some("dns-out") => *outbound = dns_outbound(),
            _ => {}
        }
    }
    Ok(())
}

/// The utun of the extension. Its fd arrives in `env["xray.tun.fd"]`, so the
/// name is only a label here: Xray creates no interface on iOS.
fn tun_inbound() -> Value {
    json!({
        "tag": "tun-in",
        "protocol": "tun",
        "settings": { "name": "utun", "mtu": TUN_MTU },
        // Route by the name the app asked for, connect to the address it got:
        // the address came from our own DNS a moment ago, and resolving the
        // name again somewhere else could only disagree with it.
        "sniffing": {
            "enabled": true,
            "destOverride": ["http", "tls", "quic"],
            "routeOnly": true
        }
    })
}

/// `geoip:private` → literal networks; any other `geoip:` → dropped (see the
/// header), and a rule left with no networks is dropped with it.
fn inline_geoip(rules: &mut Vec<Value>) {
    for rule in rules.iter_mut() {
        let Some(ips) = rule.get_mut("ip").and_then(Value::as_array_mut) else { continue };
        let mut inlined = Vec::with_capacity(ips.len());
        for ip in ips.iter() {
            match ip.as_str() {
                Some("geoip:private") => inlined.extend(PRIVATE_NETWORKS.iter().map(|n| json!(n))),
                Some(other) if other.starts_with("geoip:") || other.starts_with("ext:") => {}
                _ => inlined.push(ip.clone()),
            }
        }
        *ips = inlined;
    }
    rules.retain(|rule| !matches!(rule.get("ip").and_then(Value::as_array), Some(ips) if ips.is_empty()));
}

/// Xray's own queries to Yandex go out directly. Placed right before the
/// desktop rule that sends every query of the resolver (`dns-in`) through the
/// node, which would otherwise catch them first.
fn route_direct_dns(rules: &mut Vec<Value>) {
    let direct = json!({
        "type": "field",
        "inboundTag": ["dns-in"],
        "ip": [DIRECT_DNS],
        "outboundTag": "direct"
    });
    let at = rules
        .iter()
        .position(|r| r.get("inboundTag").is_some_and(|t| t == &json!(["dns-in"])))
        .unwrap_or(0);
    rules.insert(at, direct);
}

/// Domain patterns the routing sends direct, and those it forces through the
/// node, in rule order. Read from the finished rules so a name is resolved on
/// the same side of the tunnel it is routed to, whatever produced the rule
/// (the service's profile, the seed list, the person's own lists).
fn names_by_outbound(rules: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut direct = Vec::new();
    let mut proxy = Vec::new();
    for rule in rules {
        let Some(domains) = rule.get("domain").and_then(Value::as_array) else { continue };
        match rule.get("outboundTag").and_then(Value::as_str) {
            Some("direct") => direct.extend(domains.iter().cloned()),
            Some("proxy") => proxy.extend(domains.iter().cloned()),
            _ => {}
        }
    }
    (direct, proxy)
}

/// The resolver. Order matters: the first server whose `domains` match a name
/// answers it (`finalQuery`), so the person's "always through VPN" list beats
/// the direct list, as it does in routing.
///
/// `queryStrategy` is per server on purpose. Names the apps ask for get A
/// records only (no node has an IPv6 exit, see `adapt`). The node's own name
/// may need AAAA: on an IPv6-only network with NAT64 (App Review's, some
/// carriers') the system resolver synthesizes an IPv6 address for it, and
/// that is the only way to reach an IPv4 node from there. `UseSystem` asks
/// for the families the device can actually route, and the node outbound's
/// `UseIPv4v6` then takes IPv4 when there is one.
fn dns_section(node_host: &str, direct_names: &[Value], proxy_names: &[Value]) -> Value {
    let mut servers = Vec::new();
    if node_host.parse::<IpAddr>().is_err() {
        servers.push(json!({
            "address": "localhost",
            "domains": [format!("full:{node_host}")],
            "skipFallback": true,
            "finalQuery": true,
            "queryStrategy": "UseSystem"
        }));
    }
    if !proxy_names.is_empty() {
        servers.push(json!({
            "address": TUNNEL_DOH,
            "domains": proxy_names,
            "skipFallback": true,
            "finalQuery": true,
            "queryStrategy": "UseIPv4"
        }));
    }
    if !direct_names.is_empty() {
        servers.push(json!({
            "address": DIRECT_DNS,
            "port": 53,
            "domains": direct_names,
            "skipFallback": true,
            "finalQuery": true,
            "queryStrategy": "UseIPv4"
        }));
    }
    servers.push(json!({ "address": TUNNEL_DOH, "queryStrategy": "UseIPv4" }));
    json!({
        "servers": servers,
        // Global strategy stays wide; the servers above narrow it.
        "queryStrategy": "UseIP",
        "serveStale": true,
        "serveExpiredTTL": 3600,
        "tag": "dns-in"
    })
}

/// Name queries caught from the apps: A and AAAA are answered by the resolver
/// above; any other type gets an empty NOERROR at once. The desktop forwards
/// those through the node with `proxySettings`, which Xray 26.9 refuses, and
/// its measured alternative (`dialerProxy`) lost answers. An empty answer is
/// what most names have for HTTPS/SVCB records anyway, and unlike REFUSED it
/// never makes the system give up on the resolver (the desktop's lesson of
/// 27.09.2026).
fn dns_outbound() -> Value {
    json!({
        "tag": "dns-out",
        "protocol": "dns",
        "settings": {
            "rules": [
                { "action": "hijack", "qType": "1,28" },
                { "action": "return", "rCode": 0 }
            ]
        }
    })
}

/// The node's address goes through the resolver's `localhost` server (the
/// system resolver, see `dns_section`), IPv4 first and IPv6 when the network
/// has no IPv4. Merged into the existing `sockopt`, which may already carry
/// the fragmenting `dialerProxy`.
fn resolve_node_through_system(outbound: &mut Value) -> Result<()> {
    let stream = outbound
        .get_mut("streamSettings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow!("the node outbound has no streamSettings"))?;
    let sockopt = stream.entry("sockopt").or_insert_with(|| json!({}));
    let sockopt = sockopt.as_object_mut().ok_or_else(|| anyhow!("sockopt is not an object"))?;
    sockopt.insert("domainStrategy".into(), json!("UseIPv4v6"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::{VlessConfig, VlessTransport, XhttpMode, RU_DIRECT_DOMAINS};

    // Shapes of real entries with made-up values: 32-byte keys and pins, an
    // 8-byte short id, names under example.* only.
    const PUBLIC_KEY: &str = "TkpIxwXCNzLwTbT_fY3lPR3wVSFbrZXXS1yDI5ULWCQ";
    const PIN_HEX: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

    fn vless(transport: VlessTransport, flow: &str) -> ServerConfig {
        ServerConfig::Vless(VlessConfig {
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            host: "node.example.net".into(),
            port: 443,
            encryption: "none".into(),
            public_key: PUBLIC_KEY.into(),
            short_id: "ab12".into(),
            sni: "www.example.org".into(),
            fingerprint: "chrome".into(),
            flow: flow.into(),
            spider_x: "/".into(),
            remark: "Test".into(),
            transport,
        })
    }

    fn vision() -> ServerConfig {
        vless(VlessTransport::Tcp, "xtls-rprx-vision")
    }

    fn xhttp() -> ServerConfig {
        vless(
            VlessTransport::Xhttp { path: "/87588135c873".into(), mode: XhttpMode::StreamOne, host: None },
            "",
        )
    }

    fn hy2(pin: &str, insecure: bool) -> ServerConfig {
        ServerConfig::Hy2(Hy2Config {
            password: "secret".into(),
            host: "hy2.example.net".into(),
            port: 8443,
            sni: "hy2.example.net".into(),
            pin_sha256: pin.into(),
            insecure,
            remark: "Test Hy2".into(),
        })
    }

    fn build(server: &ServerConfig) -> Value {
        build_config(server, None, &TunnelPrefs::default()).expect("builds")
    }

    fn proxy(config: &Value) -> &Value {
        config["outbounds"]
            .as_array()
            .and_then(|o| o.iter().find(|o| o["tag"] == "proxy"))
            .expect("proxy outbound")
    }

    fn rules(config: &Value) -> &Vec<Value> {
        config["routing"]["rules"].as_array().expect("rules")
    }

    fn servers(config: &Value) -> &Vec<Value> {
        config["dns"]["servers"].as_array().expect("dns servers")
    }

    fn text(config: &Value) -> String {
        serde_json::to_string(config).expect("json")
    }

    #[test]
    fn vision_over_reality_keeps_the_desktop_outbound() {
        let config = build(&vision());
        let out = proxy(&config);
        assert_eq!(out["protocol"], "vless");
        assert_eq!(out["settings"]["vnext"][0]["users"][0]["flow"], "xtls-rprx-vision");
        assert_eq!(out["streamSettings"]["network"], "tcp");
        assert_eq!(out["streamSettings"]["security"], "reality");
        assert_eq!(out["streamSettings"]["realitySettings"]["publicKey"], PUBLIC_KEY);
        assert_eq!(out["streamSettings"]["realitySettings"]["serverName"], "www.example.org");
        // QUIC is refused for a TCP-only node, first thing.
        assert_eq!(rules(&config)[0]["network"], "udp");
        assert_eq!(rules(&config)[0]["outboundTag"], "block");
    }

    #[test]
    fn xhttp_stream_one_is_built_as_xhttp_without_vision() {
        let config = build(&xhttp());
        let out = proxy(&config);
        assert_eq!(out["streamSettings"]["network"], "xhttp");
        assert_eq!(out["streamSettings"]["xhttpSettings"]["mode"], "stream-one");
        assert_eq!(out["streamSettings"]["xhttpSettings"]["path"], "/87588135c873");
        assert!(out["settings"]["vnext"][0]["users"][0].get("flow").is_none());
        assert_eq!(out["streamSettings"]["security"], "reality");
    }

    /// REALITY against Xray >= 26.9.8 needs the client to offer an ML-KEM key
    /// share, which Xray's REALITY client does with a browser fingerprint.
    /// Nothing here may swap that client out or turn the fingerprint off.
    #[test]
    fn reality_goes_through_xrays_own_client_with_a_browser_fingerprint() {
        for server in [vision(), xhttp()] {
            let config = build(&server);
            let reality = &proxy(&config)["streamSettings"]["realitySettings"];
            assert_eq!(reality["fingerprint"], "chrome");
            assert_eq!(reality["shortId"], "ab12");
            let all = text(&config);
            for forbidden in ["\"unsafe\"", "hellogolang", "allowInsecure"] {
                assert!(!all.contains(forbidden), "{forbidden} in config");
            }
        }
    }

    #[test]
    fn hysteria2_with_a_pin_is_verified_by_the_pin() {
        let config = build(&hy2(PIN_HEX, true));
        let out = proxy(&config);
        assert_eq!(out["protocol"], "hysteria");
        assert_eq!(out["settings"]["version"], 2);
        assert_eq!(out["settings"]["address"], "hy2.example.net");
        assert_eq!(out["settings"]["port"], 8443);
        assert_eq!(out["streamSettings"]["network"], "hysteria");
        assert_eq!(out["streamSettings"]["hysteriaSettings"]["auth"], "secret");
        let tls = &out["streamSettings"]["tlsSettings"];
        assert_eq!(tls["pinnedPeerCertSha256"], PIN_HEX);
        assert_eq!(tls["serverName"], "hy2.example.net");
        assert_eq!(tls["alpn"], json!(["h3"]));
        assert!(!text(&config).contains("allowInsecure"));
        // It carries UDP itself: no QUIC block.
        assert!(!rules(&config).iter().any(|r| r["network"] == "udp"));
    }

    #[test]
    fn every_pin_spelling_becomes_the_same_hex() {
        let colons: String = PIN_HEX
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).expect("ascii").to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(":");
        let bytes: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&PIN_HEX[i * 2..i * 2 + 2], 16).expect("hex")).collect();
        let b64 = format!("sha256/{}", B64.encode(&bytes));
        for spelling in [PIN_HEX.to_string(), PIN_HEX.to_ascii_uppercase(), colons, b64] {
            assert_eq!(normalize_pins(&spelling).expect("pin"), PIN_HEX, "{spelling}");
        }
        assert_eq!(
            normalize_pins(&format!("{PIN_HEX}, {PIN_HEX}")).expect("two pins"),
            format!("{PIN_HEX},{PIN_HEX}")
        );
    }

    #[test]
    fn a_broken_pin_refuses_to_connect_rather_than_trust_anything() {
        for bad in ["AA:BB", "zz", &PIN_HEX[..62], "sha256/!!!", "sha256/AAAA", ","] {
            assert!(build_config(&hy2(bad, true), None, &TunnelPrefs::default()).is_err(), "{bad}");
        }
    }

    #[test]
    fn insecure_without_a_pin_is_refused() {
        let err = build_config(&hy2("", true), None, &TunnelPrefs::default()).expect_err("refused");
        assert!(!err.to_string().contains("example"), "the error must not name the node");
    }

    #[test]
    fn hysteria2_without_pin_or_insecure_uses_normal_verification() {
        let config = build(&hy2("", false));
        let tls = &proxy(&config)["streamSettings"]["tlsSettings"];
        assert!(tls.get("pinnedPeerCertSha256").is_none());
        assert_eq!(tls["serverName"], "hy2.example.net");
    }

    #[test]
    fn the_tun_inbound_replaces_the_socks_port() {
        let config = build(&vision());
        let inbounds = config["inbounds"].as_array().expect("inbounds");
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["protocol"], "tun");
        assert_eq!(inbounds[0]["settings"]["mtu"], TUN_MTU);
        assert_eq!(inbounds[0]["sniffing"]["routeOnly"], true);
        // The fd is the extension's to add; a stale one here would be fatal.
        assert!(config.get("env").is_none());
        assert!(!text(&config).contains("10808"));
    }

    #[test]
    fn no_geo_file_is_referenced_and_private_networks_are_inline() {
        let routing = RoutingRules {
            direct_domains: vec!["domain:ya.ru".into(), "full:gosuslugi.ru".into()],
            direct_ips: vec!["geoip:ru".into(), "5.8.0.0/16".into(), "geoip:private".into()],
            domain_strategy: "IPIfNonMatch".into(),
        };
        for server in [vision(), hy2(PIN_HEX, true)] {
            for routing in [None, Some(&routing)] {
                let config = build_config(&server, routing, &TunnelPrefs::default()).expect("builds");
                let all = text(&config);
                assert!(!all.contains("geoip:") && !all.contains("geosite:") && !all.contains("ext:"), "{all}");
                let private = rules(&config)
                    .iter()
                    .find(|r| r["outboundTag"] == "direct" && r["ip"].as_array().is_some_and(|ips| ips.contains(&json!("192.168.0.0/16"))))
                    .expect("private networks direct");
                assert!(private["ip"].as_array().expect("ips").contains(&json!("fe80::/10")));
            }
            // The service's own networks survive, next to what the category meant.
            let config = build_config(&server, Some(&routing), &TunnelPrefs::default()).expect("builds");
            let service = rules(&config)
                .iter()
                .find(|r| r["ip"][0] == "5.8.0.0/16")
                .expect("the service's networks");
            assert_eq!(service["outboundTag"], "direct");
            assert_eq!(service["ip"].as_array().expect("ips").len(), 1 + PRIVATE_NETWORKS.len());
            assert_eq!(config["routing"]["domainStrategy"], "IPIfNonMatch");
            // A rule whose only network was a dropped category goes with it.
            let only_ru = RoutingRules { direct_ips: vec!["geoip:ru".into()], ..routing.clone() };
            let config = build_config(&server, Some(&only_ru), &TunnelPrefs::default()).expect("builds");
            assert!(!rules(&config).iter().any(|r| r["ip"].as_array().is_some_and(|ips| ips.is_empty())));
        }
    }

    #[test]
    fn dns_follows_the_notice_doh_through_the_node_and_yandex_for_direct_names() {
        let config = build(&vision());
        let servers = servers(&config);
        // The node's own name: the system resolver (NAT64-aware), both families.
        assert_eq!(servers[0]["address"], "localhost");
        assert_eq!(servers[0]["domains"], json!(["full:node.example.net"]));
        assert_eq!(servers[0]["queryStrategy"], "UseSystem");
        // Russian names: Yandex, and its queries leave directly.
        let yandex = servers.iter().find(|s| s["address"] == DIRECT_DNS).expect("yandex");
        let names = yandex["domains"].as_array().expect("domains");
        assert!(names.contains(&json!("domain:yandex.ru")));
        assert_eq!(names.len(), RU_DIRECT_DOMAINS.len());
        // Everything else: Cloudflare DoH, IPv4 answers only.
        let last = servers.last().expect("default server");
        assert_eq!(last["address"], TUNNEL_DOH);
        assert_eq!(last["queryStrategy"], "UseIPv4");
        assert!(last.get("domains").is_none());
        assert_eq!(config["dns"]["tag"], "dns-in");

        let rules = rules(&config);
        let direct_dns = rules.iter().position(|r| r["ip"] == json!([DIRECT_DNS])).expect("yandex direct");
        let via_node = rules
            .iter()
            .position(|r| r["inboundTag"] == json!(["dns-in"]) && r["outboundTag"] == "proxy")
            .expect("resolver through the node");
        assert!(direct_dns < via_node);
        assert_eq!(rules[direct_dns]["outboundTag"], "direct");
    }

    #[test]
    fn the_persons_own_lists_decide_where_a_name_is_resolved() {
        let prefs = TunnelPrefs {
            direct_domains: vec!["domain:direct.example.com".into()],
            proxy_domains: vec!["full:vpn.example.ru".into()],
            ..TunnelPrefs::default()
        };
        let config = build_config(&vision(), None, &prefs).expect("builds");
        let servers = servers(&config);
        let doh_first = servers.iter().position(|s| s["domains"] == json!(["full:vpn.example.ru"])).expect("own DoH");
        let yandex = servers.iter().position(|s| s["address"] == DIRECT_DNS).expect("yandex");
        assert!(doh_first < yandex, "\"always through VPN\" must win over the direct list");
        assert_eq!(servers[doh_first]["address"], TUNNEL_DOH);
        assert!(servers[yandex]["domains"].as_array().expect("names").contains(&json!("domain:direct.example.com")));
    }

    #[test]
    fn the_global_profile_resolves_everything_through_the_node() {
        let config = build_config(&vision(), Some(&RoutingRules::global()), &TunnelPrefs::default()).expect("builds");
        assert!(!servers(&config).iter().any(|s| s["address"] == DIRECT_DNS));
    }

    #[test]
    fn an_address_literal_node_needs_no_resolver_entry() {
        let mut server = vision();
        if let ServerConfig::Vless(cfg) = &mut server {
            cfg.host = "192.0.2.10".into();
        }
        let config = build(&server);
        assert!(!servers(&config).iter().any(|s| s["address"] == "localhost"));
    }

    #[test]
    fn the_node_is_resolved_ipv4_first_with_ipv6_as_fallback() {
        for server in [vision(), xhttp(), hy2(PIN_HEX, true)] {
            let config = build(&server);
            assert_eq!(proxy(&config)["streamSettings"]["sockopt"]["domainStrategy"], "UseIPv4v6");
        }
        let prefs = TunnelPrefs { fragment: true, ..TunnelPrefs::default() };
        let config = build_config(&vision(), None, &prefs).expect("builds");
        let sockopt = &proxy(&config)["streamSettings"]["sockopt"];
        assert_eq!(sockopt["dialerProxy"], "fragment");
        assert_eq!(sockopt["domainStrategy"], "UseIPv4v6");
    }

    #[test]
    fn hysteria2_leaves_the_fragmenter_out() {
        let prefs = TunnelPrefs { fragment: true, ..TunnelPrefs::default() };
        let config = build_config(&hy2(PIN_HEX, true), None, &prefs).expect("builds");
        assert!(!config["outbounds"].as_array().expect("outbounds").iter().any(|o| o["tag"] == "fragment"));
    }

    #[test]
    fn nothing_xray_26_9_refuses_is_left_in() {
        for server in [vision(), xhttp(), hy2(PIN_HEX, true)] {
            let config = build(&server);
            let all = text(&config);
            assert!(!all.contains("proxySettings"), "removed in Xray 26.9");
            assert!(!all.contains("nonIPQuery"), "legacy DNS outbound form");
            let dns_out = config["outbounds"]
                .as_array()
                .and_then(|o| o.iter().find(|o| o["tag"] == "dns-out"))
                .expect("dns-out");
            assert_eq!(dns_out["settings"]["rules"][0]["qType"], "1,28");
        }
    }

    #[test]
    fn ipv6_that_would_reach_the_node_is_refused_last() {
        let config = build(&vision());
        let last = rules(&config).last().expect("rules");
        assert_eq!(last["ip"], json!(["::/0"]));
        assert_eq!(last["outboundTag"], "block");
    }

    #[test]
    fn the_system_is_given_the_resolver_that_xray_catches() {
        let settings = tunnel_settings();
        assert_eq!(settings.dns_servers, vec![TUNNEL_DNS.to_string()]);
        let config = build(&vision());
        assert!(rules(&config).iter().any(|r| r["ip"] == json!([TUNNEL_DNS]) && r["outboundTag"] == "dns-out"));
        assert_eq!(settings.mtu, TUN_MTU);
        // The resolver address sits inside the tunnel's own /30 and outside
        // every excluded network.
        assert!(!settings.ipv4_excluded.iter().any(|r| r.address.starts_with("198.18.")));
    }

    #[test]
    fn the_envelope_carries_both_halves() {
        let raw = provider_configuration(&xhttp(), None, &TunnelPrefs::default()).expect("envelope");
        let envelope: Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(envelope["version"], 1);
        assert_eq!(envelope["xray"]["inbounds"][0]["protocol"], "tun");
        assert_eq!(envelope["tunnel"]["ipv4Address"], TUN_IPV4);
        assert_eq!(envelope["tunnel"]["ipv6Prefix"], TUN_IPV6_PREFIX);
        assert_eq!(envelope["tunnel"]["ipv4Excluded"][0]["mask"], "255.0.0.0");
        assert_eq!(envelope["tunnel"]["ipv6Excluded"][0]["prefix"], 7);
    }
}
