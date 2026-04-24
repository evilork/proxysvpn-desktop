// src-tauri/src/subscription.rs
// Fetches subscription, parses VLESS URLs, builds Xray JSON config.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};
use url::Url;

/// Parsed VLESS Reality config extracted from a `vless://` URL.
#[derive(Debug, Clone)]
pub struct VlessConfig {
    pub uuid: String,
    pub host: String,
    pub port: u16,
    pub encryption: String,  // "none" | "mlkem768x25519plus..." etc.
    pub public_key: String,
    pub short_id: String,
    pub sni: String,
    pub fingerprint: String,
    pub flow: String,
    pub spider_x: String,
    pub remark: String,
}

/// Fetch a subscription URL and return the first usable VLESS config.
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

/// Parse `vless://UUID@host:port?params#remark` into VlessConfig.
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

/// Generate Xray JSON config with SOCKS5 + HTTP inbounds.
pub fn build_xray_config(cfg: &VlessConfig) -> Value {
    // Build user object. Only include `flow` if it's non-empty — otherwise xray
    // rejects the empty string. Same logic applies for `spiderX`.
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
            },
            {
                "tag": "http-in",
                "listen": "127.0.0.1",
                "port": 10809,
                "protocol": "http",
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
            { "tag": "block", "protocol": "blackhole" }
        ],
        "routing": {
            "domainStrategy": "IPIfNonMatch",
            "rules": [
                { "type": "field", "outboundTag": "direct", "ip": ["geoip:private"] }
            ]
        }
    })
}
