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
