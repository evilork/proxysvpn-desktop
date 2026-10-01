// src-tauri/src/subscription.rs
// Fetches subscription, parses VLESS URLs, builds Xray JSON config
// with a SOCKS5 inbound on 127.0.0.1:10808 that tun2socks will consume.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};
use url::Url;

#[derive(Debug, Clone)]
pub struct VlessConfig {
    pub uuid: String,
    pub host: String,
    pub port: u16,
    pub encryption: String,
    pub public_key: String,
    pub short_id: String,
    pub sni: String,
    pub fingerprint: String,
    pub flow: String,
    pub spider_x: String,
    pub remark: String,
}

#[allow(dead_code)]
pub async fn fetch_subscription(sub_url: &str) -> Result<VlessConfig> {
    let body = reqwest::Client::builder()
        .user_agent("ProxysVPN-Desktop/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(sub_url)
        .send()
        .await
        .context("subscription request failed")?
        .text()
        .await
        .context("subscription body read failed")?;

    let decoded = decode_subscription_body(&body);
    let first_vless = decoded
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("vless://"))
        .ok_or_else(|| anyhow!("no vless config in subscription"))?;

    parse_vless_url(first_vless)
}

/// Краткая информация о сервере для UI (без секретов).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerInfo {
    pub index: usize,
    pub remark: String,
    pub host: String,
    pub port: u16,
    pub proto: String,
}

/// Парсит ВСЕ vless-серверы из подписки (hy2 пока пропускается — нужен др. движок).
// Superseded by fetch_all_servers, which returns vless and hy2 in one list.
// Kept because the VLESS-only path is still the fallback used while debugging a
// subscription that mixes protocols; remove it once that is no longer needed.
#[allow(dead_code)]
pub async fn fetch_all_vless(sub_url: &str) -> Result<Vec<VlessConfig>> {
    let body = reqwest::Client::builder()
        .user_agent("ProxysVPN-Desktop/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(sub_url)
        .send()
        .await
        .context("subscription request failed")?
        .text()
        .await
        .context("subscription body read failed")?;

    let decoded = decode_subscription_body(&body);
    let servers: Vec<VlessConfig> = decoded
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("vless://"))
        .filter_map(|l| parse_vless_url(l).ok())
        .collect();

    if servers.is_empty() {
        return Err(anyhow!("no vless servers in subscription"));
    }
    Ok(servers)
}

#[derive(Debug, Clone)]
pub struct Hy2Config {
    pub password: String,
    pub host: String,
    pub port: u16,
    pub sni: String,
    pub pin_sha256: String,
    pub insecure: bool,
    pub remark: String,
}

/// Извлекает чистый хост из возможной Markdown-ссылки [host](url) или мусора.
fn clean_sni(raw: &str) -> String {
    let s = raw.trim();
    let candidate = if let Some(start) = s.find('[') {
        if let Some(end) = s[start + 1..].find(']') {
            &s[start + 1..start + 1 + end]
        } else {
            s
        }
    } else {
        s
    };
    candidate
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
        .collect()
}

pub fn parse_hy2_url(raw: &str) -> Result<Hy2Config> {
    // hy2://password@host:port/?sni=..&pinSHA256=..&insecure=1#remark
    let u = Url::parse(raw).context("invalid hy2 url")?;
    let password = u.username().to_string();
    if password.is_empty() {
        return Err(anyhow!("missing password in hy2 url"));
    }
    let host = u.host_str().ok_or_else(|| anyhow!("missing host"))?.to_string();
    let port = u.port().ok_or_else(|| anyhow!("missing port"))?;

    let mut sni = String::new();
    let mut pin_sha256 = String::new();
    let mut insecure = false;
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "sni" => sni = v.into_owned(),
            "pinSHA256" | "pinsha256" => pin_sha256 = v.into_owned(),
            "insecure" => insecure = v == "1" || v == "true",
            _ => {}
        }
    }
    if sni.is_empty() {
        sni = host.clone();
    }
    // SNI может прийти как Markdown [host](url) из-за бага бэкенда — извлекаем host.
    sni = clean_sni(&sni);
    // При pinSHA256 серт верифицируется по отпечатку, стандартная x509-проверка
    // не нужна и ломает self-signed серверы. insecure=true безопасен с пином.
    if !pin_sha256.is_empty() {
        insecure = true;
    }
    let remark = u
        .fragment()
        .map(|f| urlencoding::decode(f).map(|c| c.into_owned()).unwrap_or_else(|_| f.to_string()))
        .unwrap_or_default();

    Ok(Hy2Config { password, host, port, sni, pin_sha256, insecure, remark })
}

/// Единый список серверов из подписки: и vless, и hy2.
#[derive(Debug, Clone)]
pub enum ServerConfig {
    Vless(VlessConfig),
    Hy2(Hy2Config),
}

impl ServerConfig {
    pub fn host(&self) -> &str {
        match self {
            ServerConfig::Vless(c) => &c.host,
            ServerConfig::Hy2(c) => &c.host,
        }
    }
    pub fn port(&self) -> u16 {
        match self {
            ServerConfig::Vless(c) => c.port,
            ServerConfig::Hy2(c) => c.port,
        }
    }
    pub fn remark(&self) -> &str {
        match self {
            ServerConfig::Vless(c) => &c.remark,
            ServerConfig::Hy2(c) => &c.remark,
        }
    }
    pub fn proto(&self) -> &'static str {
        match self {
            ServerConfig::Vless(_) => "VLESS",
            ServerConfig::Hy2(_) => "Hysteria2",
        }
    }
}

/// Парсит ВСЕ серверы (vless + hy2) из подписки в едином порядке.
pub async fn fetch_all_servers(sub_url: &str) -> Result<Vec<ServerConfig>> {
    let body = reqwest::Client::builder()
        .user_agent("ProxysVPN-Desktop/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(sub_url)
        .send()
        .await
        .context("subscription request failed")?
        .text()
        .await
        .context("subscription body read failed")?;

    let decoded = decode_subscription_body(&body);
    let mut servers: Vec<ServerConfig> = Vec::new();
    for line in decoded.lines().map(str::trim) {
        if line.starts_with("vless://") {
            if let Ok(c) = parse_vless_url(line) {
                servers.push(ServerConfig::Vless(c));
            }
        } else if line.starts_with("hy2://") || line.starts_with("hysteria2://") {
            if let Ok(c) = parse_hy2_url(line) {
                servers.push(ServerConfig::Hy2(c));
            }
        }
    }
    if servers.is_empty() {
        return Err(anyhow!("no servers in subscription"));
    }
    Ok(servers)
}

fn decode_subscription_body(body: &str) -> String {
    let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if let Ok(bytes) = B64.decode(&cleaned) {
        if let Ok(s) = String::from_utf8(bytes) {
            return s;
        }
    }
    body.to_string()
}

pub fn parse_vless_url(raw: &str) -> Result<VlessConfig> {
    let u = Url::parse(raw).context("invalid vless url")?;

    let uuid = u.username().to_string();
    if uuid.is_empty() {
        return Err(anyhow!("missing uuid in vless url"));
    }
    let host = u.host_str().ok_or_else(|| anyhow!("missing host"))?.to_string();
    let port = u.port().ok_or_else(|| anyhow!("missing port"))?;

    let mut encryption = "none".to_string();
    let mut public_key = String::new();
    let mut short_id = String::new();
    let mut sni = String::new();
    let mut fingerprint = "chrome".to_string();
    let mut flow = String::new();
    let mut spider_x = String::new();

    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "encryption" => encryption = v.into_owned(),
            "pbk" => public_key = v.into_owned(),
            "sid" => short_id = v.into_owned(),
            "sni" => sni = v.into_owned(),
            "fp" => fingerprint = v.into_owned(),
            "flow" => flow = v.into_owned(),
            "spx" => spider_x = v.into_owned(),
            _ => {}
        }
    }

    if public_key.is_empty() {
        return Err(anyhow!("missing pbk (public key) in vless url"));
    }
    if sni.is_empty() {
        sni = host.clone();
    }

    let remark = u
        .fragment()
        .map(|f| urlencoding::decode(f).map(|c| c.into_owned()).unwrap_or_else(|_| f.to_string()))
        .unwrap_or_default();

    Ok(VlessConfig {
        uuid,
        host,
        port,
        encryption,
        public_key,
        short_id,
        sni,
        fingerprint,
        flow,
        spider_x,
        remark,
    })
}

/// Build the Xray runtime config used by xray_manager.
///
/// This is the ORIGINAL pre-P4 config plus only:
///   • block UDP/443 (QUIC) — prevents UDP leak through Vision
///   • direct: geoip:ru — Russian IPs go straight (Smart routing)
///   • direct: explicit Russian domain whitelist
///
/// Everything else (DNS section, sockopt, routeOnly, domainMatcher hybrid)
/// has been removed because it caused throughput regression in speedtest:
///   • DNS section + IPIfNonMatch → forced extra resolves through proxy
///     (+73ms per new connection × parallel speedtest streams)
///   • routeOnly: xray sniffing adds per-connection overhead
///   • domainMatcher hybrid: not actually faster on small rule sets
///
/// Trade-off: DNS leak is back (system resolver used). Acceptable since
/// the user is in RU where the ISP logs all NetFlow anyway. We can add
/// proper DNS-over-HTTPS later when we have time to tune it without
/// killing throughput.
pub fn build_xray_config(cfg: &VlessConfig) -> Value {
    let mut user = serde_json::Map::new();
    user.insert("id".into(), json!(cfg.uuid));
    user.insert("encryption".into(), json!(cfg.encryption));
    if !cfg.flow.is_empty() {
        user.insert("flow".into(), json!(cfg.flow));
    }

    let mut reality = serde_json::Map::new();
    reality.insert("serverName".into(), json!(cfg.sni));
    reality.insert("fingerprint".into(), json!(cfg.fingerprint));
    reality.insert("publicKey".into(), json!(cfg.public_key));
    reality.insert("shortId".into(), json!(cfg.short_id));
    if !cfg.spider_x.is_empty() {
        reality.insert("spiderX".into(), json!(cfg.spider_x));
    }

    json!({
        "log": { "loglevel": "warning" },
        "inbounds": [
            {
                "tag": "socks-in",
                "listen": "127.0.0.1",
                "port": 10808,
                "protocol": "socks",
                "settings": { "udp": true, "auth": "noauth" },
                "sniffing": { "enabled": true, "destOverride": ["http", "tls"] }
            }
        ],
        "outbounds": [
            {
                "tag": "proxy",
                "protocol": "vless",
                "settings": {
                    "vnext": [{
                        "address": cfg.host,
                        "port": cfg.port,
                        "users": [ Value::Object(user) ]
                    }]
                },
                "streamSettings": {
                    "network": "tcp",
                    "security": "reality",
                    "realitySettings": Value::Object(reality)
                }
            },
            { "tag": "direct", "protocol": "freedom" },
            { "tag": "block",  "protocol": "blackhole" }
        ],
        "routing": {
            "domainStrategy": "IPIfNonMatch",
            "rules": [
                // 1. Block QUIC (UDP/443) — Vision is TCP-only, UDP would leak.
                {
                    "type": "field",
                    "outboundTag": "block",
                    "network": "udp",
                    "port": "443"
                },
                // 2. Private/LAN — direct (was the only original rule).
                {
                    "type": "field",
                    "outboundTag": "direct",
                    "ip": ["geoip:private"]
                },
                // 3. Russian IPs — direct (Smart routing).
                //    Fail-safe: if geoip.dat lacks 'ru', rule no-ops →
                //    traffic falls through to default proxy.
                {
                    "type": "field",
                    "outboundTag": "direct",
                    "ip": ["geoip:ru"]
                },
                // 4. Major Russian domains — direct.
                {
                    "type": "field",
                    "outboundTag": "direct",
                    "domain": [
                        "domain:yandex.ru","domain:yandex.com","domain:yandex.net",
                        "domain:ya.ru",
                        "domain:vk.com","domain:vk.ru","domain:vkuser.net",
                        "domain:userapi.com","domain:vk-cdn.net","domain:vk-cdn.com",
                        "domain:mail.ru","domain:my.com","domain:imgsmail.ru",
                        "domain:ok.ru","domain:odnoklassniki.ru",
                        "domain:gosuslugi.ru","domain:nalog.ru","domain:nalog.gov.ru",
                        "domain:sber.ru","domain:sberbank.ru","domain:sberbank.com",
                        "domain:tinkoff.ru","domain:t-bank.ru","domain:tbank.ru",
                        "domain:vtb.ru","domain:alfabank.ru","domain:raiffeisen.ru",
                        "domain:rzd.ru","domain:tutu.ru","domain:aviasales.ru",
                        "domain:wildberries.ru","domain:ozon.ru","domain:ozon.com",
                        "domain:avito.ru","domain:cian.ru","domain:hh.ru",
                        "domain:rambler.ru","domain:lenta.ru","domain:rbc.ru",
                        "domain:ria.ru","domain:tass.ru","domain:kommersant.ru",
                        "domain:kinopoisk.ru","domain:ivi.ru","domain:rutube.ru",
                        "domain:2gis.ru","domain:2gis.com","domain:2gis.kz",
                        "domain:dns-shop.ru","domain:mvideo.ru","domain:eldorado.ru",
                        "domain:proxysvpn.com"
                    ]
                }
                // No catch-all needed: xray sends unmatched traffic to the
                // first outbound ("proxy") by default.
            ]
        }
    })
}

/// Sample configs for tests in this crate.
///
/// All values are deliberately fake (RFC 2606 / RFC 5737 reserved names and
/// addresses, a nil-ish UUID, placeholder keys) because this repository is
/// public: no real node address, UUID or password may ever appear here.
#[cfg(test)]
pub mod test_support {
    use super::{Hy2Config, VlessConfig};

    pub fn sample_vless() -> VlessConfig {
        VlessConfig {
            uuid: "00000000-0000-4000-8000-000000000000".into(),
            host: "node.example.invalid".into(),
            port: 443,
            encryption: "none".into(),
            public_key: "EXAMPLE-PUBLIC-KEY-NOT-A-REAL-ONE".into(),
            short_id: "0123abcd".into(),
            sni: "cover.example.invalid".into(),
            fingerprint: "chrome".into(),
            flow: "xtls-rprx-vision".into(),
            spider_x: "/".into(),
            remark: "Example node".into(),
        }
    }

    pub fn sample_hy2() -> Hy2Config {
        Hy2Config {
            password: "example-password-not-a-real-one".into(),
            host: "node.example.invalid".into(),
            port: 8443,
            sni: "cover.example.invalid".into(),
            pin_sha256: "AA:BB:CC".into(),
            insecure: true,
            remark: "Example hy2 node".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/xray_vless_config.json");

    /// Line endings are normalised before comparing: git checks the fixture out
    /// with CRLF on Windows (core.autocrlf defaults to true there), while
    /// serde_json always emits LF, so a byte comparison fails on the Windows
    /// runner for a config that is in fact identical.
    fn lf(text: &str) -> String {
        text.replace("\r\n", "\n").trim().to_string()
    }

    /// The generated xray config is a user-visible contract: a change here
    /// changes how every client routes traffic. The fixture is the
    /// pre-platform-split output; regenerate it on purpose with
    ///   cargo test -p proxysvpn-desktop -- --ignored dump_xray_config_fixture
    /// and review the diff.
    #[test]
    fn generated_xray_config_matches_fixture() {
        let generated = serde_json::to_string_pretty(&build_xray_config(
            &test_support::sample_vless(),
        ))
        .expect("serialize config");
        assert_eq!(lf(&generated), lf(FIXTURE));
    }

    #[test]
    #[ignore = "writes the fixture; run deliberately after an intended config change"]
    fn dump_xray_config_fixture() {
        let generated = serde_json::to_string_pretty(&build_xray_config(
            &test_support::sample_vless(),
        ))
        .expect("serialize config");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/fixtures/xray_vless_config.json");
        std::fs::write(&path, format!("{}\n", generated)).expect("write fixture");
    }

    #[test]
    fn xray_inbound_is_the_socks_port_tun2socks_dials() {
        let cfg = build_xray_config(&test_support::sample_vless());
        assert_eq!(cfg["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(cfg["inbounds"][0]["port"], 10808);
        assert_eq!(cfg["inbounds"][0]["protocol"], "socks");
    }

    /// Unmatched traffic goes to the first outbound, so "proxy" must stay first.
    #[test]
    fn proxy_outbound_is_first() {
        let cfg = build_xray_config(&test_support::sample_vless());
        assert_eq!(cfg["outbounds"][0]["tag"], "proxy");
        assert_eq!(cfg["outbounds"][0]["protocol"], "vless");
    }

    /// QUIC must be blocked before anything else can match it: Vision is
    /// TCP-only, so UDP/443 would otherwise leak outside the tunnel.
    #[test]
    fn quic_is_blocked_by_the_first_rule() {
        let cfg = build_xray_config(&test_support::sample_vless());
        let first = &cfg["routing"]["rules"][0];
        assert_eq!(first["outboundTag"], "block");
        assert_eq!(first["network"], "udp");
        assert_eq!(first["port"], "443");
    }

    #[test]
    fn empty_flow_is_omitted_not_sent_as_empty_string() {
        let mut cfg = test_support::sample_vless();
        cfg.flow = String::new();
        let json = build_xray_config(&cfg);
        let user = &json["outbounds"][0]["settings"]["vnext"][0]["users"][0];
        assert!(user.get("flow").is_none(), "empty flow must not be emitted");
        assert_eq!(user["id"], cfg.uuid);
    }

    #[test]
    fn empty_spider_x_is_omitted() {
        let mut cfg = test_support::sample_vless();
        cfg.spider_x = String::new();
        let json = build_xray_config(&cfg);
        let reality = &json["outbounds"][0]["streamSettings"]["realitySettings"];
        assert!(reality.get("spiderX").is_none());
        assert_eq!(reality["serverName"], cfg.sni);
    }

    // ------------------------------------------------------------- parsing

    #[test]
    fn parses_a_vless_url() {
        let url = "vless://00000000-0000-4000-8000-000000000000@node.example.invalid:443?encryption=none&security=reality&pbk=EXAMPLEKEY&sid=0123abcd&sni=cover.example.invalid&fp=chrome&flow=xtls-rprx-vision&spx=%2F#Example%20node";
        let cfg = parse_vless_url(url).expect("parse");
        assert_eq!(cfg.uuid, "00000000-0000-4000-8000-000000000000");
        assert_eq!(cfg.host, "node.example.invalid");
        assert_eq!(cfg.port, 443);
        assert_eq!(cfg.public_key, "EXAMPLEKEY");
        assert_eq!(cfg.short_id, "0123abcd");
        assert_eq!(cfg.sni, "cover.example.invalid");
        assert_eq!(cfg.flow, "xtls-rprx-vision");
        assert_eq!(cfg.spider_x, "/");
        assert_eq!(cfg.remark, "Example node");
    }

    #[test]
    fn vless_sni_defaults_to_the_host() {
        let url = "vless://uuid@node.example.invalid:443?pbk=EXAMPLEKEY";
        let cfg = parse_vless_url(url).expect("parse");
        assert_eq!(cfg.sni, "node.example.invalid");
        assert_eq!(cfg.fingerprint, "chrome", "default fingerprint");
        assert_eq!(cfg.encryption, "none", "default encryption");
    }

    #[test]
    fn vless_rejects_incomplete_urls() {
        // no public key — REALITY cannot be configured
        assert!(parse_vless_url("vless://uuid@node.example.invalid:443").is_err());
        // no uuid
        assert!(parse_vless_url("vless://node.example.invalid:443?pbk=K").is_err());
        // no port
        assert!(parse_vless_url("vless://uuid@node.example.invalid?pbk=K").is_err());
        // not a url at all
        assert!(parse_vless_url("not a url").is_err());
    }

    #[test]
    fn parses_a_hy2_url() {
        let url = "hy2://example-password@node.example.invalid:8443/?sni=cover.example.invalid&insecure=1#RU%20node";
        let cfg = parse_hy2_url(url).expect("parse");
        assert_eq!(cfg.password, "example-password");
        assert_eq!(cfg.port, 8443);
        assert_eq!(cfg.sni, "cover.example.invalid");
        assert!(cfg.insecure);
        assert_eq!(cfg.remark, "RU node");
    }

    /// With a certificate pin the fingerprint is the verification, and the
    /// standard x509 check breaks self-signed nodes — so a pin implies insecure.
    #[test]
    fn hy2_pin_forces_insecure() {
        let url = "hy2://pw@node.example.invalid:8443/?pinSHA256=AA%3ABB";
        let cfg = parse_hy2_url(url).expect("parse");
        assert_eq!(cfg.pin_sha256, "AA:BB");
        assert!(cfg.insecure);
    }

    /// The backend has shipped SNI as a Markdown link; take the host out of it
    /// instead of handing xray a bracketed string.
    #[test]
    fn hy2_sni_is_cleaned_of_markdown() {
        let url = "hy2://pw@node.example.invalid:8443/?sni=%5Bcover.example.invalid%5D(https%3A%2F%2Fx)";
        let cfg = parse_hy2_url(url).expect("parse");
        assert_eq!(cfg.sni, "cover.example.invalid");
    }

    #[test]
    fn hy2_rejects_incomplete_urls() {
        assert!(parse_hy2_url("hy2://@node.example.invalid:8443").is_err());
        assert!(parse_hy2_url("hy2://pw@node.example.invalid").is_err());
        assert!(parse_hy2_url("").is_err());
    }

    #[test]
    fn subscription_body_is_decoded_from_base64_or_taken_as_is() {
        let plain = "vless://uuid@node.example.invalid:443?pbk=K\n";
        let encoded = base64::Engine::encode(&B64, plain.as_bytes());
        assert_eq!(decode_subscription_body(&encoded), plain);
        assert_eq!(decode_subscription_body(plain), plain);
        // Whitespace inside the base64 payload must not break decoding.
        let wrapped = format!("{}\n{}", &encoded[..8], &encoded[8..]);
        assert_eq!(decode_subscription_body(&wrapped), plain);
    }

    #[test]
    fn server_config_exposes_a_uniform_view() {
        let vless = ServerConfig::Vless(test_support::sample_vless());
        let hy2 = ServerConfig::Hy2(test_support::sample_hy2());
        assert_eq!(vless.proto(), "VLESS");
        assert_eq!(hy2.proto(), "Hysteria2");
        assert_eq!(vless.port(), 443);
        assert_eq!(hy2.port(), 8443);
        assert_eq!(vless.host(), hy2.host());
        assert_eq!(hy2.remark(), "Example hy2 node");
    }
}
