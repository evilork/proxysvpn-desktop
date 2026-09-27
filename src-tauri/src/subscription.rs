// src-tauri/src/subscription.rs
//
// Everything the service already tells us, finally listened to.
//
// ── What this layer used to do ──────────────────────────────────────────────
// `fetch_all_servers` read `.text()` and threw the rest of the response away.
// Three consequences, all of them things people wrote to support about:
//
//   1. Seven headers the service fills in on purpose — the expiry date, the
//      outage notice, the profile name, the support link, whether split
//      routing should be on — never reached the screen, so the app could not
//      say a word about the account it was serving.
//   2. Five paid situations arrive as a STUB link (`127.0.0.1:1`) whose remark
//      carries the explanation and the cure. The stub has no `pbk`, so
//      `parse_vless_url` dropped it, the list came out empty and the person
//      read "no servers in subscription" instead of "пополните баланс".
//   3. A Hysteria2 address with a port-hopping list (`host:443,20000-50000`,
//      live since 19.09.2026) makes `url::Url::parse` answer "invalid port
//      number". The error was swallowed by `if let Ok(...)` and the location
//      vanished from the list without a word.
//
// ── What it does now ────────────────────────────────────────────────────────
// One fetch, walked over a ladder of addresses because the subscription domain
// is filtered on some networks; headers parsed into `SubMeta`; refusal stubs
// named with the `ErrorCode` they actually are; ports parsed by hand before
// the URL parser gets a say; the routing rules taken from the service instead
// of a second, home-grown list; and every failure leaving here as an
// `AppError`, never as a sentence.
//
// Nothing here ever puts a node address into a message meant for a person: the
// `detail` of an error is either text the SERVER wrote for the user, or
// nothing at all.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{STANDARD as B64, STANDARD_NO_PAD as B64_NO_PAD};
use base64::Engine as _;
use serde_json::Value;
use url::Url;

use crate::errors::{AppError, ErrorCode};
use crate::events::SubMeta;

// `lib.rs` is not ours to edit, and a module file nobody declares is a file
// nobody compiles. Declaring it here keeps the change inside its own two files;
// move this to a plain `mod device_id;` in `lib.rs` the next time that file is
// touched, and delete these two lines.
#[path = "device_id.rs"]
pub mod device_id;

// ───────────────────────────────────────────────────────────────────────────
// Wire types
// ───────────────────────────────────────────────────────────────────────────

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
    /// What runs inside the REALITY handshake, from the link's `type=`.
    pub transport: VlessTransport,
}

/// Transport of a VLESS entry.
///
/// Until 24.09.2026 every entry was TCP, and the builder had `"tcp"` written
/// into it. The service now also sends XHTTP entries («Британия · XHTTP»):
/// the same REALITY handshake, then HTTP requests inside it instead of one raw
/// stream. Built as TCP, such an entry never connects and looks exactly like a
/// dead node — which is what this app did with it until 27.09.2026.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VlessTransport {
    /// `type=tcp` or no `type` at all. Vision (`flow`) lives only here.
    #[default]
    Tcp,
    /// `type=xhttp` with the `path`, `mode` and optional `host` of the link.
    Xhttp {
        path: String,
        mode: XhttpMode,
        host: Option<String>,
    },
}

/// XHTTP client modes Xray accepts. The service always names one explicitly
/// (`lib/inbound-transport.ts`): "auto" resolves differently per client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhttpMode {
    Auto,
    PacketUp,
    StreamUp,
    StreamOne,
}

impl XhttpMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "auto" => Some(Self::Auto),
            "packet-up" => Some(Self::PacketUp),
            "stream-up" => Some(Self::StreamUp),
            "stream-one" => Some(Self::StreamOne),
            _ => None,
        }
    }

    /// The value for `xhttpSettings.mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::PacketUp => "packet-up",
            Self::StreamUp => "stream-up",
            Self::StreamOne => "stream-one",
        }
    }
}

/// The request path the service may send: "/" plus RFC 3986 unreserved
/// characters and "/", at most 128 in all — the rule of the service's own
/// validator (`isXhttpSettings`), so anything it would refuse is refused here.
fn valid_xhttp_path(path: &str) -> bool {
    path.len() <= 128
        && path.starts_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'/' | b'-'))
}

/// A DNS name for the XHTTP `host`: labels of letters, digits and inner
/// hyphens, dot-separated, 253 characters at most.
fn valid_xhttp_host(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
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

/// Short server description for the UI. Carries `host` because the engine
/// needs it; the window is under orders never to render it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerInfo {
    pub index: usize,
    pub remark: String,
    pub host: String,
    pub port: u16,
    pub proto: String,
}

/// One subscription entry, whichever protocol it speaks.
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

    /// What the list of countries shows under a location: the protocol and,
    /// for VLESS, what runs inside REALITY. Names of technologies, the same in
    /// every language. Without it two «Британия» rows — Vision and XHTTP —
    /// looked identical (owner, 27.09.2026).
    pub fn protocol_label(&self) -> &'static str {
        match self {
            ServerConfig::Vless(c) => match c.transport {
                VlessTransport::Xhttp { .. } => "VLESS · XHTTP",
                VlessTransport::Tcp if c.flow.starts_with("xtls-rprx-vision") => "VLESS · Vision",
                VlessTransport::Tcp => "VLESS · TCP",
            },
            ServerConfig::Hy2(_) => "Hysteria2",
        }
    }
}

/// `sing_box`: the iOS build runs sing-box (`singbox.rs`), which has no XHTTP
/// transport — `transport/` of SagerNet/sing-box has none as of 27.09.2026.
/// Built there, an XHTTP entry would go out as plain TCP and fail every time
/// while looking like a dead node, so that build does not list it at all.
fn engine_supports_on(server: &ServerConfig, sing_box: bool) -> bool {
    match server {
        ServerConfig::Vless(c) => !(sing_box && matches!(c.transport, VlessTransport::Xhttp { .. })),
        ServerConfig::Hy2(_) => true,
    }
}

/// Split-routing rules as the SERVICE defines them.
///
/// Parsed out of the `happ://routing/onadd/<base64 json>` line the
/// subscription puts first in the body. See `parse_routing_line` for why this
/// is the single source of truth and the constant below is only a seed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingRules {
    /// Domains that must bypass the tunnel, in xray rule form.
    pub direct_domains: Vec<String>,
    /// Networks that must bypass the tunnel, CIDR or `geoip:` form.
    pub direct_ips: Vec<String>,
    /// `AsIs` or `IPIfNonMatch`, exactly as the service chose it.
    pub domain_strategy: String,
}

impl RoutingRules {
    /// A profile that names nothing to send directly — the service's "global"
    /// profile, for people abroad. Distinguishing it from "we have no rules"
    /// matters: the first means send everything through the tunnel, the second
    /// means fall back to the seed.
    pub fn is_global(&self) -> bool {
        self.direct_domains.is_empty()
    }

    /// Everything through the tunnel. `AsIs` matches the service's own global
    /// profile: with nothing to match by name there is no reason to pay for a
    /// resolve before every connection.
    pub fn global() -> Self {
        Self {
            direct_domains: Vec::new(),
            direct_ips: Vec::new(),
            domain_strategy: "AsIs".to_string(),
        }
    }
}

/// One subscription response, understood.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub servers: Vec<ServerConfig>,
    pub meta: SubMeta,
    /// `None` when the response carried no routing profile (`?split=0`, or a
    /// single-node account).
    pub routing: Option<RoutingRules>,
    /// Lines that looked like servers and could not be read. Surfaced so a
    /// location never disappears in silence — the window says "одну локацию не
    /// удалось прочитать" instead of showing a shorter list.
    pub unreadable_lines: usize,
    /// Which address of the ladder answered. The subscription site, never a
    /// node — safe for the support report.
    pub source_host: String,
}

// ───────────────────────────────────────────────────────────────────────────
// The ladder of subscription addresses
// ───────────────────────────────────────────────────────────────────────────

/// Reserve addresses, in the order measurement put them.
///
/// Taken from the service, not invented here:
///   • `proxysvnovich.vercel.app` is `FALLBACK_ORIGIN` / `RESERVE_SITE_DEFAULT`
///     (frontend/src/lib/routing-profile.ts, reserve-site.ts). The comment
///     there records the 19.09.2026 measurement from three Russian nodes: it
///     answered on all three, while our own names did not — the filter reads
///     the NAME in the TLS handshake, and a well-known platform's name passes
///     where ours does not.
///   • `proksya.com` / `proksya.xyz` are named in the same comment as working
///     reserves on two networks out of three, which is why they come after.
///
/// `proxysvpn.com` is not listed: it is normally the host of the link itself
/// and is added first by `candidate_urls`. It is appended only when the
/// person's link already lives on a reserve.
///
/// The path and the token never change — the binding lives on the token
/// (`sub:<token>:hwid`), not on the site name, so walking the ladder does not
/// weaken "one link — one device". For the same reason `/api/vpn/sub-alt` is
/// never called from here: that endpoint deliberately CLEARS the binding.
const RESERVE_HOSTS: &[&str] = &[
    "proxysvnovich.vercel.app",
    "proksya.com",
    "proksya.xyz",
    "proxysvpn.com",
];

/// How long one address gets before we stop waiting for it.
const HOST_TIMEOUT: Duration = Duration::from_secs(4);

/// How long after the previous address the next one joins the race.
///
/// Not zero: firing five requests at once would quintuple the load for the
/// 99 % of people whose primary address works, and the service's own limiter
/// counts per token (30/min). Not larger either: 1.2 s is under the point where
/// a person starts wondering whether the button worked.
const STAGGER: Duration = Duration::from_millis(1200);

/// Total budget for the whole ladder.
const LADDER_BUDGET: Duration = Duration::from_secs(9);

/// Largest subscription body we will read.
///
/// The real one is 11-31 KB. The cap exists because a captive portal answers
/// every request with a page of its own, and a hotel login page must not be
/// read into memory as if it were a subscription.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

fn http_client() -> Result<&'static reqwest::Client, AppError> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(build_client)
        .as_ref()
        .ok_or_else(|| AppError::with_detail(ErrorCode::Unknown, "http client init failed"))
}

fn build_client() -> Option<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();

    // One link — one device. The server binds strictly inside `if (hwid)`, so
    // an app that omits this header is an app that opts out of the rule. See
    // device_id.rs for why the value may legitimately be absent.
    if let Some(id) = device_id::device_id() {
        if let Ok(value) = reqwest::header::HeaderValue::from_str(id) {
            headers.insert("x-hwid", value);
        }
    }

    reqwest::Client::builder()
        .user_agent(device_id::user_agent())
        .default_headers(headers)
        .timeout(HOST_TIMEOUT)
        .build()
        .ok()
}

/// Every address to try, in order, with the query we need.
///
/// Two query parameters are forced onto the link, and both are the service's
/// own, documented flags — no backend change is involved:
///
/// `format=vless` — the default format is chosen by a server-side environment
/// variable (`SUBSCRIPTION_FORMAT_DEFAULT`, "xray" in production). In that
/// format the body is an xray JSON config, which this parser cannot read at
/// all: an account with no Hysteria2 node would get a valid response and an
/// empty server list. Asking for the list format makes the answer independent
/// of a server-side setting we do not control.
///
/// `routing=inline` — see `parse_routing_line`.
fn candidate_urls(sub_url: &str) -> Result<Vec<String>, AppError> {
    let trimmed = sub_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(ErrorCode::NoSubscription));
    }

    let mut primary = Url::parse(trimmed).map_err(|_| AppError::new(ErrorCode::SubMalformed))?;
    if !matches!(primary.scheme(), "http" | "https") {
        return Err(AppError::new(ErrorCode::SubMalformed));
    }
    if primary.host_str().is_none() {
        return Err(AppError::new(ErrorCode::SubMalformed));
    }

    force_query(&mut primary);

    let primary_host = primary.host_str().unwrap_or_default().to_ascii_lowercase();
    let mut urls = vec![primary.to_string()];

    for host in RESERVE_HOSTS {
        if *host == primary_host {
            continue;
        }
        let mut alt = primary.clone();
        // A reserve is reached on the default port; carrying a port from the
        // original link would point at nothing.
        if alt.set_host(Some(host)).is_err() || alt.set_port(None).is_err() {
            continue;
        }
        urls.push(alt.to_string());
    }

    Ok(urls)
}

/// Set the two parameters we depend on, keeping everything else the link had.
///
/// `lang` and `split` are the person's own choices and are never touched: the
/// first decides the language of the texts the server writes, the second is
/// how someone abroad turns split routing off.
fn force_query(url: &mut Url) {
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "format" && k != "routing")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let mut qs = url.query_pairs_mut();
    qs.clear();
    for (k, v) in kept {
        qs.append_pair(&k, &v);
    }
    qs.append_pair("format", "vless");
    qs.append_pair("routing", "inline");
    drop(qs);

    // `query_pairs_mut` on a URL that had no query leaves an empty one behind
    // when nothing is appended; we always append, so this only normalises.
    if url.query() == Some("") {
        url.set_query(None);
    }
}

/// Drop `routing=inline` from a URL, for the one retry described in
/// `fetch_subscription`.
fn without_inline_routing(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "routing")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut qs = url.query_pairs_mut();
        qs.clear();
        for (k, v) in kept {
            qs.append_pair(&k, &v);
        }
    }
    if url.query() == Some("") {
        url.set_query(None);
    }
    Some(url.to_string())
}

/// What one address answered.
struct HostAnswer {
    /// The exact URL that produced this answer, kept so the one retry of
    /// `fetch_subscription` goes back to the address that already worked.
    url: String,
    host: String,
    status: u16,
    headers: Headers,
    body: String,
}

/// Hand-written so that a panic message, a log line or a support report can
/// never carry the subscription token, which lives in `url`.
impl std::fmt::Debug for HostAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostAnswer")
            .field("host", &self.host)
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

impl HostAnswer {
    fn request_url(&self) -> &str {
        &self.url
    }

    /// Whether this answer settles the question.
    ///
    /// A 200 with a body is the answer, stub or not. A 404 and a plain 403 are
    /// the BACKEND speaking, and every address on the ladder reaches the same
    /// backend, so waiting for the others would only add seconds to the same
    /// verdict. A 429, a 403 from the hosting shield and a 5xx are the
    /// PLATFORM speaking, and that one differs per address — those keep the
    /// race going.
    fn is_final(&self) -> bool {
        match self.status {
            200 => !self.body.trim().is_empty(),
            403 => !self.headers.contains_key("x-vercel-mitigated"),
            404 => true,
            _ => false,
        }
    }
}

type Headers = HashMap<String, String>;

/// Fetch the subscription, understood, over the ladder of addresses.
pub async fn fetch_subscription(sub_url: &str) -> Result<Subscription, AppError> {
    let urls = candidate_urls(sub_url)?;
    let answer = race_hosts(&urls).await?;

    let parsed = interpret_response(
        &answer.host,
        answer.status,
        &answer.headers,
        &answer.body,
        now_secs(),
    );

    match parsed {
        Ok(sub) => {
            adopt_routing(&sub);
            Ok(sub)
        }
        Err(err) if retry_without_inline(&err) => {
            // The inline routing profile adds about 20 KB to a body that is
            // otherwise 11. Some Russian networks cut a foreign data-centre
            // stream at a fixed size (the "16 KB curtain" of 16.09.2026), and
            // a body cut in half decodes to nothing readable. Rather than
            // guess whether this person is on such a network, ask once more
            // without the big part: the small response is the one that has
            // always worked, and the routing rules then fall back to the seed.
            let retry_url =
                without_inline_routing(answer.request_url()).ok_or_else(|| err.clone())?;
            let second = race_hosts(&[retry_url]).await.map_err(|_| err.clone())?;
            let sub = interpret_response(
                &second.host,
                second.status,
                &second.headers,
                &second.body,
                now_secs(),
            )?;
            adopt_routing(&sub);
            Ok(sub)
        }
        Err(err) => Err(err),
    }
}

/// Put the routing rules of this response into force.
///
/// `routing-enable: false` is the ONLY thing that can switch off a profile the
/// client already has — the service says so itself, because `happ://routing/
/// onadd` overwrites and re-activates a profile by name on every refresh, so
/// "just don't send one" turns nothing off. A person abroad who added `?split=0`
/// to the link gets that header and no profile line at all; without this branch
/// the previously cached Russian rules would quietly stay in the xray config
/// and his banking and RuTube would keep breaking — the exact complaint the
/// flag exists to answer.
fn adopt_routing(sub: &Subscription) {
    if sub.meta.routing_enabled == Some(false) {
        remember_routing(Some(RoutingRules::global()));
        return;
    }
    remember_routing(sub.routing.clone());
}

/// Only a body we could not read is worth asking again for.
///
/// A refusal, a dead link or an unreachable network all say the same thing on
/// the second attempt, and a person waiting for the button does not get to pay
/// for our curiosity.
fn retry_without_inline(err: &AppError) -> bool {
    matches!(err.code, ErrorCode::SubInvalid | ErrorCode::SubEmpty)
}

/// Compatibility entry point for `lib.rs`, which asks only for the list.
pub async fn fetch_all_servers(sub_url: &str) -> Result<Vec<ServerConfig>, AppError> {
    Ok(fetch_subscription(sub_url).await?.servers)
}

/// Run the ladder: start the first address now, let each next one join 1.2 s
/// later, and take the first answer that settles the question.
async fn race_hosts(urls: &[String]) -> Result<HostAnswer, AppError> {
    let client = http_client()?;
    let mut tasks = tokio::task::JoinSet::new();

    for (i, url) in urls.iter().enumerate() {
        let client = client.clone();
        let url = url.clone();
        let delay = STAGGER * i as u32;
        tasks.spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            try_host(&client, &url).await
        });
    }

    let deadline = tokio::time::Instant::now() + LADDER_BUDGET;
    // The best non-final answer seen so far: a 429 or a shield 403 still tells
    // the person more than "the network is down".
    let mut fallback: Option<HostAnswer> = None;
    let mut transport: Option<AppError> = None;

    loop {
        match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            // Out of budget, or every address has answered.
            Err(_) | Ok(None) => break,
            // A task that panicked tells us nothing; the others still run.
            Ok(Some(Err(_))) => continue,
            Ok(Some(Ok(Ok(answer)))) => {
                if answer.is_final() {
                    return Ok(answer);
                }
                if fallback.is_none() {
                    fallback = Some(answer);
                }
            }
            Ok(Some(Ok(Err(err)))) => {
                if transport.is_none() {
                    transport = Some(err);
                }
            }
        }
    }

    if let Some(answer) = fallback {
        return Ok(answer);
    }
    // Every address failed before answering. That is almost always the
    // person's own network — which is exactly why the ladder exists, and why
    // the code says "unreachable" rather than blaming the service.
    Err(transport.unwrap_or_else(|| AppError::new(ErrorCode::SubUnreachable)))
}

async fn try_host(client: &reqwest::Client, url: &str) -> Result<HostAnswer, AppError> {
    let host = Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| classify_transport(&e))?;

    let status = response.status().as_u16();
    let headers = collect_headers(response.headers());

    let bytes = response
        .bytes()
        .await
        .map_err(|e| classify_transport(&e))?;
    // Lossy on purpose: a body that is almost text is still worth reading, and
    // a hard error here would hide a refusal notice over one bad byte.
    let body = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BODY_BYTES)]).into_owned();

    Ok(HostAnswer {
        url: url.to_string(),
        host,
        status,
        headers,
        body,
    })
}

/// Name a transport failure without ever quoting the address it happened on.
fn classify_transport(err: &reqwest::Error) -> AppError {
    // No `detail`: the text of a reqwest error carries the URL, and the URL
    // carries the subscription token.
    if err.is_timeout() {
        return AppError::new(ErrorCode::SubUnreachable);
    }
    if err.is_connect() {
        return AppError::new(ErrorCode::SubUnreachable);
    }
    if err.is_body() || err.is_decode() {
        return AppError::new(ErrorCode::SubInvalid);
    }
    AppError::new(ErrorCode::SubUnreachable)
}

fn collect_headers(map: &reqwest::header::HeaderMap) -> Headers {
    let mut out = Headers::new();
    for (name, value) in map.iter() {
        // Header values are Latin-1 by the HTTP specification, and the service
        // relies on that: anything outside it is sent `base64:`-prefixed
        // instead, because Node refuses to emit a non-Latin-1 header at all.
        // So byte-to-char is the correct decoding here, not a shortcut.
        let text: String = value.as_bytes().iter().map(|&b| b as char).collect();
        out.insert(name.as_str().to_ascii_lowercase(), text);
    }
    out
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ───────────────────────────────────────────────────────────────────────────
// Turning one response into a verdict
// ───────────────────────────────────────────────────────────────────────────

/// The whole decision, with no I/O in it, so every branch is testable.
fn interpret_response(
    host: &str,
    status: u16,
    headers: &Headers,
    body: &str,
    now: u64,
) -> Result<Subscription, AppError> {
    interpret_response_on(host, status, headers, body, now, cfg!(target_os = "ios"))
}

/// The same, with the engine named: `sing_box` as in engine_supports_on.
fn interpret_response_on(
    host: &str,
    status: u16,
    headers: &Headers,
    body: &str,
    now: u64,
    sing_box: bool,
) -> Result<Subscription, AppError> {
    let meta = parse_meta(headers);

    if status != 200 {
        return Err(status_error(status, headers, &meta, now));
    }

    if body.trim().is_empty() {
        return Err(AppError::new(ErrorCode::SubEmpty));
    }

    let decoded = decode_subscription_body(body);
    let mut servers: Vec<ServerConfig> = Vec::new();
    let mut routing: Option<RoutingRules> = None;
    let mut unreadable = 0usize;
    let mut engine_skipped = 0usize;
    let mut stub_remark: Option<String> = None;
    let mut link_lines = 0usize;

    for line in decoded.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(rules) = parse_routing_line(line) {
            routing = Some(rules);
            continue;
        }
        if !is_link_line(line) {
            continue;
        }
        link_lines += 1;

        if let Some(remark) = stub_notice(line) {
            // A stub is a message, not a server. Keep the first one: the
            // service never sends more than one.
            if stub_remark.is_none() {
                stub_remark = Some(remark);
            }
            continue;
        }

        match parse_server_line(line) {
            // Left out, not counted as unreadable: the line is fine, this
            // build's engine just cannot run it (see engine_supports).
            Ok(cfg) if !engine_supports_on(&cfg, sing_box) => engine_skipped += 1,
            Ok(cfg) => servers.push(cfg),
            // The line is not lost: the count travels to the window, which
            // says "одну локацию не удалось прочитать" instead of quietly
            // showing a shorter list.
            Err(_) => unreadable += 1,
        }
    }

    if servers.is_empty() {
        if let Some(remark) = stub_remark {
            return Err(refusal_error(&meta, &remark, now));
        }
        if engine_skipped > 0 {
            // Lines were read fine; this build cannot run any of them. Not
            // SubInvalid: that one triggers a second request, which would
            // bring the same list back.
            return Err(AppError::new(ErrorCode::EngineUnsupported));
        }
        if link_lines == 0 {
            // A 200 with a body that holds no links at all: a captive portal,
            // an HTML error page, or a format we did not ask for.
            return Err(AppError::new(ErrorCode::SubInvalid));
        }
        // Links were there and not one of them could be read.
        return Err(AppError::new(ErrorCode::SubInvalid));
    }

    Ok(Subscription {
        servers,
        meta,
        routing,
        unreadable_lines: unreadable,
        source_host: host.to_string(),
    })
}

/// A non-200 answer, named.
fn status_error(status: u16, headers: &Headers, meta: &SubMeta, now: u64) -> AppError {
    match status {
        // The subscription itself is gone: an unknown token, a deleted device,
        // an account that no longer exists. The bodies behind these are
        // internal English tags ("Token not found"), so nothing of them is
        // shown to the person — the screen has a translated phrase for the
        // code, and the code is the truth here.
        404 => AppError::new(ErrorCode::NoDevices),
        403 if headers.contains_key("x-vercel-mitigated") => {
            // Our own hosting's shield, not the person's provider and not a
            // block. Continuing to poll extends it — two nodes once sat out
            // two and a half hours — so this must read as "try later", which
            // is what SubUnreachable's single button says.
            AppError::new(ErrorCode::SubUnreachable)
        }
        // A plain 403 is "Invalid token": the link is not one of ours.
        403 => AppError::new(ErrorCode::SubMalformed),
        // The service allows 30 requests a minute per token and the app needs
        // two. Hitting this means something is looping, and the cure is to
        // wait — the same cure as an unreachable address.
        429 => AppError::new(ErrorCode::SubUnreachable),
        _ => {
            // Nothing else is documented. If the expiry we already hold has
            // passed, say so rather than shrugging.
            if let Some(expires) = meta.expires_at {
                if expires <= now {
                    return AppError::new(ErrorCode::Expired);
                }
            }
            AppError::new(ErrorCode::SubUnreachable)
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Headers
// ───────────────────────────────────────────────────────────────────────────

/// Marker the service puts in front of any value that is not Latin-1.
const B64_HEADER_PREFIX: &str = "base64:";

/// Read one header as text, decoding the `base64:` form the service uses for
/// Cyrillic. Empty values come back as `None` so the window never renders a
/// blank line where a notice should be.
fn header_text(headers: &Headers, name: &str) -> Option<String> {
    let raw = headers.get(name)?.trim();
    let text = match raw.strip_prefix(B64_HEADER_PREFIX) {
        Some(encoded) => {
            let bytes = decode_base64(encoded.trim())?;
            String::from_utf8(bytes).ok()?
        }
        None => raw.to_string(),
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// `subscription-userinfo: upload=0; download=0; total=0; expire=1760000000`.
///
/// `expire=0` is the service's way of saying "no date", not "expired in 1970" —
/// treating it as a date would tell everyone without a paid period that their
/// access ran out fifty years ago.
fn parse_expire(value: &str) -> Option<u64> {
    for part in value.split(';') {
        let part = part.trim();
        if let Some(num) = part.strip_prefix("expire=") {
            let seconds: u64 = num.trim().parse().ok()?;
            return if seconds == 0 { None } else { Some(seconds) };
        }
    }
    None
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// Everything the response says about the account.
///
/// Field for field, these are the headers `HAPP_UI` and the refusal branches
/// of `frontend/src/app/api/sub/[token]/route.ts` set. Names and shapes are
/// taken from there, not guessed.
fn parse_meta(headers: &Headers) -> SubMeta {
    let info_text = header_text(headers, "sub-info-text").filter(|t| {
        // "0" is not a message: `balanceInfoHeaders(false)` sends it to CLEAR a
        // sticky info block, because an empty header value would crash the
        // whole response on the server side.
        t != "0"
    });

    SubMeta {
        title: header_text(headers, "profile-title"),
        announce: header_text(headers, "announce"),
        announce_url: header_text(headers, "announce-url"),
        expires_at: headers
            .get("subscription-userinfo")
            .and_then(|v| parse_expire(v)),
        info_text,
        support_url: header_text(headers, "support-url"),
        routing_enabled: headers.get("routing-enable").and_then(|v| parse_bool(v)),
        update_interval_hours: headers
            .get("profile-update-interval")
            .and_then(|v| v.trim().parse::<u32>().ok()),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Body
// ───────────────────────────────────────────────────────────────────────────

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    B64.decode(&cleaned)
        .or_else(|_| B64_NO_PAD.decode(&cleaned))
        .ok()
}

/// The body arrives base64-wrapped by default and in plain text with `?raw=1`.
/// Both are accepted: a body we can read is worth more than a body in the
/// shape we expected.
fn decode_subscription_body(body: &str) -> String {
    if let Some(bytes) = decode_base64(body) {
        if let Ok(text) = String::from_utf8(bytes) {
            // A decode that produces no line we recognise is a coincidence,
            // not a subscription — keep the original in that case.
            if text.lines().any(|l| is_link_line(l.trim())) {
                return text;
            }
        }
    }
    body.to_string()
}

fn is_link_line(line: &str) -> bool {
    line.starts_with("vless://") || line.starts_with("hy2://") || line.starts_with("hysteria2://")
}

fn parse_server_line(line: &str) -> Result<ServerConfig, AppError> {
    if line.starts_with("vless://") {
        return parse_vless_url(line).map(ServerConfig::Vless);
    }
    if line.starts_with("hy2://") || line.starts_with("hysteria2://") {
        return parse_hy2_url(line).map(ServerConfig::Hy2);
    }
    Err(AppError::new(ErrorCode::SubInvalid))
}

// ───────────────────────────────────────────────────────────────────────────
// Refusal stubs — the five paid situations that used to read as a parser bug
// ───────────────────────────────────────────────────────────────────────────

/// Address every refusal stub points at.
const STUB_HOST: &str = "127.0.0.1";
const STUB_PORT: u16 = 1;

/// The stub's own name, decoded, when this line is a refusal notice rather
/// than a server.
///
/// Detected by the address, not by the text: the address is the one part of
/// the stub the service has never changed, and the texts are rewritten every
/// time support learns a better wording.
fn stub_notice(line: &str) -> Option<String> {
    let (normalised, _) = normalize_port(line).ok()?;
    let url = Url::parse(&normalised).ok()?;
    if url.host_str()? != STUB_HOST || url.port()? != STUB_PORT {
        return None;
    }
    Some(url_fragment(&url))
}

/// Profile titles and stub names the service writes, verbatim from
/// `frontend/src/app/api/sub/[token]/route.ts`.
const DEVICE_TAKEN_MARKERS: &[&str] = &[
    "Сбросьте привязку",
    "Ссылка уже привязана к другому устройству",
];

/// `DELETED_SUB_*` in the same file: the device was removed, or the link is an
/// old one from an old chat.
const NO_DEVICES_MARKERS: &[&str] = &[
    "Ссылка недействительна",
    "Возьмите новую ссылку в боте",
    "Эта ссылка больше не работает",
];

/// `EMPTY_BALANCE_*`, plus the reserve wording of
/// `frontend/src/lib/sub-reserve-notice.ts`.
const BALANCE_EMPTY_MARKERS: &[&str] = &[
    "Пополните баланс",
    "Пополнить:",
    "Баланс на нуле",
    "баланс на нуле",
];

/// The ten `emptyTitle` values of `sub-reserve-notice.ts`, one per language the
/// bot speaks. Listed rather than matched loosely: a title is 25 characters and
/// a loose match on so little text picks up the wrong branch.
const BALANCE_EMPTY_TITLES: &[&str] = &[
    "⚠️ Нет средств",
    "⚠️ No funds",
    "⚠️ 余额不足",
    "⚠️ Sin saldo",
    "⚠️ Bakiye yok",
    "⚠️ لا يوجد رصيد",
    "⚠️ 残高不足",
    "⚠️ Kein Guthaben",
    "⚠️ Solde épuisé",
    "⚠️ 잔액 없음",
];

/// Name the refusal, and carry the service's own words underneath it.
///
/// Order is by specificity, not by frequency: the binding notice and the dead
/// link both name themselves unmistakably, while "no funds" has ten
/// translations and needs the widest net, so it goes last among the markers.
fn refusal_error(meta: &SubMeta, remark: &str, now: u64) -> AppError {
    let title = meta.title.as_deref().unwrap_or("");
    let announce = meta.announce.as_deref().unwrap_or("");
    let info = meta.info_text.as_deref().unwrap_or("");

    let code = if matches_any(DEVICE_TAKEN_MARKERS, &[title, remark, announce, info]) {
        ErrorCode::DeviceTaken
    } else if matches_any(NO_DEVICES_MARKERS, &[title, remark, announce, info]) {
        ErrorCode::NoDevices
    } else if BALANCE_EMPTY_TITLES.iter().any(|t| title.contains(t))
        || matches_any(BALANCE_EMPTY_MARKERS, &[title, remark, announce, info])
        // Every localised empty-balance title starts with a warning sign,
        // while both working titles carry a shield or a gem. A language added
        // to the bot tomorrow lands here instead of in "some other notice".
        || title.starts_with('\u{26a0}')
    {
        ErrorCode::BalanceEmpty
    } else if meta.expires_at.is_some_and(|e| e <= now) {
        ErrorCode::Expired
    } else {
        ErrorCode::SubNotice
    };

    // The service writes these texts for the person, in the person's own
    // language, and support has been rewriting them for months — the cure for
    // the binding notice is inside the sentence itself. Ours would be a worse
    // copy, so we show theirs under our translated heading.
    let detail = first_non_empty(&[info, announce, remark]);
    match detail {
        Some(text) => AppError::with_detail(code, text),
        None => AppError::new(code),
    }
}

fn matches_any(markers: &[&str], haystacks: &[&str]) -> bool {
    haystacks
        .iter()
        .any(|h| markers.iter().any(|m| h.contains(m)))
}

fn first_non_empty(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|c| c.trim())
        .find(|c| !c.is_empty())
        .map(|c| c.to_string())
}

// ───────────────────────────────────────────────────────────────────────────
// Ports — the Hysteria2 hopping list
// ───────────────────────────────────────────────────────────────────────────

/// Rewrite a link so that the port is a single number, and report the hopping
/// specification we removed.
///
/// Why by hand, before `Url::parse`. Since 19.09.2026 the service puts port
/// hopping where the Hysteria2 scheme says it belongs — in the port of the
/// address — and in the list form `host:443,20000-50000`, because a bare range
/// breaks every client that cannot hop (INCY on the desktop reads the first
/// port only, and INCY is what iPhone owners in Russia use). `url::Url::parse`
/// answers "invalid port number" to both forms, and the old code swallowed
/// that error with `if let Ok(...)`, so the location simply disappeared from
/// the list.
///
/// We take the FIRST port of the list, which is the plain one the service puts
/// there for exactly this reason. Hopping itself is not attempted: measurement
/// on 19.09.2026 showed it does not cure the throttling it was added for, and
/// an unproven behaviour in the connect path is not worth the second failure
/// mode.
fn normalize_port(raw: &str) -> Result<(String, Option<String>), AppError> {
    let malformed = || AppError::new(ErrorCode::SubMalformed);

    let scheme_end = raw.find("://").ok_or_else(malformed)?;
    let authority_start = scheme_end + 3;
    if authority_start > raw.len() {
        return Err(malformed());
    }
    let rest = &raw[authority_start..];
    let authority_len = rest
        .find(['/', '?', '#'])
        .unwrap_or(rest.len());
    let authority = &rest[..authority_len];

    // Userinfo may hold anything, including a ':' in a Hysteria2 password, so
    // the host starts after the LAST '@'.
    let host_offset = match authority.rfind('@') {
        Some(at) => at + 1,
        None => 0,
    };
    let hostport = &authority[host_offset..];

    // IPv6 literals keep their colons inside brackets.
    let colon = if hostport.starts_with('[') {
        match hostport.find(']') {
            Some(close) => hostport[close..].find(':').map(|i| close + i),
            None => return Err(malformed()),
        }
    } else {
        hostport.rfind(':')
    };
    let Some(colon) = colon else {
        // No port at all. Let the URL parser answer for it — its error is the
        // same one we would produce here.
        return Ok((raw.to_string(), None));
    };

    let port_start = authority_start + host_offset + colon + 1;
    let port_end = authority_start + authority_len;
    let spec = &raw[port_start..port_end];

    if spec.parse::<u16>().is_ok() {
        return Ok((raw.to_string(), None));
    }

    let port = first_port_of(spec).ok_or_else(malformed)?;
    let mut rebuilt = String::with_capacity(raw.len());
    rebuilt.push_str(&raw[..port_start]);
    rebuilt.push_str(&port.to_string());
    rebuilt.push_str(&raw[port_end..]);
    Ok((rebuilt, Some(spec.to_string())))
}

/// First usable port of `443,20000-50000`, `20000-50000` or `443`.
///
/// A bare range still yields a port — its lower bound — because the server
/// does listen there, and a location that connects on one port of the range is
/// better than a location that silently vanishes.
fn first_port_of(spec: &str) -> Option<u16> {
    for item in spec.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let candidate = match item.split_once('-') {
            Some((low, _high)) => low.trim(),
            None => item,
        };
        if let Ok(port) = candidate.parse::<u16>() {
            if port != 0 {
                return Some(port);
            }
        }
    }
    None
}

fn url_fragment(url: &Url) -> String {
    url.fragment()
        .map(|f| {
            urlencoding::decode(f)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| f.to_string())
        })
        .unwrap_or_default()
}

// ───────────────────────────────────────────────────────────────────────────
// Link parsers
// ───────────────────────────────────────────────────────────────────────────

/// Pull a clean host out of a possible Markdown link `[host](url)` or noise.
///
/// A backend bug has been known to wrap the SNI that way, and an SNI with
/// brackets in it makes the handshake fail on every connection.
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

pub fn parse_vless_url(raw: &str) -> Result<VlessConfig, AppError> {
    let malformed = || AppError::new(ErrorCode::SubMalformed);
    let (normalised, _) = normalize_port(raw)?;
    let u = Url::parse(&normalised).map_err(|_| malformed())?;

    let uuid = u.username().to_string();
    if uuid.is_empty() {
        return Err(malformed());
    }
    let host = u.host_str().ok_or_else(malformed)?.to_string();
    let port = u.port().ok_or_else(malformed)?;

    let mut encryption = "none".to_string();
    let mut public_key = String::new();
    let mut short_id = String::new();
    let mut sni = String::new();
    let mut fingerprint = "chrome".to_string();
    let mut flow = String::new();
    let mut spider_x = String::new();
    let mut network: Option<String> = None;
    let mut xhttp_path: Option<String> = None;
    let mut xhttp_mode: Option<String> = None;
    let mut xhttp_host: Option<String> = None;

    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "encryption" => encryption = v.into_owned(),
            "pbk" => public_key = v.into_owned(),
            "sid" => short_id = v.into_owned(),
            "sni" => sni = v.into_owned(),
            "fp" => fingerprint = v.into_owned(),
            "flow" => flow = v.into_owned(),
            "spx" => spider_x = v.into_owned(),
            "type" => network = Some(v.into_owned()),
            "path" => xhttp_path = Some(v.into_owned()),
            "mode" => xhttp_mode = Some(v.into_owned()),
            "host" => xhttp_host = Some(v.into_owned()),
            _ => {}
        }
    }

    let transport = match network.as_deref() {
        None | Some("") | Some("tcp") => VlessTransport::Tcp,
        Some("xhttp") => {
            let path = xhttp_path.filter(|p| valid_xhttp_path(p)).ok_or_else(malformed)?;
            // Absent means what Xray itself does without it; present and
            // unknown is a line we cannot build honestly.
            let mode = match xhttp_mode.as_deref() {
                None | Some("") => XhttpMode::Auto,
                Some(m) => XhttpMode::parse(m).ok_or_else(malformed)?,
            };
            let host = match xhttp_host {
                None => None,
                Some(h) if h.is_empty() => None,
                Some(h) if valid_xhttp_host(&h) => Some(h),
                Some(_) => return Err(malformed()),
            };
            // Vision is a TCP-only feature: the service never puts `flow` on
            // an XHTTP line, and one that did would not connect.
            flow.clear();
            VlessTransport::Xhttp { path, mode, host }
        }
        // ws, grpc, httpupgrade…: nothing here builds them, and a TCP config
        // in their place would fail every time while looking like a dead
        // node. Counted as unreadable instead, so the window says so.
        Some(_) => return Err(malformed()),
    };

    // REALITY without a public key cannot connect. This is also what makes the
    // refusal stubs unreadable as servers — which is why they are recognised
    // by address before they ever reach this function.
    if public_key.is_empty() {
        return Err(malformed());
    }
    if sni.is_empty() {
        sni = host.clone();
    }

    Ok(VlessConfig {
        uuid,
        host,
        port,
        encryption,
        public_key,
        short_id,
        sni: clean_sni(&sni),
        fingerprint,
        flow,
        spider_x,
        remark: url_fragment(&u),
        transport,
    })
}

pub fn parse_hy2_url(raw: &str) -> Result<Hy2Config, AppError> {
    let malformed = || AppError::new(ErrorCode::SubMalformed);
    let (normalised, _) = normalize_port(raw)?;
    let u = Url::parse(&normalised).map_err(|_| malformed())?;

    let password = u.username().to_string();
    if password.is_empty() {
        return Err(malformed());
    }
    let host = u.host_str().ok_or_else(malformed)?.to_string();
    let port = u.port().ok_or_else(malformed)?;

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
    let sni = clean_sni(&sni);
    // With a certificate pin the fingerprint IS the verification, and the
    // standard x509 check only breaks the self-signed certificates our nodes
    // use. `insecure` is safe here precisely because the pin is present.
    if !pin_sha256.is_empty() {
        insecure = true;
    }

    Ok(Hy2Config {
        password,
        host,
        port,
        sni,
        pin_sha256,
        insecure,
        remark: url_fragment(&u),
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Routing: one source of truth, and it is the service
// ───────────────────────────────────────────────────────────────────────────

/// The line the subscription puts first in the body when split routing is on.
const ROUTING_PREFIX: &str = "happ://routing/onadd/";

/// Parse the service's routing profile out of the subscription body.
///
/// ── Why we take the rules from the server at all ────────────────────────────
/// There were two lists. Ours, written by hand, held about sixty domains. The
/// service's is generated (`frontend/src/lib/ru-direct.ts`, "СГЕНЕРИРОВАНО
/// scripts/build-ru-direct.mjs") and holds 819 domains and 174 prefixes, and it
/// is regenerated when a Russian service breaks — the four names added by hand
/// after the complaints of 31.08.2026 are in it. Two lists mean the app routes
/// differently from every other client of the same subscription, and the
/// difference shows up as "Такси Максим не работает" months later. So the app
/// stops having an opinion: the rules arrive with the servers, in the same
/// response, and the constant below is only what we use when they do not.
///
/// ── Why `?routing=inline` ───────────────────────────────────────────────────
/// The subscription's default routing profile is small because it REFERS to
/// geo categories (`geosite:category-ru`, `geoip:ru`) that the client is
/// expected to download. We do not download them — the files are in the bundle
/// — but a reference to a category that a bundled file does not contain stops
/// the xray core from starting at all, silently, and we cannot check the
/// contents of a `.dat` at runtime. `?routing=inline` is the service's own
/// flag (`inlineRoutingAllowed` in the subscription route) for the profile that
/// carries the lists LITERALLY, with no geo references and nothing to
/// download. Literal names and prefixes are safe to translate into xray rules
/// one for one, so that is what we ask for.
///
/// ── What we drop, and why ───────────────────────────────────────────────────
/// Any `geosite:` entry that does slip through, and any `geoip:` entry other
/// than the two the config has always used. See above: an unresolvable
/// category is not a missing rule, it is a core that does not start.
fn parse_routing_line(line: &str) -> Option<RoutingRules> {
    let encoded = line.strip_prefix(ROUTING_PREFIX)?;
    let bytes = decode_base64(encoded)?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;

    let direct_domains = string_array(&value, "DirectSites")
        .into_iter()
        .filter_map(|e| to_xray_domain(&e))
        .collect();
    let direct_ips = string_array(&value, "DirectIp")
        .into_iter()
        .filter_map(|e| to_xray_ip(&e))
        .collect();
    let domain_strategy = value
        .get("DomainStrategy")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        // Same default the service uses. `AsIs` decides by name only, so a
        // Russian service missing from the lists went through the tunnel and
        // broke on a foreign exit address; `IPIfNonMatch` resolves and checks
        // the address too, which fixed the whole class on 31.08.2026.
        .unwrap_or("IPIfNonMatch")
        .to_string();

    Some(RoutingRules {
        direct_domains,
        direct_ips,
        domain_strategy,
    })
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Happ's rule form to xray's. A bare name is a suffix in both.
fn to_xray_domain(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() || entry.starts_with("geosite:") {
        return None;
    }
    if entry.starts_with("full:")
        || entry.starts_with("domain:")
        || entry.starts_with("regexp:")
        || entry.starts_with("keyword:")
    {
        return Some(entry.to_string());
    }
    Some(format!("domain:{entry}"))
}

fn to_xray_ip(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    if let Some(category) = entry.strip_prefix("geoip:") {
        // Only the two categories this config has shipped with and the bundled
        // geoip.dat is known to carry.
        return match category {
            "ru" | "private" => Some(entry.to_string()),
            _ => None,
        };
    }
    Some(entry.to_string())
}

/// Last routing profile the service sent us.
///
/// Process-wide state, which the project's rules normally forbid, and the
/// reason is worth stating: the routing rules and the xray config are built in
/// two different places, and the function that builds the config
/// (`build_xray_config`, called from `lib.rs`) takes only a server. Threading
/// the rules through would mean editing a file this change does not own, and
/// leaving them unthreaded would mean the hand-written list stays in charge —
/// the exact defect being fixed. The cache is written immediately after every
/// successful fetch and read immediately after, in `vpn_connect`, so it is
/// never stale in practice. `build_xray_config_with_routing` takes the rules
/// as an argument and is what the tests use, so nothing here is untestable.
fn routing_cache() -> &'static RwLock<Option<RoutingRules>> {
    static CACHE: OnceLock<RwLock<Option<RoutingRules>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

/// Keep the newest rules. A fetch that brought none leaves the previous ones
/// in place: a subscription served without a profile (a single-node account,
/// or `?split=0` on someone else's link) is not a reason to forget what the
/// service last told us.
fn remember_routing(rules: Option<RoutingRules>) {
    let Some(rules) = rules else { return };
    let lock = routing_cache();
    // A poisoned lock means another thread panicked while holding it; the data
    // is still a plain value and is safe to replace.
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(rules);
}

/// The rules in force, if the service has told us any.
pub fn last_routing() -> Option<RoutingRules> {
    let lock = routing_cache();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.clone()
}

/// Russian domains routed around the tunnel when the service told us nothing.
///
/// A SEED, not a source of truth. It is used on a first run that could not
/// reach the subscription, and by the iOS sing-box builder until that file is
/// rewritten to take `RoutingRules`. Do not extend it: a domain added here and
/// not to `scripts/build-ru-direct.mjs` recreates the two-lists problem.
pub const RU_DIRECT_DOMAINS: &[&str] = &[
    "yandex.ru", "yandex.com", "yandex.net",
    "ya.ru",
    "vk.com", "vk.ru", "vkuser.net",
    "userapi.com", "vk-cdn.net", "vk-cdn.com",
    "mail.ru", "my.com", "imgsmail.ru",
    "ok.ru", "odnoklassniki.ru",
    "gosuslugi.ru", "nalog.ru", "nalog.gov.ru",
    "sber.ru", "sberbank.ru", "sberbank.com",
    "tinkoff.ru", "t-bank.ru", "tbank.ru",
    "vtb.ru", "alfabank.ru", "raiffeisen.ru",
    "rzd.ru", "tutu.ru", "aviasales.ru",
    "wildberries.ru", "ozon.ru", "ozon.com",
    "avito.ru", "cian.ru", "hh.ru",
    "rambler.ru", "lenta.ru", "rbc.ru",
    "ria.ru", "tass.ru", "kommersant.ru",
    "kinopoisk.ru", "ivi.ru", "rutube.ru",
    "2gis.ru", "2gis.com", "2gis.kz",
    "dns-shop.ru", "mvideo.ru", "eldorado.ru",
    "taximaxim.ru", "taxsee.com", "max.ru", "oneme.ru",
    "proxysvpn.com",
];

/// The DNS server the system is given while the tunnel is up (sysdns.rs):
/// inside the tunnel's reach (128.0.0.0/1 goes to utun), outside every real
/// network, and routed to `dns-out` by a rule placed before the private one.
pub const TUNNEL_DNS: &str = "198.18.0.2";

/// Networks routed around the tunnel when the service told us nothing.
#[cfg(any(target_os = "macos", test))]
const SEED_DIRECT_IPS: &[&str] = &["geoip:private", "geoip:ru"];

/// Build the xray runtime config for a VLESS node, using the routing rules the
/// service last sent.
#[cfg(target_os = "macos")]
pub fn build_xray_config(cfg: &VlessConfig) -> Value {
    build_xray_config_with_routing_and_prefs(cfg, last_routing().as_ref(), &crate::tunnel_prefs::load())
}

/// The same, with the rules passed in. Everything testable lives here.
///
/// `None` means the service has not spoken and the seed is used. `Some` with an
/// empty domain list is NOT the same thing: that is the service's global
/// profile, chosen for someone abroad, and it means everything goes through
/// the tunnel on purpose.
#[cfg(any(target_os = "macos", test))]
pub fn build_xray_config_with_routing(cfg: &VlessConfig, routing: Option<&RoutingRules>) -> Value {
    build_xray_config_with_routing_and_prefs(cfg, routing, &crate::tunnel_prefs::TunnelPrefs::default())
}

/// То же, но с настройками туннеля. Всё проверяемое живёт здесь.
#[cfg(any(target_os = "macos", test))]
pub fn build_xray_config_with_routing_and_prefs(
    cfg: &VlessConfig,
    routing: Option<&RoutingRules>,
    prefs: &crate::tunnel_prefs::TunnelPrefs,
) -> Value {
    use serde_json::json;

    let mut user = serde_json::Map::new();
    user.insert("id".into(), json!(cfg.uuid));
    user.insert("encryption".into(), json!(cfg.encryption));
    // Vision only over TCP; the parser already drops it for XHTTP, and the
    // builder does not rely on that.
    if !cfg.flow.is_empty() && cfg.transport == VlessTransport::Tcp {
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

    let (direct_domains, direct_ips, domain_strategy) = match routing {
        Some(rules) => (
            rules.direct_domains.clone(),
            rules.direct_ips.clone(),
            rules.domain_strategy.clone(),
        ),
        None => (
            RU_DIRECT_DOMAINS
                .iter()
                .map(|d| format!("domain:{d}"))
                .collect(),
            SEED_DIRECT_IPS.iter().map(|s| s.to_string()).collect(),
            "IPIfNonMatch".to_string(),
        ),
    };

    let mut rules: Vec<Value> = Vec::new();
    // 1. Block QUIC. Vision is TCP-only, so UDP/443 would leave the tunnel.
    rules.push(json!({
        "type": "field",
        "outboundTag": "block",
        "network": "udp",
        "port": "443"
    }));
    // 2. СОБСТВЕННЫЕ запросы имён xray - через узел, и только через него.
    //
    // Это лечение поломки, которая стоила нам 1,2 секунды на КАЖДОЕ новое
    // соединение (замер 22.09.2026: пять подряд - 1,18 / 1,17 / 1,25 / 1,20 /
    // 1,21 с, при том что само имя разрешалось за 2 мс).
    //
    // Механика. `domainStrategy: IPIfNonMatch` заставляет xray разрешать имя
    // каждого соединения, чтобы выбрать маршрут. Раздела `dns` у нас не было,
    // поэтому он спрашивал СИСТЕМНЫЙ резолвер - то есть слал пакет на
    // 8.8.8.8 прямо из процесса. Исходящим соединениям мы физический
    // интерфейс прописываем (xray_manager.rs, sockopt.interface), а этому
    // запросу - нет: он уходил в маршрут по умолчанию, то есть ОБРАТНО В
    // ТУННЕЛЬ, к самому xray. Петля разрывалась только по таймеру.
    //
    // Браузер открывает десятки соединений на страницу, и по секунде с
    // лишним на каждое - это и есть «подключилось, но ничего не грузит».
    // Happ и INCY этого не знают, потому что раздел `dns` у них есть.
    rules.push(json!({
        "type": "field",
        "inboundTag": ["dns-in"],
        "outboundTag": "proxy"
    }));
    // 2a. The system's own resolver while the tunnel is up (sysdns.rs). BEFORE
    //     the private-network rule: 198.18.0.0/15 is special-use space, and
    //     sent "direct" the query would go nowhere and every name would hang.
    rules.push(json!({
        "type": "field",
        "outboundTag": "dns-out",
        "ip": [TUNNEL_DNS]
    }));
    // 3. LAN always direct, whatever the profile says: a printer and a router
    //    admin page have no business crossing a border.
    rules.push(json!({
        "type": "field",
        "outboundTag": "direct",
        "ip": ["geoip:private"]
    }));
    // 4. Чужие запросы имён (браузера, системы) тоже забирает xray.
    //
    // Идёт ПОСЛЕ правила про частные сети: запрос к домашнему роутеру - это
    // локальное имя, и отправлять его за границу незачем.
    rules.push(json!({
        "type": "field",
        "outboundTag": "dns-out",
        "port": 53
    }));
    if !direct_ips.is_empty() {
        rules.push(json!({
            "type": "field",
            "outboundTag": "direct",
            "ip": direct_ips
        }));
    }
    if !direct_domains.is_empty() {
        rules.push(json!({
            "type": "field",
            "outboundTag": "direct",
            "domain": direct_domains
        }));
    }
    // No catch-all: xray sends what matched nothing to the first outbound.

    let outbounds = build_outbounds(cfg, user, reality, prefs);

    json!({
        "log": { "loglevel": "warning" },
        // Свой резолвер вместо системного. Без него xray спрашивал имя в
        // обход наших правил - см. длинный разбор у правила 2 выше.
        //
        // `UseIPv4` намеренно: IPv6-выхода у наших узлов нет (проверено
        // 07.09.2026 по всему флоту), и запрос AAAA - это гарантированное
        // ожидание впустую на каждом имени.
        "dns": {
            // DoH и TCP, а не голый UDP - и это исправление собственной ошибки,
            // сделанной несколькими часами раньше.
            //
            // Сначала тут стояло ["1.1.1.1", "8.8.8.8"], то есть запросы по
            // UDP. А отправляем мы их через `proxy`, и двумя правилами выше
            // сами же написали: Vision работает ТОЛЬКО по TCP, потому и QUIC
            // заблокирован. Запрос уходил в выход, который его не несёт, и
            // первые секунды после подъёма туннеля имена не разрешались вовсе:
            // замер показал 30 секунд на первое имя, а дальше - миллисекунды.
            //
            // DoH идёт по TCP/443 и проходит там же, где обычный трафик.
            // `tcp://` вторым - на случай, если DoH у человека режут: тот же
            // транспорт, но без HTTPS поверх.
            "servers": prefs.dns_servers(),
            // IPv6-выхода у наших узлов нет ни на одном (проверено по флоту
            // 07.09.2026), поэтому запрос AAAA - гарантированное ожидание
            // впустую на каждом имени.
            "queryStrategy": prefs.ip_kind.query_strategy(),
            "tag": "dns-in"
        },
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
        "outbounds": outbounds,
        "routing": {
            "domainStrategy": domain_strategy,
            "rules": rules
        }
    })
}

/// Исходящие: узел, прямой выход, чёрная дыра, резолвер и - по желанию -
/// дробильщик рукопожатия.
///
/// Вынесено отдельной функцией, потому что дробление вставляет ЛИШНИЙ
/// исходящий и меняет настройки сокета у основного: собирать это вперемешку с
/// маршрутизацией в одном литерале стало нечитаемо.
#[cfg(any(target_os = "macos", test))]
fn build_outbounds(
    cfg: &VlessConfig,
    user: serde_json::Map<String, Value>,
    reality: serde_json::Map<String, Value>,
    prefs: &crate::tunnel_prefs::TunnelPrefs,
) -> Vec<Value> {
    use serde_json::json;

    let mut stream = serde_json::Map::new();
    match &cfg.transport {
        VlessTransport::Tcp => {
            stream.insert("network".into(), json!("tcp"));
        }
        VlessTransport::Xhttp { path, mode, host } => {
            stream.insert("network".into(), json!("xhttp"));
            let mut xhttp = serde_json::Map::new();
            xhttp.insert("path".into(), json!(path));
            xhttp.insert("mode".into(), json!(mode.as_str()));
            if let Some(host) = host {
                xhttp.insert("host".into(), json!(host));
            }
            // No `xmux` block: Xray's own defaults, the same a Happ user gets
            // from this line. Tuning it is a measured experiment (DESIGN §9,
            // phase 0), not a guess shipped to everyone.
            stream.insert("xhttpSettings".into(), Value::Object(xhttp));
        }
    }
    stream.insert("security".into(), json!("reality"));
    stream.insert("realitySettings".into(), Value::Object(reality));
    if prefs.fragment {
        // Основной выход дозванивается ЧЕРЕЗ дробильщик: тот режет первый
        // пакет рукопожатия, и запрещённое имя в ClientHello оказывается
        // разрезанным между пакетами. Простая проверка по образцу его уже не
        // находит.
        stream.insert("sockopt".into(), json!({ "dialerProxy": "fragment" }));
    }

    let mut outbounds = vec![
        json!({
            "tag": "proxy",
            "protocol": "vless",
            "settings": { "vnext": [{
                "address": cfg.host,
                "port": cfg.port,
                "users": [ Value::Object(user) ]
            }]},
            "streamSettings": Value::Object(stream)
        }),
        json!({ "tag": "direct", "protocol": "freedom" }),
        json!({ "tag": "block",  "protocol": "blackhole" }),
        // Отвечает на запросы имён сам, по разделу `dns` выше.
        //
        // Только на запросы адресов (A, AAAA). Остальные типы xray 26.3 по
        // умолчанию отбивает ответом REFUSED, и это стоило туннелю DNS целиком
        // (27.09.2026, запись пакетов на utun225): macOS шлёт рядом с каждым
        // именем запрос HTTPS (тип 65), получает REFUSED от обоих серверов,
        // считает их сломанными и перестаёт спрашивать вовсе. Новые имена
        // висели по 30 секунд - «подключилось, но Claude не думает» - а
        // прогрев при подключении ждал 16 секунд и не дожидался.
        //
        // `skip` отдаёт такие запросы настоящему серверу: 1.1.1.1 по TCP и
        // ЧЕРЕЗ УЗЕЛ (`proxySettings`). Без цепочки запрос ушёл бы в маршрут
        // по умолчанию, то есть обратно в туннель, к этому же правилу; а с
        // привязкой к физическому интерфейсу имена сайтов уходили бы мимо
        // туннеля. Замер на том же xray: 16 из 16 верных ответов, в среднем
        // 0,7 с, повтор из кеша - 74 мс; через `sockopt.dialerProxy` тот же
        // приём давал потери и ложный NXDOMAIN.
        json!({
            "tag": "dns-out",
            "protocol": "dns",
            "settings": {
                "nonIPQuery": "skip",
                "network": "tcp",
                "address": "1.1.1.1",
                "port": 53
            },
            "proxySettings": { "tag": "proxy" }
        }),
    ];

    if prefs.fragment {
        outbounds.push(json!({
            "tag": "fragment",
            "protocol": "freedom",
            "settings": { "fragment": {
                // Режем ТОЛЬКО приветствие TLS: дробить весь поток дорого и
                // заметно само по себе.
                "packets": "tlshello",
                "length": "100-200",
                "interval": "10-20"
            }}
        }));
    }
    outbounds
}


// ───────────────────────────────────────────────────────────────────────────
// Tests
//
// Every fixture below is the shape the live service actually sends: the header
// names come from `frontend/src/app/api/sub/[token]/route.ts`, the stub bodies
// from the same file and from `lib/sub-reserve-notice.ts`, and the Hysteria2
// port list from `lib/hy2-ports.ts`. A test written against an invented shape
// proves nothing about what reaches people — that is the whole lesson of the
// 17.09.2026 port-hopping rollout.
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(text: &str) -> String {
        B64.encode(text.as_bytes())
    }

    fn b64_header(text: &str) -> String {
        format!("base64:{}", b64(text))
    }

    fn headers(pairs: &[(&str, &str)]) -> Headers {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
            .collect()
    }

    fn frag(text: &str) -> String {
        urlencoding::encode(text).into_owned()
    }

    const VLESS_LINE: &str = "vless://11111111-2222-3333-4444-555555555555@de.example.net:443?encryption=none&security=reality&pbk=PUBKEY&sid=ab12&sni=www.bing.com&fp=chrome&flow=xtls-rprx-vision&type=tcp#%D0%93%D0%B5%D1%80%D0%BC%D0%B0%D0%BD%D0%B8%D1%8F";

    /// The live form since 19.09.2026: a plain port first, the hop range after.
    const HY2_LINE: &str = "hy2://11111111-2222-3333-4444-555555555555@nl.example.net:443,20000-50000/?sni=www.bing.com&pinSHA256=AA%3ABB&mport=20000-50000#%D0%9D%D0%B8%D0%B4%D0%B5%D1%80%D0%BB%D0%B0%D0%BD%D0%B4%D1%8B";

    fn stub_line(name: &str) -> String {
        format!(
            "vless://00000000-0000-0000-0000-000000000000@127.0.0.1:1?encryption=none&type=tcp&security=none#{}",
            frag(name)
        )
    }

    fn routing_line(json: &str) -> String {
        format!("{ROUTING_PREFIX}{}", b64(json))
    }

    const INLINE_PROFILE: &str = r#"{"Name":"ProxysVPN RU Split 9","GlobalProxy":"true","DirectSites":["taximaxim.ru","max.ru","full:mvideo.edna.io","geosite:category-ru"],"DirectIp":["87.240.128.0/18","geoip:ru","geoip:de","10.0.0.0/8"],"DomainStrategy":"IPIfNonMatch","FakeDNS":"false"}"#;

    fn working_headers() -> Headers {
        headers(&[
            ("profile-title", &b64_header("🛡️ proxysvpn.com")),
            ("subscription-userinfo", "upload=0; download=0; total=0; expire=1760400000"),
            ("sub-info-text", "0"),
            ("support-url", "https://t.me/proxysvpn_bot"),
            ("routing-enable", "true"),
            ("profile-update-interval", "12"),
            ("content-type", "text/plain; charset=utf-8"),
        ])
    }

    // ── Headers ────────────────────────────────────────────────────────────

    #[test]
    fn headers_are_parsed_into_meta() {
        let h = headers(&[
            ("profile-title", &b64_header("🛡️ proxysvpn.com")),
            ("announce", &b64_header("Идут работы на серверах")),
            ("announce-url", "https://proxysvnovich.vercel.app/login"),
            ("subscription-userinfo", "upload=0; download=0; total=0; expire=1760400000"),
            ("sub-info-text", &b64_header("Ваш баланс на нуле.")),
            ("support-url", "https://t.me/proxysvpn_bot"),
            ("routing-enable", "true"),
            ("profile-update-interval", "12"),
        ]);
        let meta = parse_meta(&h);
        assert_eq!(meta.title.as_deref(), Some("🛡️ proxysvpn.com"));
        assert_eq!(meta.announce.as_deref(), Some("Идут работы на серверах"));
        assert_eq!(
            meta.announce_url.as_deref(),
            Some("https://proxysvnovich.vercel.app/login")
        );
        assert_eq!(meta.expires_at, Some(1_760_400_000));
        assert_eq!(meta.info_text.as_deref(), Some("Ваш баланс на нуле."));
        assert_eq!(meta.support_url.as_deref(), Some("https://t.me/proxysvpn_bot"));
        assert_eq!(meta.routing_enabled, Some(true));
        assert_eq!(meta.update_interval_hours, Some(12));
    }

    #[test]
    fn sticky_info_reset_is_not_a_message() {
        // `balanceInfoHeaders(false)` sends "0" to clear the block, because an
        // empty header value crashes the response on the server side.
        let meta = parse_meta(&headers(&[("sub-info-text", "0")]));
        assert!(meta.info_text.is_none());
    }

    #[test]
    fn missing_headers_leave_meta_empty() {
        let meta = parse_meta(&headers(&[]));
        assert!(meta.title.is_none());
        assert!(meta.expires_at.is_none());
        assert!(meta.routing_enabled.is_none());
    }

    #[test]
    fn expire_zero_is_no_date_not_nineteen_seventy() {
        assert_eq!(parse_expire("upload=0; download=0; total=0; expire=0"), None);
        assert_eq!(parse_expire("expire=1760400000"), Some(1_760_400_000));
        assert_eq!(parse_expire("upload=0; download=0"), None);
        assert_eq!(parse_expire("expire=not-a-number"), None);
    }

    #[test]
    fn routing_enable_reads_both_spellings() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn broken_base64_header_does_not_take_the_response_down() {
        let meta = parse_meta(&headers(&[("profile-title", "base64:!!!not-base64!!!")]));
        assert!(meta.title.is_none());
    }

    // ── Refusal stubs ──────────────────────────────────────────────────────

    fn refusal_of(body_line: &str, extra: &[(&str, &str)]) -> AppError {
        let mut h = working_headers();
        for (k, v) in extra {
            h.insert(k.to_ascii_lowercase(), v.to_string());
        }
        let body = b64(body_line);
        interpret_response("proxysvpn.com", 200, &h, &body, 1_760_000_000)
            .expect_err("a stub is never a server list")
    }

    #[test]
    fn empty_balance_stub_is_named_and_keeps_the_server_text() {
        let err = refusal_of(
            &stub_line("Пополните баланс - proxysvpn.com"),
            &[
                ("profile-title", &b64_header("⚠️ Нет средств")),
                ("sub-info-text", &b64_header("Ваш баланс на нуле. Пополните, чтобы продолжить пользоваться сервисом.")),
            ],
        );
        assert_eq!(err.code, ErrorCode::BalanceEmpty);
        assert!(err.detail.as_deref().unwrap_or("").contains("баланс"));
    }

    #[test]
    fn empty_balance_stub_is_named_in_every_language() {
        for title in BALANCE_EMPTY_TITLES {
            let err = refusal_of(
                &stub_line("Top up: proxysvnovich.vercel.app"),
                &[("profile-title", &b64_header(title))],
            );
            assert_eq!(err.code, ErrorCode::BalanceEmpty, "title {title}");
        }
    }

    #[test]
    fn second_device_stub_is_named() {
        let err = refusal_of(
            &stub_line("Сбросьте привязку в боте"),
            &[
                ("profile-title", &b64_header("Сбросьте привязку")),
                ("announce", &b64_header("Ссылка уже привязана к другому устройству. Откройте бот → «📡 Мои устройства».")),
            ],
        );
        assert_eq!(err.code, ErrorCode::DeviceTaken);
        // The cure is inside the server's sentence; we must not drop it.
        assert!(err.detail.as_deref().unwrap_or("").contains("Мои устройства"));
    }

    #[test]
    fn deleted_link_stub_is_named() {
        let err = refusal_of(
            &stub_line("Возьмите новую ссылку в боте"),
            &[
                ("profile-title", &b64_header("Ссылка недействительна")),
                ("announce", &b64_header("Эта ссылка больше не работает: устройство удалено или ссылка устарела.")),
            ],
        );
        assert_eq!(err.code, ErrorCode::NoDevices);
    }

    #[test]
    fn stub_with_a_past_expiry_and_no_marker_reads_as_expired() {
        let err = refusal_of(
            &stub_line("Продлите доступ"),
            &[
                ("profile-title", &b64_header("Доступ приостановлен")),
                ("subscription-userinfo", "upload=0; download=0; total=0; expire=1750000000"),
                ("announce", &b64_header("Доступ приостановлен.")),
            ],
        );
        assert_eq!(err.code, ErrorCode::Expired);
    }

    #[test]
    fn unknown_stub_is_a_notice_not_a_parser_bug() {
        let err = refusal_of(
            &stub_line("Технические работы"),
            &[("profile-title", &b64_header("Сервис недоступен"))],
        );
        assert_eq!(err.code, ErrorCode::SubNotice);
        assert_eq!(err.detail.as_deref(), Some("Технические работы"));
    }

    #[test]
    fn a_refusal_never_carries_a_node_address() {
        let err = refusal_of(
            &stub_line("Пополните баланс - proxysvpn.com"),
            &[("profile-title", &b64_header("⚠️ Нет средств"))],
        );
        let detail = err.detail.unwrap_or_default();
        assert!(!detail.contains("127.0.0.1"), "{detail}");
        assert!(!detail.contains("vless://"), "{detail}");
    }

    #[test]
    fn a_stub_is_recognised_by_address_not_by_wording() {
        assert_eq!(
            stub_notice(&stub_line("что угодно")).as_deref(),
            Some("что угодно")
        );
        assert!(stub_notice(VLESS_LINE).is_none());
        assert!(stub_notice(HY2_LINE).is_none());
    }

    // ── Ports ──────────────────────────────────────────────────────────────

    #[test]
    fn port_hopping_list_keeps_the_location_alive() {
        // The exact shape the live subscription serves since 19.09.2026.
        let cfg = parse_hy2_url(HY2_LINE).expect("hop list must parse");
        assert_eq!(cfg.port, 443, "the plain port comes first in the list");
        assert_eq!(cfg.host, "nl.example.net");
        assert_eq!(cfg.remark, "Нидерланды");
        assert!(cfg.insecure, "a pin implies the standard check is skipped");
    }

    #[test]
    fn bare_range_falls_back_to_its_lower_bound() {
        // The 17.09-19.09 form, still alive in old stored links.
        let raw = "hy2://pass@node.example.net:20000-50000/?sni=a.b#X";
        let cfg = parse_hy2_url(raw).expect("bare range must still connect");
        assert_eq!(cfg.port, 20_000);
    }

    #[test]
    fn plain_port_is_left_alone() {
        let (out, hop) = normalize_port(HY2_LINE.replace(":443,20000-50000", ":443").as_str())
            .expect("plain port");
        assert!(hop.is_none());
        assert!(out.contains(":443/"));
    }

    #[test]
    fn port_helper_reads_every_documented_form() {
        assert_eq!(first_port_of("443,20000-50000"), Some(443));
        assert_eq!(first_port_of("20000-50000"), Some(20_000));
        assert_eq!(first_port_of("443"), Some(443));
        assert_eq!(first_port_of("8443-8445,443"), Some(8443));
        assert_eq!(first_port_of(""), None);
        assert_eq!(first_port_of("0"), None);
        assert_eq!(first_port_of("abc"), None);
        assert_eq!(first_port_of("99999"), None);
    }

    #[test]
    fn a_password_with_a_colon_does_not_confuse_the_port() {
        let raw = "hy2://user:pa:ss@node.example.net:443,20000-50000/?sni=a.b#X";
        let (out, hop) = normalize_port(raw).expect("userinfo colon");
        assert_eq!(hop.as_deref(), Some("443,20000-50000"));
        assert!(out.ends_with(":443/?sni=a.b#X"), "{out}");
    }

    #[test]
    fn a_link_without_a_port_is_left_for_the_url_parser() {
        let raw = "vless://uuid@node.example.net?pbk=K";
        let (out, hop) = normalize_port(raw).expect("no port is not a crash");
        assert_eq!(out, raw);
        assert!(hop.is_none());
        assert_eq!(
            parse_vless_url(raw).expect_err("a portless node cannot be dialled").code,
            ErrorCode::SubMalformed
        );
    }

    // ── Bodies ─────────────────────────────────────────────────────────────

    #[test]
    fn a_working_subscription_is_read_whole() {
        let body = b64(&format!(
            "{}\n{}\n{}",
            routing_line(INLINE_PROFILE),
            VLESS_LINE,
            HY2_LINE
        ));
        let sub = interpret_response(
            "proxysvpn.com",
            200,
            &working_headers(),
            &body,
            1_760_000_000,
        )
        .expect("a real body");

        assert_eq!(sub.servers.len(), 2);
        assert_eq!(sub.servers[0].proto(), "VLESS");
        assert_eq!(sub.servers[0].remark(), "Германия");
        assert_eq!(sub.servers[1].proto(), "Hysteria2");
        assert_eq!(sub.unreadable_lines, 0);
        assert_eq!(sub.source_host, "proxysvpn.com");
        assert_eq!(sub.meta.expires_at, Some(1_760_400_000));
        assert_eq!(sub.meta.routing_enabled, Some(true));
        assert!(sub.routing.is_some(), "the service's rules must arrive with it");
    }

    #[test]
    fn a_plain_text_body_is_read_too() {
        // `?raw=1` exists because base64 is itself the obstacle when a person
        // has to move the links by hand.
        let raw = format!("{VLESS_LINE}\n{HY2_LINE}");
        let sub = interpret_response("proxysvpn.com", 200, &working_headers(), &raw, 0)
            .expect("raw body");
        assert_eq!(sub.servers.len(), 2);
    }

    #[test]
    fn an_unreadable_line_is_counted_not_hidden() {
        let broken = "vless://11111111-2222-3333-4444-555555555555@de.example.net:443?encryption=none#NoPublicKey";
        let body = b64(&format!("{VLESS_LINE}\n{broken}\n{HY2_LINE}"));
        let sub = interpret_response("proxysvpn.com", 200, &working_headers(), &body, 0)
            .expect("two good servers");
        assert_eq!(sub.servers.len(), 2);
        assert_eq!(
            sub.unreadable_lines, 1,
            "a location must never disappear in silence"
        );
    }

    #[test]
    fn an_empty_body_is_named_empty() {
        let err = interpret_response("proxysvpn.com", 200, &working_headers(), "   ", 0)
            .expect_err("nothing to connect to");
        assert_eq!(err.code, ErrorCode::SubEmpty);
    }

    #[test]
    fn a_captive_portal_page_is_not_a_subscription() {
        let err = interpret_response(
            "proxysvpn.com",
            200,
            &working_headers(),
            "<html><body>Please sign in to the hotel Wi-Fi</body></html>",
            0,
        )
        .expect_err("html is not a subscription");
        assert_eq!(err.code, ErrorCode::SubInvalid);
    }

    #[test]
    fn a_body_of_only_unreadable_links_is_invalid() {
        let broken = "vless://uuid@node.example.net:443?encryption=none#NoKey";
        let err = interpret_response("proxysvpn.com", 200, &working_headers(), &b64(broken), 0)
            .expect_err("nothing usable");
        assert_eq!(err.code, ErrorCode::SubInvalid);
    }

    #[test]
    fn base64_wrapping_is_optional_and_detected() {
        let wrapped = b64(VLESS_LINE);
        assert!(decode_subscription_body(&wrapped).starts_with("vless://"));
        assert_eq!(decode_subscription_body(VLESS_LINE), VLESS_LINE);
    }

    // ── HTTP statuses ──────────────────────────────────────────────────────

    #[test]
    fn a_dead_link_is_named_not_shrugged_at() {
        let err = interpret_response(
            "proxysvpn.com",
            404,
            &headers(&[]),
            "Token not found",
            0,
        )
        .expect_err("404");
        assert_eq!(err.code, ErrorCode::NoDevices);
        // The body is an internal English tag; the person gets the translated
        // phrase for the code instead.
        assert!(err.detail.is_none());
    }

    #[test]
    fn the_hosting_shield_is_not_the_users_provider() {
        let err = interpret_response(
            "proxysvpn.com",
            403,
            &headers(&[("x-vercel-mitigated", "challenge")]),
            "",
            0,
        )
        .expect_err("403 from the shield");
        assert_eq!(err.code, ErrorCode::SubUnreachable);

        let plain = interpret_response("proxysvpn.com", 403, &headers(&[]), "Invalid token", 0)
            .expect_err("403 from the backend");
        assert_eq!(plain.code, ErrorCode::SubMalformed);
    }

    #[test]
    fn rate_limiting_reads_as_try_again() {
        let err = interpret_response(
            "proxysvpn.com",
            429,
            &headers(&[("retry-after", "47")]),
            "Too many requests",
            0,
        )
        .expect_err("429");
        assert_eq!(err.code, ErrorCode::SubUnreachable);
    }

    #[test]
    fn a_server_fault_with_a_passed_expiry_says_expired() {
        let h = headers(&[(
            "subscription-userinfo",
            "upload=0; download=0; total=0; expire=1750000000",
        )]);
        let err = interpret_response("proxysvpn.com", 500, &h, "Server error", 1_760_000_000)
            .expect_err("500");
        assert_eq!(err.code, ErrorCode::Expired);
    }

    #[test]
    fn a_final_answer_stops_the_ladder_and_a_shield_does_not() {
        let answer = |status: u16, h: Headers, body: &str| HostAnswer {
            url: "https://proxysvpn.com/api/sub/t".into(),
            host: "proxysvpn.com".into(),
            status,
            headers: h,
            body: body.into(),
        };
        assert!(answer(200, headers(&[]), "body").is_final());
        assert!(!answer(200, headers(&[]), "  ").is_final());
        assert!(answer(404, headers(&[]), "Token not found").is_final());
        assert!(answer(403, headers(&[]), "Invalid token").is_final());
        assert!(!answer(403, headers(&[("x-vercel-mitigated", "x")]), "").is_final());
        assert!(!answer(429, headers(&[]), "").is_final());
        assert!(!answer(502, headers(&[]), "").is_final());
    }

    // ── The ladder ─────────────────────────────────────────────────────────

    #[test]
    fn the_ladder_keeps_the_path_and_the_token() {
        let urls = candidate_urls("https://proxysvpn.com/api/sub/abcdef0123456789").unwrap();
        assert!(urls.len() >= 4);
        for url in &urls {
            assert!(
                url.contains("/api/sub/abcdef0123456789"),
                "the token never changes: {url}"
            );
            assert!(url.contains("format=vless"), "{url}");
            assert!(url.contains("routing=inline"), "{url}");
        }
        assert!(urls[0].starts_with("https://proxysvpn.com/"));
        assert!(urls[1].starts_with("https://proxysvnovich.vercel.app/"));
    }

    #[test]
    fn a_link_already_on_a_reserve_does_not_repeat_it() {
        let urls =
            candidate_urls("https://proxysvnovich.vercel.app/api/sub/token12345678").unwrap();
        let first_hosts: Vec<String> = urls
            .iter()
            .filter_map(|u| Url::parse(u).ok()?.host_str().map(str::to_string))
            .collect();
        let mut seen = first_hosts.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), first_hosts.len(), "{first_hosts:?}");
        assert_eq!(first_hosts[0], "proxysvnovich.vercel.app");
        assert!(first_hosts.contains(&"proxysvpn.com".to_string()));
    }

    #[test]
    fn the_persons_own_query_survives_and_ours_wins() {
        let urls =
            candidate_urls("https://proxysvpn.com/api/sub/tok12345?split=0&lang=en&format=xray")
                .unwrap();
        let first = &urls[0];
        assert!(first.contains("split=0"), "{first}");
        assert!(first.contains("lang=en"), "{first}");
        assert!(first.contains("format=vless"), "{first}");
        assert!(!first.contains("format=xray"), "{first}");
    }

    #[test]
    fn the_retry_drops_only_the_inline_flag() {
        let url = "https://proxysvpn.com/api/sub/tok12345?lang=en&format=vless&routing=inline";
        let retry = without_inline_routing(url).expect("retry url");
        assert!(!retry.contains("routing="), "{retry}");
        assert!(retry.contains("format=vless"), "{retry}");
        assert!(retry.contains("lang=en"), "{retry}");
    }

    #[test]
    fn only_an_unreadable_body_is_worth_a_second_request() {
        assert!(retry_without_inline(&AppError::new(ErrorCode::SubInvalid)));
        assert!(retry_without_inline(&AppError::new(ErrorCode::SubEmpty)));
        for code in [
            ErrorCode::BalanceEmpty,
            ErrorCode::DeviceTaken,
            ErrorCode::NoDevices,
            ErrorCode::SubUnreachable,
            ErrorCode::SubMalformed,
        ] {
            assert!(!retry_without_inline(&AppError::new(code)), "{code:?}");
        }
    }

    #[test]
    fn a_link_that_is_not_ours_is_refused_before_any_request() {
        assert_eq!(
            candidate_urls("").expect_err("empty").code,
            ErrorCode::NoSubscription
        );
        assert_eq!(
            candidate_urls("   ").expect_err("blank").code,
            ErrorCode::NoSubscription
        );
        assert_eq!(
            candidate_urls("not a url").expect_err("garbage").code,
            ErrorCode::SubMalformed
        );
        assert_eq!(
            candidate_urls("ftp://proxysvpn.com/api/sub/x").expect_err("scheme").code,
            ErrorCode::SubMalformed
        );
    }

    /// A closed port on loopback is the cheapest honest stand-in for "the
    /// network dropped us": no DNS, no traffic, and the same reqwest error
    /// path a filtered address produces.
    #[tokio::test]
    async fn a_dead_network_reads_as_unreachable() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(800))
            .build()
            .expect("client");
        let err = try_host(&client, "http://127.0.0.1:1/api/sub/token12345678")
            .await
            .expect_err("nothing listens on port 1");
        assert_eq!(err.code, ErrorCode::SubUnreachable);
        // Never the URL: it carries the subscription token.
        assert!(err.detail.is_none(), "{:?}", err.detail);
    }

    // ── Routing ────────────────────────────────────────────────────────────

    #[test]
    fn the_services_routing_profile_becomes_our_rules() {
        let rules = parse_routing_line(&routing_line(INLINE_PROFILE)).expect("profile");
        assert!(rules.direct_domains.contains(&"domain:taximaxim.ru".to_string()));
        assert!(rules.direct_domains.contains(&"domain:max.ru".to_string()));
        // `full:` is a rule form in both worlds and must survive untouched.
        assert!(rules
            .direct_domains
            .contains(&"full:mvideo.edna.io".to_string()));
        // A geo category we cannot verify inside the bundled .dat would stop
        // the core from starting at all.
        assert!(!rules
            .direct_domains
            .iter()
            .any(|d| d.starts_with("geosite:")));
        assert!(rules.direct_ips.contains(&"87.240.128.0/18".to_string()));
        assert!(rules.direct_ips.contains(&"geoip:ru".to_string()));
        assert!(!rules.direct_ips.contains(&"geoip:de".to_string()));
        assert_eq!(rules.domain_strategy, "IPIfNonMatch");
        assert!(!rules.is_global());
    }

    #[test]
    fn the_global_profile_is_not_mistaken_for_no_rules() {
        let global = r#"{"Name":"ProxysVPN Global 1","DirectSites":[],"DirectIp":["10.0.0.0/8"],"DomainStrategy":"AsIs"}"#;
        let rules = parse_routing_line(&routing_line(global)).expect("global profile");
        assert!(rules.is_global());
        assert_eq!(rules.domain_strategy, "AsIs");
    }

    #[test]
    fn turning_split_routing_off_is_not_forgetting_the_rules() {
        // `?split=0` is how someone abroad says "send everything through the
        // tunnel". The response then carries the header and NO profile line,
        // and an app that merely kept its previous rules would go on sending
        // Russian sites direct from Berlin.
        let sub = Subscription {
            servers: Vec::new(),
            meta: SubMeta {
                routing_enabled: Some(false),
                ..SubMeta::default()
            },
            routing: None,
            unreadable_lines: 0,
            source_host: "proxysvpn.com".into(),
        };
        remember_routing(parse_routing_line(&routing_line(INLINE_PROFILE)));
        adopt_routing(&sub);
        let in_force = last_routing().expect("something must be in force");
        assert!(in_force.is_global(), "{in_force:?}");
        assert!(in_force.direct_domains.is_empty());
    }

    #[test]
    fn a_damaged_routing_line_is_ignored_not_fatal() {
        assert!(parse_routing_line(&format!("{ROUTING_PREFIX}!!!")).is_none());
        assert!(parse_routing_line(&format!("{ROUTING_PREFIX}{}", b64("not json"))).is_none());
        assert!(parse_routing_line(VLESS_LINE).is_none());
    }

    #[test]
    fn a_body_whose_routing_line_is_broken_still_yields_servers() {
        let body = b64(&format!("{ROUTING_PREFIX}!!!\n{VLESS_LINE}"));
        let sub = interpret_response("proxysvpn.com", 200, &working_headers(), &body, 0)
            .expect("servers survive a bad profile");
        assert_eq!(sub.servers.len(), 1);
        assert!(sub.routing.is_none());
    }

    // ── xray config ────────────────────────────────────────────────────────

    fn vless_fixture() -> VlessConfig {
        parse_vless_url(VLESS_LINE).expect("fixture")
    }

    /// The XHTTP line exactly as the service builds it (`buildVlessUrl` in
    /// frontend `lib/xpanel.ts`): `/?type=xhttp`, no `flow`, and the path,
    /// mode and host of `xhttpLinkParams`, percent-encoded.
    const XHTTP_LINE: &str = "vless://11111111-2222-3333-4444-555555555555@uk.example.net:2053/?type=xhttp&security=reality&pbk=PUBKEY&fp=firefox&sni=www.nhs.uk&sid=ab12&spx=%2F&path=%2F87588135c873&mode=stream-one#%D0%91%D1%80%D0%B8%D1%82%D0%B0%D0%BD%D0%B8%D1%8F%20%C2%B7%20XHTTP";

    fn xhttp_fixture() -> VlessConfig {
        parse_vless_url(XHTTP_LINE).expect("xhttp fixture")
    }

    #[test]
    fn an_xhttp_line_is_read_as_xhttp_with_its_path_and_mode() {
        let cfg = xhttp_fixture();
        assert_eq!(
            cfg.transport,
            VlessTransport::Xhttp { path: "/87588135c873".into(), mode: XhttpMode::StreamOne, host: None }
        );
        assert_eq!(cfg.port, 2053);
        assert_eq!(cfg.sni, "www.nhs.uk");
        assert_eq!(cfg.fingerprint, "firefox");
        assert_eq!(cfg.remark, "Британия · XHTTP");
        assert_eq!(cfg.flow, "");
    }

    #[test]
    fn a_tcp_line_and_a_line_without_type_stay_tcp() {
        assert_eq!(vless_fixture().transport, VlessTransport::Tcp);
        let no_type = VLESS_LINE.replace("&type=tcp", "");
        assert_eq!(parse_vless_url(&no_type).expect("parses").transport, VlessTransport::Tcp);
        assert_eq!(parse_vless_url(&no_type).expect("parses").flow, "xtls-rprx-vision");
    }

    #[test]
    fn vision_never_rides_on_xhttp() {
        let line = XHTTP_LINE.replace("&mode=", "&flow=xtls-rprx-vision&mode=");
        let cfg = parse_vless_url(&line).expect("parses");
        assert_eq!(cfg.flow, "");
        let built = build_xray_config_with_routing(&cfg, None);
        assert!(built["outbounds"][0]["settings"]["vnext"][0]["users"][0].get("flow").is_none());
    }

    #[test]
    fn host_is_kept_when_valid_and_refused_when_not() {
        let with_host = format!("{}", XHTTP_LINE.replace("&mode=", "&host=cdn.example.com&mode="));
        match parse_vless_url(&with_host).expect("parses").transport {
            VlessTransport::Xhttp { host, .. } => assert_eq!(host.as_deref(), Some("cdn.example.com")),
            other => panic!("expected xhttp, got {other:?}"),
        }
        let empty_host = XHTTP_LINE.replace("&mode=", "&host=&mode=");
        match parse_vless_url(&empty_host).expect("parses").transport {
            VlessTransport::Xhttp { host, .. } => assert_eq!(host, None),
            other => panic!("expected xhttp, got {other:?}"),
        }
        for bad in ["-bad.example", "a..b", "space%20host", "under_score.example"] {
            let line = XHTTP_LINE.replace("&mode=", &format!("&host={bad}&mode="));
            assert!(parse_vless_url(&line).is_err(), "host {bad} accepted");
        }
    }

    #[test]
    fn a_missing_mode_is_what_xray_does_without_one() {
        let line = XHTTP_LINE.replace("&mode=stream-one", "");
        match parse_vless_url(&line).expect("parses").transport {
            VlessTransport::Xhttp { mode, .. } => assert_eq!(mode, XhttpMode::Auto),
            other => panic!("expected xhttp, got {other:?}"),
        }
    }

    #[test]
    fn a_broken_xhttp_line_is_unreadable_not_a_tcp_server() {
        for (what, line) in [
            ("no path", XHTTP_LINE.replace("&path=%2F87588135c873", "")),
            ("path without slash", XHTTP_LINE.replace("path=%2F87588135c873", "path=87588135c873")),
            ("path with a space", XHTTP_LINE.replace("path=%2F87588135c873", "path=%2Fa%20b")),
            ("path with a query", XHTTP_LINE.replace("path=%2F87588135c873", "path=%2Fa%3Fb%3D1")),
            ("unknown mode", XHTTP_LINE.replace("mode=stream-one", "mode=fast")),
            ("ws", XHTTP_LINE.replace("type=xhttp", "type=ws")),
            ("grpc", VLESS_LINE.replace("type=tcp", "type=grpc")),
        ] {
            assert!(parse_vless_url(&line).is_err(), "{what} accepted");
        }
        let long = format!("/{}", "a".repeat(128));
        let line = XHTTP_LINE.replace("%2F87588135c873", &long);
        assert!(parse_vless_url(&line).is_err(), "129-character path accepted");
    }

    #[test]
    fn the_xhttp_config_carries_xhttp_settings_and_keeps_reality() {
        let cfg = build_xray_config_with_routing(&xhttp_fixture(), None);
        let stream = &cfg["outbounds"][0]["streamSettings"];
        assert_eq!(cfg["outbounds"][0]["tag"], "proxy");
        assert_eq!(stream["network"], "xhttp");
        assert_eq!(stream["xhttpSettings"]["path"], "/87588135c873");
        assert_eq!(stream["xhttpSettings"]["mode"], "stream-one");
        assert!(stream["xhttpSettings"].get("host").is_none());
        assert!(stream["xhttpSettings"].get("xmux").is_none(), "xmux is an experiment, not a default");
        assert_eq!(stream["security"], "reality");
        assert_eq!(stream["realitySettings"]["serverName"], "www.nhs.uk");
        assert_eq!(stream["realitySettings"]["fingerprint"], "firefox");
    }

    #[test]
    fn the_tcp_config_is_what_it_was() {
        let cfg = build_xray_config_with_routing(&vless_fixture(), None);
        let stream = &cfg["outbounds"][0]["streamSettings"];
        assert_eq!(stream["network"], "tcp");
        assert!(stream.get("xhttpSettings").is_none());
        assert_eq!(cfg["outbounds"][0]["settings"]["vnext"][0]["users"][0]["flow"], "xtls-rprx-vision");
    }

    #[test]
    fn fragmenting_works_the_same_over_xhttp() {
        use crate::tunnel_prefs::TunnelPrefs;
        let prefs = TunnelPrefs { fragment: true, ..Default::default() };
        let cfg = build_xray_config_with_routing_and_prefs(&xhttp_fixture(), None, &prefs);
        assert_eq!(cfg["outbounds"][0]["streamSettings"]["sockopt"]["dialerProxy"], "fragment");
        assert_eq!(cfg["outbounds"][0]["streamSettings"]["network"], "xhttp");
    }

    /// Live check, off by default: builds the app's config for a real link and
    /// writes it out, so the bundled xray can be run against a real node.
    /// `WATAFAST_LIVE_LINK=vless://… WATAFAST_LIVE_OUT=/path.json cargo test
    /// --lib live_config -- --ignored`. The link is read from the environment,
    /// never printed.
    #[test]
    #[ignore]
    fn live_config() {
        let link = std::env::var("WATAFAST_LIVE_LINK").expect("WATAFAST_LIVE_LINK");
        let out = std::env::var("WATAFAST_LIVE_OUT").expect("WATAFAST_LIVE_OUT");
        let cfg = build_xray_config_with_routing(&parse_vless_url(&link).expect("link parses"), None);
        std::fs::write(&out, serde_json::to_string_pretty(&cfg).expect("serializes")).expect("written");
    }

    #[test]
    fn the_system_resolver_inside_the_tunnel_is_answered_not_sent_direct() {
        let built = build_xray_config_with_routing(&vless_fixture(), None);
        let rules = built["routing"]["rules"].as_array().expect("rules");
        let ours = rules
            .iter()
            .position(|r| r["ip"].as_array().is_some_and(|a| a.iter().any(|v| v == TUNNEL_DNS)))
            .expect("a rule for the tunnel resolver");
        let private = rules
            .iter()
            .position(|r| r["ip"].as_array().is_some_and(|a| a.iter().any(|v| v == "geoip:private")))
            .expect("the private-network rule");
        assert_eq!(rules[ours]["outboundTag"], "dns-out");
        assert!(ours < private, "sent direct, the resolver would go nowhere");
    }

    #[test]
    fn non_address_queries_go_to_a_real_server_through_the_node() {
        for cfg in [vless_fixture(), xhttp_fixture()] {
            let built = build_xray_config_with_routing(&cfg, None);
            let dns_out = built["outbounds"]
                .as_array()
                .expect("outbounds")
                .iter()
                .find(|o| o["tag"] == "dns-out")
                .expect("dns-out exists");
            // REFUSED on type 65 is what took the whole of DNS down on macOS.
            assert_eq!(dns_out["settings"]["nonIPQuery"], "skip");
            assert_eq!(dns_out["settings"]["network"], "tcp");
            assert_eq!(dns_out["settings"]["address"], "1.1.1.1");
            assert_eq!(dns_out["settings"]["port"], 53);
            // Through the node: not back into the tunnel, not past it.
            assert_eq!(dns_out["proxySettings"]["tag"], "proxy");
            assert!(dns_out.get("streamSettings").is_none(), "dialerProxy lost answers in the measurement");
        }
    }

    #[test]
    fn every_kind_of_entry_names_its_protocol() {
        assert_eq!(ServerConfig::Vless(vless_fixture()).protocol_label(), "VLESS · Vision");
        assert_eq!(ServerConfig::Vless(xhttp_fixture()).protocol_label(), "VLESS · XHTTP");
        let plain = parse_vless_url(&VLESS_LINE.replace("&flow=xtls-rprx-vision", "")).expect("parses");
        assert_eq!(ServerConfig::Vless(plain).protocol_label(), "VLESS · TCP");
        let hy2 = parse_hy2_url("hysteria2://secret@nl.example.net:443?sni=www.bing.com&insecure=1#NL")
            .expect("hy2 parses");
        assert_eq!(ServerConfig::Hy2(hy2).protocol_label(), "Hysteria2");
    }

    #[test]
    fn a_sing_box_build_with_only_xhttp_entries_says_so_and_does_not_retry() {
        let body = b64(XHTTP_LINE);
        let err = interpret_response_on("proxysvpn.com", 200, &working_headers(), &body, 0, true)
            .expect_err("nothing this engine can run");
        assert_eq!(err.code, ErrorCode::EngineUnsupported);
        assert!(!retry_without_inline(&err), "asking again brings the same list");

        // The same body on the xray build is a normal one-server list.
        let sub = interpret_response_on("proxysvpn.com", 200, &working_headers(), &body, 0, false)
            .expect("xray runs it");
        assert_eq!(sub.servers.len(), 1);
    }

    #[test]
    fn a_sing_box_build_keeps_its_tcp_entries_and_counts_nothing_unreadable() {
        let body = b64(&format!("{VLESS_LINE}\n{XHTTP_LINE}"));
        let sub = interpret_response_on("proxysvpn.com", 200, &working_headers(), &body, 0, true)
            .expect("the TCP entry remains");
        assert_eq!(sub.servers.len(), 1);
        assert_eq!(sub.servers[0].remark(), "Германия");
        assert_eq!(sub.unreadable_lines, 0, "a skipped entry is not an unreadable one");
    }

    #[test]
    fn the_sing_box_build_leaves_xhttp_out_and_keeps_the_rest() {
        let xhttp = ServerConfig::Vless(xhttp_fixture());
        let tcp = ServerConfig::Vless(vless_fixture());
        assert!(engine_supports_on(&xhttp, false), "xray runs it");
        assert!(!engine_supports_on(&xhttp, true), "sing-box would build it as TCP");
        assert!(engine_supports_on(&tcp, true));
        assert!(engine_supports_on(&tcp, false));
    }

    #[test]
    fn the_config_prefers_the_services_rules_over_ours() {
        let rules = parse_routing_line(&routing_line(INLINE_PROFILE)).expect("profile");
        let cfg = build_xray_config_with_routing(&vless_fixture(), Some(&rules));
        let text = cfg.to_string();
        assert!(text.contains("domain:taximaxim.ru"), "server rule missing");
        // A domain that exists only in our seed must NOT appear when the
        // service has spoken — that is what "one list" means.
        assert!(!text.contains("domain:kinopoisk.ru"), "seed leaked in");
        assert_eq!(cfg["routing"]["domainStrategy"], "IPIfNonMatch");
    }

    #[test]
    fn without_rules_the_seed_keeps_russian_sites_direct() {
        let cfg = build_xray_config_with_routing(&vless_fixture(), None);
        let text = cfg.to_string();
        assert!(text.contains("domain:gosuslugi.ru"));
        assert!(text.contains("geoip:ru"));
    }

    #[test]
    fn the_global_profile_sends_everything_through_the_tunnel() {
        let rules = RoutingRules {
            direct_domains: Vec::new(),
            direct_ips: vec!["10.0.0.0/8".into()],
            domain_strategy: "AsIs".into(),
        };
        let cfg = build_xray_config_with_routing(&vless_fixture(), Some(&rules));
        let text = cfg.to_string();
        assert!(!text.contains("domain:gosuslugi.ru"), "seed must not creep back");
        // The LAN still never crosses a border, whatever the profile says.
        assert!(text.contains("geoip:private"));
    }

    #[test]
    fn quic_stays_blocked_in_every_shape_of_config() {
        for routing in [None, Some(&RoutingRules::default())] {
            let cfg = build_xray_config_with_routing(&vless_fixture(), routing);
            let first = &cfg["routing"]["rules"][0];
            assert_eq!(first["outboundTag"], "block");
            assert_eq!(first["network"], "udp");
            assert_eq!(first["port"], "443");
        }
    }

    /// У xray обязан быть СВОЙ резолвер, и его запросы - идти через узел.
    ///
    /// Без этого xray спрашивал имя системным резолвером, запрос уходил в
    /// маршрут по умолчанию (то есть обратно в туннель, к самому xray) и
    /// разрешался только по таймеру. Замер 22.09.2026: 1,2 секунды на каждое
    /// новое соединение при том, что само имя разрешается за 2 мс.
    #[test]
    fn xray_resolves_names_through_the_node_not_through_the_tunnel() {
        let cfg = build_xray_config_with_routing(&vless_fixture(), None);

        let dns = &cfg["dns"];
        assert!(!dns.is_null(), "раздела dns нет - xray снова пойдёт в систему");
        assert_eq!(
            dns["queryStrategy"], "UseIPv4",
            "IPv6-выхода у узлов нет, и запрос AAAA - это ожидание впустую"
        );
        assert_eq!(dns["tag"], "dns-in", "без метки правило маршрутизации не сработает");

        // Транспорт резолвера обязан быть TCP-совместимым: запросы идут через
        // `proxy`, а Vision несёт только TCP - поэтому голый UDP-адрес здесь
        // означает мёртвые первые секунды после подъёма туннеля.
        for server in dns["servers"].as_array().expect("серверы перечислены") {
            let addr = server.as_str().expect("адрес строкой");
            assert!(
                addr.starts_with("https://") || addr.starts_with("tcp://"),
                "{addr}: резолвер по UDP не пройдёт через выход, который несёт только TCP"
            );
        }

        let rules = cfg["routing"]["rules"].as_array().expect("правила есть");
        let own = rules
            .iter()
            .find(|r| r["inboundTag"][0] == "dns-in")
            .expect("собственные запросы xray должны иметь своё правило");
        assert_eq!(
            own["outboundTag"], "proxy",
            "запросы имён самого xray обязаны идти через узел, иначе петля"
        );

        // И этот же outbound должен существовать, иначе xray не стартует.
        let outbounds = cfg["outbounds"].as_array().expect("исходящие есть");
        assert!(
            outbounds.iter().any(|o| o["tag"] == "dns-out" && o["protocol"] == "dns"),
            "правило указывает на dns-out, а его в конфигурации нет"
        );

        // Порядок: частные сети раньше перехвата порта 53, иначе запрос к
        // домашнему роутеру уедет за границу.
        let private_at = rules.iter().position(|r| r["ip"][0] == "geoip:private");
        let dns_port_at = rules.iter().position(|r| r["port"] == 53);
        assert!(
            private_at < dns_port_at,
            "локальный резолвер должен остаться локальным"
        );
    }

    /// Конфигурацию принимает НАСТОЯЩИЙ xray, а не только наши ожидания.
    ///
    /// Тест знает, чего мы хотим; бинарь знает, что он примет. Второе важнее:
    /// 22.09.2026 мы уже отправляли в него раздел `dns` с голыми UDP-адресами
    /// - он их принял, а работать не стало.
    ///
    /// Ключ REALITY генерируется тем же бинарём: в образце стоит заглушка,
    /// которую xray справедливо отвергает.
    #[test]
    fn xray_itself_accepts_every_combination_we_can_produce() {
        use crate::tunnel_prefs::{DnsChoice, IpKind, TunnelPrefs};
        use std::process::Command;

        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/xray-aarch64-apple-darwin");
        if !bin.exists() {
            // На чужой машине бинаря может не быть - это не повод падать.
            eprintln!("xray рядом не найден, проверка на живом бинаре пропущена");
            return;
        }

        let keys = Command::new(&bin).arg("x25519").output().expect("x25519 запустился");
        let text = String::from_utf8_lossy(&keys.stdout);
        let public = text
            .lines()
            .find(|l| l.to_lowercase().contains("public"))
            .and_then(|l| l.split_whitespace().last())
            .expect("открытый ключ напечатан")
            .to_string();

        let dir = std::env::temp_dir().join("proxysvpn-xray-check");
        std::fs::create_dir_all(&dir).expect("папка создалась");

        let cases: Vec<(&str, TunnelPrefs)> = vec![
            ("умолчания", TunnelPrefs::default()),
            ("дробление", TunnelPrefs { fragment: true, ..Default::default() }),
            ("системный DNS", TunnelPrefs { dns: DnsChoice::System, ..Default::default() }),
            ("свой DNS", TunnelPrefs { dns: DnsChoice::Custom, custom_dns: "9.9.9.9".into(), ..Default::default() }),
            ("оба семейства", TunnelPrefs { ip_kind: IpKind::Both, ..Default::default() }),
            ("всё разом", TunnelPrefs {
                fragment: true, ip_kind: IpKind::Both,
                dns: DnsChoice::Custom, custom_dns: "https://dns.quad9.net/dns-query".into(),
            }),
        ];

        let fixtures = [("tcp", vless_fixture()), ("xhttp", xhttp_fixture())];
        for ((name, prefs), (transport, fixture)) in cases
            .into_iter()
            .flat_map(|case| fixtures.iter().map(move |f| (case.clone(), f.clone())))
        {
            let name = format!("{name}-{transport}");
            let mut cfg = build_xray_config_with_routing_and_prefs(&fixture, None, &prefs);
            cfg["outbounds"][0]["streamSettings"]["realitySettings"]["publicKey"] =
                serde_json::Value::String(public.clone());
            // Порт SOCKS у образца тот же, что у живого приложения; для
            // проверки конфигурации это неважно, xray её только разбирает.
            let path = dir.join(format!("{}.json", name.replace(' ', "-")));
            std::fs::write(&path, serde_json::to_string(&cfg).expect("сериализуется"))
                .expect("записалось");

            let out = Command::new(&bin)
                .args(["run", "-test", "-c"])
                .arg(&path)
                .output()
                .expect("xray запустился");
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                !said.contains("Failed to start"),
                "xray отверг конфигурацию «{name}»:\n{said}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Дробление рукопожатия: лишний выход и звонок основного через него.
    #[test]
    fn fragmentation_adds_a_dialer_and_routes_the_node_through_it() {
        use crate::tunnel_prefs::TunnelPrefs;

        let off = build_xray_config_with_routing_and_prefs(
            &vless_fixture(), None, &TunnelPrefs::default());
        let outs = off["outbounds"].as_array().expect("исходящие есть");
        assert!(
            !outs.iter().any(|o| o["tag"] == "fragment"),
            "выключенное дробление не должно оставлять следов в конфигурации"
        );
        assert!(
            off["outbounds"][0]["streamSettings"]["sockopt"].is_null(),
            "без дробления узел звонит напрямую"
        );

        let on = build_xray_config_with_routing_and_prefs(
            &vless_fixture(), None,
            &TunnelPrefs { fragment: true, ..Default::default() });
        let outs = on["outbounds"].as_array().expect("исходящие есть");
        let frag = outs.iter().find(|o| o["tag"] == "fragment").expect("дробильщик добавлен");
        assert_eq!(frag["protocol"], "freedom");
        assert_eq!(
            frag["settings"]["fragment"]["packets"], "tlshello",
            "режем только приветствие TLS, а не весь поток"
        );
        assert_eq!(
            on["outbounds"][0]["streamSettings"]["sockopt"]["dialerProxy"], "fragment",
            "без этого дробильщик стоит в стороне и ничего не делает"
        );
        // И узел остаётся узлом: дробление не должно подменять выход.
        assert_eq!(on["outbounds"][0]["tag"], "proxy");
        assert_eq!(on["outbounds"][0]["protocol"], "vless");
    }

    /// Настройки DNS доезжают до конфигурации, а не остаются в окне.
    #[test]
    fn dns_preferences_reach_the_config() {
        use crate::tunnel_prefs::{DnsChoice, IpKind, TunnelPrefs};

        let system = build_xray_config_with_routing_and_prefs(
            &vless_fixture(), None,
            &TunnelPrefs { dns: DnsChoice::System, ..Default::default() });
        assert_eq!(system["dns"]["servers"][0], "localhost");

        let both = build_xray_config_with_routing_and_prefs(
            &vless_fixture(), None,
            &TunnelPrefs { ip_kind: IpKind::Both, ..Default::default() });
        assert_eq!(both["dns"]["queryStrategy"], "UseIP");

        let own = build_xray_config_with_routing_and_prefs(
            &vless_fixture(), None,
            &TunnelPrefs { dns: DnsChoice::Custom, custom_dns: "9.9.9.9".into(), ..Default::default() });
        assert_eq!(own["dns"]["servers"][0], "tcp://9.9.9.9");
    }

    #[test]
    fn reality_parameters_reach_the_config() {
        let cfg = build_xray_config_with_routing(&vless_fixture(), None);
        let reality = &cfg["outbounds"][0]["streamSettings"]["realitySettings"];
        assert_eq!(reality["publicKey"], "PUBKEY");
        assert_eq!(reality["shortId"], "ab12");
        assert_eq!(reality["serverName"], "www.bing.com");
        assert_eq!(cfg["outbounds"][0]["settings"]["vnext"][0]["port"], 443);
    }
}
