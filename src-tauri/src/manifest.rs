// src-tauri/src/manifest.rs
//
// The app's half of the Watafast CandidateManifest v1 contract —
// ~/vpn-project/watafast/MANIFEST-v1.md, agreed 27.09.2026. The server side is
// written to it later; this file changes only for a v2.
//
// ── Why a signed document instead of another flat list ──────────────────────
// The subscription already tells the app which servers to use, over TLS. TLS
// protects the wire, not the source: this project has already had a third
// party sitting on the Vercel account that serves it (vercel-third-party-
// access.md), and a registry that desynced from the panels underneath it
// without anyone asking for that. A signed manifest means a compromised host,
// a desynced registry or a network in the middle can hand this app a
// malicious relay only by also holding the Ed25519 private key — which never
// leaves the owner's Vercel secrets. See MANIFEST-v1.md "Why".
//
// ── Why this table starts empty ──────────────────────────────────────────────
// `MANIFEST_KEYS` is empty until the owner generates the key pair
// (ops/watafast-manifest-key/gen-key.mjs) and ships the public half here. An
// empty table is not "half-built": `manifest_enabled()` gates every other
// function in this module, so with nothing in the table the app fetches only
// the subscription, byte for byte as it did before this file existed.
//
// ── What this file does NOT do ───────────────────────────────────────────────
// No networking. The ladder walk, the HTTP client and the host list all
// already live in subscription.rs (candidate_urls / race_hosts / HOST_TIMEOUT
// / STAGGER / LADDER_BUDGET) and MANIFEST-v1.md is explicit that the manifest
// fetch reuses them rather than growing a second copy. `subscription.rs`
// calls into the pure functions here — verify, map, merge — and does the I/O.
//
// ── Trust boundary, restated for whoever edits this next ────────────────────
// Everything that reaches `verify_envelope` is attacker-controlled until it
// returns `Ok`. Order matters: size, then format version, then signature,
// THEN JSON parsing, THEN the content checks — parsing untrusted JSON before
// the signature is checked is exactly the bug this design exists to avoid.
// Rejection reasons are logged as their enum name only (`{reason:?}`); never
// log a token, a uuid, a key or an address here, per this project's rule.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::AppError;
use crate::events::SubMeta;
use crate::subscription::{self, Hy2Config, ServerConfig, Subscription, VlessConfig, VlessTransport, XhttpMode};
use crate::tunnel_prefs::TransportPref;

// ─────────────────────────────────────────────────────────────────────────
// Keys
// ─────────────────────────────────────────────────────────────────────────

/// `kid → 32-byte Ed25519 public key`.
///
/// `wf1`: generated 27.09.2026 by `ops/watafast-manifest-key/gen-key.mjs` on the
/// owner's Mac; public key `wq2wAT6NwZZpsyz8LmW7wUkTYOR/Eg6hpzKX9h/M2/w=`. The
/// private half lives only in Vercel (`WATAFAST_MANIFEST_KEY`, kid in
/// `WATAFAST_MANIFEST_KID`) and a 0600 backup on the owner's Mac.
/// Rotation keeps the old entry alongside the new one for two releases —
/// MANIFEST-v1.md "Keys" — an unknown `kid` is rejected, never guessed.
pub(crate) const MANIFEST_KEYS: &[(&str, [u8; 32])] = &[(
    "wf1",
    [
        0xc2, 0xad, 0xb0, 0x01, 0x3e, 0x8d, 0xc1, 0x96, 0x69, 0xb3, 0x2c, 0xfc, 0x2e, 0x65, 0xbb, 0xc1,
        0x49, 0x13, 0x60, 0xe4, 0x7f, 0x12, 0x0e, 0xa1, 0xa7, 0x32, 0x97, 0xf6, 0x1f, 0xcc, 0xdb, 0xfc,
    ],
)];

/// Whether the manifest path is switched on at all: true while a key is
/// compiled in. With an empty table every other function in this module is
/// unreachable: the caller in `subscription.rs` checks this before doing any
/// I/O, so a disabled table costs nothing — not a request, not a disk read.
pub(crate) fn manifest_enabled() -> bool {
    !MANIFEST_KEYS.is_empty()
}

// ─────────────────────────────────────────────────────────────────────────
// Wire shapes — MANIFEST-v1.md "Envelope" and "Payload"
// ─────────────────────────────────────────────────────────────────────────
//
// Every field the app does not itself use (the location's `id`, `priority`,
// the account's `plan`, `notice`, a candidate's `ports` hop range…) is simply
// left out of these structs: serde ignores JSON fields it was not asked to
// read, which is a more honest way of saying "not v1's concern" than a
// `#[allow(dead_code)]` field nobody ever looks at.

#[derive(Debug, Clone, Deserialize)]
struct EnvelopeWire {
    v: u32,
    kid: String,
    payload: String,
    sig: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PayloadWire {
    manifest_version: u64,
    expires_at: u64,
    token_hash: String,
    account: AccountWire,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    locations: Vec<LocationWire>,
}

#[derive(Debug, Clone, Deserialize)]
struct AccountWire {
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LocationWire {
    label: String,
    #[serde(default)]
    flag: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    candidates: Vec<CandidateWire>,
}

#[derive(Debug, Clone, Deserialize)]
struct CandidateWire {
    protocol: String,
    address: String,
    port: u16,
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    sni: String,
    #[serde(default)]
    pbk: String,
    #[serde(default)]
    sid: String,
    #[serde(default)]
    fp: String,
    #[serde(default)]
    spx: String,
    #[serde(default)]
    flow: String,
    #[serde(default)]
    xhttp: Option<XhttpWire>,
    #[serde(default)]
    pin_sha256: Option<String>,
    #[serde(default)]
    insecure: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct XhttpWire {
    path: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    host: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────
// Verification — MANIFEST-v1.md rule 1
// ─────────────────────────────────────────────────────────────────────────

/// Largest envelope this app will even attempt to parse.
const MAX_ENVELOPE_BYTES: usize = 256 * 1024;

/// Domain separation: this key signs nothing else in this project.
const SIGNED_PREFIX: &[u8] = b"watafast-manifest-v1\n";

/// Clock-skew allowance past `expires_at`.
const EXPIRY_SKEW_MS: u64 = 24 * 60 * 60 * 1000;

/// Why a manifest was not used. Logged by its name only — never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectReason {
    TooLarge,
    BadEnvelopeJson,
    UnsupportedVersion,
    UnknownKid,
    BadEncoding,
    BadSignature,
    BadPayloadJson,
    TokenMismatch,
    Expired,
    Rollback,
}

/// A payload that has cleared every check of rule 1: the envelope it came in
/// verified against a known key, decoded, parsed, and matched to this token
/// with no rollback. Only what the rest of this module needs is public within
/// the crate; `locations` stays private to this file — `build_servers` is the
/// only thing that ever reduces it to a `ServerConfig` list.
pub(crate) struct Verified {
    pub(crate) manifest_version: u64,
    pub(crate) account_active: bool,
    pub(crate) hosts: Vec<String>,
    locations: Vec<LocationWire>,
}

/// The whole of MANIFEST-v1.md rule 1, in order. `keys` is a parameter (not
/// `MANIFEST_KEYS` read directly) so a test can verify against a key it
/// actually holds the private half of.
///
/// `highest_known_version`: `None` skips the anti-rollback check, which is
/// correct for re-checking an envelope this same install already accepted —
/// it cannot roll itself back — and wrong for anything arriving fresh over
/// the network, which must always pass `Some(_)`.
pub(crate) fn verify_envelope(
    raw: &[u8],
    keys: &[(&str, [u8; 32])],
    token: &str,
    highest_known_version: Option<u64>,
    now_ms: u64,
) -> Result<Verified, RejectReason> {
    if raw.len() > MAX_ENVELOPE_BYTES {
        return Err(RejectReason::TooLarge);
    }

    let envelope: EnvelopeWire =
        serde_json::from_slice(raw).map_err(|_| RejectReason::BadEnvelopeJson)?;
    if envelope.v != 1 {
        return Err(RejectReason::UnsupportedVersion);
    }
    let key_bytes = keys
        .iter()
        .find(|(kid, _)| *kid == envelope.kid)
        .map(|(_, key)| *key)
        .ok_or(RejectReason::UnknownKid)?;
    let verifying_key =
        VerifyingKey::from_bytes(&key_bytes).map_err(|_| RejectReason::UnknownKid)?;

    let payload_bytes = decode_b64url(&envelope.payload).ok_or(RejectReason::BadEncoding)?;
    let sig_bytes = decode_b64url(&envelope.sig).ok_or(RejectReason::BadEncoding)?;
    let sig_array: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| RejectReason::BadEncoding)?;
    let signature = Signature::from_bytes(&sig_array);

    // Signature FIRST, JSON parsing only after it holds — MANIFEST-v1.md is
    // explicit that there is no canonicalisation step, so this is the only
    // order that never runs a JSON parser over bytes nobody has vouched for.
    let mut message = Vec::with_capacity(SIGNED_PREFIX.len() + payload_bytes.len());
    message.extend_from_slice(SIGNED_PREFIX);
    message.extend_from_slice(&payload_bytes);
    verifying_key
        .verify_strict(&message, &signature)
        .map_err(|_| RejectReason::BadSignature)?;

    let payload: PayloadWire =
        serde_json::from_slice(&payload_bytes).map_err(|_| RejectReason::BadPayloadJson)?;

    if !payload.token_hash.eq_ignore_ascii_case(&token_hash_hex(token)) {
        return Err(RejectReason::TokenMismatch);
    }
    if now_ms > payload.expires_at.saturating_add(EXPIRY_SKEW_MS) {
        return Err(RejectReason::Expired);
    }
    if let Some(highest) = highest_known_version {
        if payload.manifest_version < highest {
            return Err(RejectReason::Rollback);
        }
    }

    Ok(Verified {
        manifest_version: payload.manifest_version,
        account_active: payload.account.status == "active",
        hosts: payload.hosts,
        locations: payload.locations,
    })
}

fn decode_b64url(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim()).ok()
}

/// First 16 hex characters of SHA-256(token) — MANIFEST-v1.md "token_hash".
/// The token itself never has to be stored anywhere for this: only its hash
/// ever reaches disk, in the persisted ledger below.
pub(crate) fn token_hash_hex(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

// ─────────────────────────────────────────────────────────────────────────
// Candidates → the app's own types — MANIFEST-v1.md rule 2
// ─────────────────────────────────────────────────────────────────────────
//
// Every check below is the one `subscription.rs`'s own line parser already
// applies to a `vless://`/`hysteria2://` line — reused through
// `subscription::clean_sni` / `valid_xhttp_path` / `valid_xhttp_host` /
// `engine_supports_on`, not re-derived here, so a future change to what the
// subscription accepts cannot quietly diverge from what the manifest accepts.

/// Reduce one location to the single `ServerConfig` v1 runs.
///
/// Order of preference: first, a candidate whose protocol matches
/// `transport` (TunnelScreen «Способ подключения») and that (a) validates and
/// (b) this build's engine can run; if `transport` is `Auto`, or no candidate
/// of the chosen protocol clears both, fall back to the first candidate, in
/// the server's own order, that clears them — today's behaviour, and what
/// keeps a location from disappearing just because it does not offer the
/// protocol asked for. `None` only when NOTHING in the location clears both —
/// MANIFEST-v1.md "drop a location left without candidates" still applies
/// exactly as before; a transport preference never drops a location by
/// itself; it only picks which of its candidates wins.
///
/// A fallback is not silent: `used_fallback` tells the caller to say so in
/// the location's own note, e.g. «XHTTP недоступен» — the location stays on
/// the list, showing what it actually connects with instead.
///
/// `lang_en` picks the language of that note the same way `set_ui_lang`
/// already threads the window's language into `Core::notify_protection`: the
/// word is baked into `remark` here, on the Rust side, and the frontend
/// renders it raw (`LocationsScreen.tsx`), so a note built in the wrong
/// language cannot be fixed downstream.
///
/// Candidates the app does not pick are not lost: the RAW envelope is what
/// gets persisted (see `record_verified`), so they are still on disk for the
/// in-location race a later version adds.
fn build_location(
    location: &LocationWire,
    sing_box: bool,
    transport: TransportPref,
    lang_en: bool,
) -> Option<ServerConfig> {
    let runnable = |candidate: &CandidateWire| -> Option<ServerConfig> {
        let server = candidate_to_server(candidate)?;
        subscription::engine_supports_on(&server, sing_box).then_some(server)
    };

    let wanted_protocol = transport.candidate_protocol();
    let preferred = wanted_protocol.and_then(|protocol| {
        location
            .candidates
            .iter()
            .filter(|candidate| candidate.protocol.eq_ignore_ascii_case(protocol))
            .find_map(runnable)
    });

    let (server, used_fallback) = match preferred {
        Some(server) => (server, false),
        None => {
            let server = location.candidates.iter().find_map(runnable)?;
            // Only a REAL fallback — `Auto` never asked for a specific
            // protocol, so taking "whatever is first" is not a fallback from
            // anything, it is the only mode `Auto` has.
            (server, wanted_protocol.is_some())
        }
    };

    let note = if used_fallback {
        let unavailable = if lang_en { "unavailable" } else { "недоступен" };
        combine_notes(
            location.note.as_deref(),
            Some(&format!("{} {unavailable}", transport.label())),
        )
    } else {
        location.note.clone()
    };
    Some(with_remark(server, &location.flag, &location.label, note.as_deref()))
}

/// Merge the location's own badge (`резерв`, `12,4 из 50 ГБ` — the SERVICE's
/// word) with the one this app adds when it had to fall back away from the
/// chosen transport. Neither replaces the other; a location can need both at
/// once.
fn combine_notes(service: Option<&str>, own: Option<&str>) -> Option<String> {
    let service = service.map(str::trim).filter(|s| !s.is_empty());
    match (service, own) {
        (Some(a), Some(b)) => Some(format!("{a}, {b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    }
}

/// All usable locations of a verified manifest, in the order the server sent
/// them (the same order rule the subscription list already follows).
pub(crate) fn build_servers(
    verified: &Verified,
    sing_box: bool,
    transport: TransportPref,
    lang_en: bool,
) -> Vec<ServerConfig> {
    verified
        .locations
        .iter()
        .filter_map(|location| build_location(location, sing_box, transport, lang_en))
        .collect()
}

/// The `remark` string `lib.rs::split_label` already knows how to read back
/// apart: an optional flag, then the label, then `" · "` and a note. Building
/// it this way means the existing list rendering needs no manifest-specific
/// branch at all.
fn build_remark(flag: &str, label: &str, note: Option<&str>) -> String {
    let mut out = String::new();
    let flag = flag.trim();
    if !flag.is_empty() {
        out.push_str(flag);
        out.push(' ');
    }
    out.push_str(label.trim());
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        out.push_str(" · ");
        out.push_str(note);
    }
    out
}

fn with_remark(server: ServerConfig, flag: &str, label: &str, note: Option<&str>) -> ServerConfig {
    let remark = build_remark(flag, label, note);
    match server {
        ServerConfig::Vless(mut cfg) => {
            cfg.remark = remark;
            ServerConfig::Vless(cfg)
        }
        ServerConfig::Hy2(mut cfg) => {
            cfg.remark = remark;
            ServerConfig::Hy2(cfg)
        }
    }
}

fn candidate_to_server(candidate: &CandidateWire) -> Option<ServerConfig> {
    match candidate.protocol.as_str() {
        "hysteria2" => candidate_to_hy2(candidate).map(ServerConfig::Hy2),
        "vision" => candidate_to_vless(candidate, true).map(ServerConfig::Vless),
        "xhttp" => candidate_to_vless(candidate, false).map(ServerConfig::Vless),
        // Forward compatibility, MANIFEST-v1.md rule 2: an unknown protocol
        // skips the ONE candidate, never the location and never the fetch.
        _ => None,
    }
}

fn candidate_to_vless(candidate: &CandidateWire, is_vision: bool) -> Option<VlessConfig> {
    if candidate.uuid.trim().is_empty() || candidate.address.trim().is_empty() || candidate.port == 0 {
        return None;
    }
    // REALITY without a public key cannot connect — the exact rule
    // `parse_vless_url` applies to a subscription line.
    if candidate.pbk.trim().is_empty() {
        return None;
    }

    let transport = if is_vision {
        // "flow only on vision": a candidate marked vision that carries no
        // flow is not actually vision, whatever its `protocol` field claims.
        if candidate.flow.trim().is_empty() {
            return None;
        }
        VlessTransport::Tcp
    } else {
        let xhttp = candidate.xhttp.as_ref()?;
        let path = xhttp.path.trim();
        if !subscription::valid_xhttp_path(path) {
            return None;
        }
        let mode = match xhttp.mode.as_deref().map(str::trim) {
            None | Some("") => XhttpMode::Auto,
            Some(raw) => XhttpMode::parse(raw)?,
        };
        let host = match xhttp.host.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(h) if subscription::valid_xhttp_host(h) => Some(h.to_string()),
            Some(_) => return None,
        };
        VlessTransport::Xhttp { path: path.to_string(), mode, host }
    };

    let sni_source = if candidate.sni.trim().is_empty() { candidate.address.as_str() } else { candidate.sni.as_str() };
    let fingerprint = if candidate.fp.trim().is_empty() { "chrome".to_string() } else { candidate.fp.clone() };
    // Vision is TCP-only; an xhttp candidate never carries a flow — the
    // subscription parser clears it for the same reason, so this does too
    // rather than trust the server never to send one.
    let flow = if is_vision { candidate.flow.clone() } else { String::new() };

    Some(VlessConfig {
        uuid: candidate.uuid.clone(),
        host: candidate.address.clone(),
        port: candidate.port,
        encryption: "none".to_string(),
        public_key: candidate.pbk.clone(),
        short_id: candidate.sid.clone(),
        sni: subscription::clean_sni(sni_source),
        fingerprint,
        flow,
        spider_x: candidate.spx.clone(),
        remark: String::new(),
        transport,
    })
}

fn candidate_to_hy2(candidate: &CandidateWire) -> Option<Hy2Config> {
    let password = candidate.uuid.trim();
    if password.is_empty() || candidate.address.trim().is_empty() || candidate.port == 0 {
        return None;
    }
    let sni_source = if candidate.sni.trim().is_empty() { candidate.address.as_str() } else { candidate.sni.as_str() };
    let pin_sha256 = candidate.pin_sha256.clone().unwrap_or_default();
    // With a certificate pin the fingerprint IS the verification — the same
    // rule `parse_hy2_url` applies.
    let insecure = candidate.insecure || !pin_sha256.trim().is_empty();

    Some(Hy2Config {
        password: password.to_string(),
        host: candidate.address.clone(),
        port: candidate.port,
        sni: subscription::clean_sni(sni_source),
        pin_sha256,
        insecure,
        remark: String::new(),
    })
}

// ─────────────────────────────────────────────────────────────────────────
// The `hosts` list — MANIFEST-v1.md rule 4
// ─────────────────────────────────────────────────────────────────────────

/// A name from the manifest's signed `hosts` list is fit to become part of a
/// URL this app builds itself: a bare DNS name, no scheme, no port, no path,
/// not an IP literal (the field is documented as host NAMES). Reuses
/// `subscription::valid_xhttp_host` for the DNS-label shape, which alone
/// would accept an IPv4 literal (every label of `1.2.3.4` is alphanumeric),
/// so the IP check here runs first.
fn valid_manifest_host(raw: &str) -> bool {
    let host = raw.trim();
    if host.is_empty()
        || host.contains("://")
        || host.contains(':')
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
    {
        return false;
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    subscription::valid_xhttp_host(host)
}

/// Validated, self-deduplicated (case-insensitive), capped at 8, order kept —
/// MANIFEST-v1.md rule 4. Cross-deduplication against the built-in ladder
/// hosts happens where the two lists are joined, in `subscription.rs`.
pub(crate) fn sanitize_extra_hosts(raw_hosts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for host in raw_hosts {
        let host = host.trim().to_ascii_lowercase();
        if !valid_manifest_host(&host) {
            continue;
        }
        if out.iter().any(|h| h == &host) {
            continue;
        }
        out.push(host);
        if out.len() == 8 {
            break;
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// Persistence — MANIFEST-v1.md rule 3
// ─────────────────────────────────────────────────────────────────────────
//
// Mirrors `netmem.rs`'s `dir_candidates`/`write_private` (same directories,
// same 0600, same best-effort story) rather than importing it: that file is
// one of the three this change is asked to leave untouched, and its helpers
// are private to it. The ~20 duplicated lines are the price of that.

fn dir_candidates() -> Vec<PathBuf> {
    crate::appdirs::state_dirs()
}

fn write_private(path: &Path, bytes: &[u8]) -> bool {
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

const STORE_FILE: &str = "watafast-manifest.json";

/// Everything about the manifest this install keeps between runs. Private to
/// this module — `subscription.rs` reaches it only through the three
/// functions below, never through this shape directly.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Store {
    /// Highest `manifest_version` ever accepted, per `token_hash`. The token
    /// itself is never a key here — only its hash, which is also all that
    /// ever reaches the wire in the payload.
    #[serde(default)]
    highest_version: HashMap<String, u64>,
    /// The last envelope that verified, exactly as the server sent it, and
    /// which ladder address served it. Re-verified against the CURRENT key
    /// table, token and clock on every read (`cached_envelope` promises
    /// nothing by itself) — a key rotation or a token change must be able to
    /// retire it without a separate migration step.
    #[serde(default)]
    last_good: Option<CachedEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CachedEnvelope {
    envelope: String,
    source_host: String,
}

impl Store {
    fn highest_for(&self, token_hash: &str) -> Option<u64> {
        self.highest_version.get(token_hash).copied()
    }

    fn bump(&mut self, token_hash: &str, version: u64) {
        let entry = self.highest_version.entry(token_hash.to_string()).or_insert(0);
        if version > *entry {
            *entry = version;
        }
    }
}

fn load_store_from(dirs: &[PathBuf]) -> Store {
    for dir in dirs {
        if let Ok(text) = std::fs::read_to_string(dir.join(STORE_FILE)) {
            if let Ok(store) = serde_json::from_str::<Store>(&text) {
                return store;
            }
        }
    }
    Store::default()
}

fn save_store_to(dirs: &[PathBuf], store: &Store) -> bool {
    let Ok(text) = serde_json::to_string(store) else {
        return false;
    };
    dirs.iter().any(|dir| write_private(&dir.join(STORE_FILE), text.as_bytes()))
}

/// Serializes every access to the persisted store across this process.
///
/// `fetch_subscription` (subscription.rs) is not always called under
/// `Core::operation` in lib.rs: `connect()`, the routing-switch flow and the
/// apply-routing-preference flow all take that lock, but the "Проверить
/// подключение" diagnostics card (`check_subscription`) calls it directly, so
/// two manifest fetches CAN run at once — one of them possibly racing a
/// desynced or third-party-controlled ladder host, exactly the threat
/// MANIFEST-v1.md's signing exists to defend against (vercel-third-party-
/// access.md already happened once on this project). Without a lock spanning
/// `load_store_from`+`save_store_to`, two such fetches interleave their
/// read-modify-write of `watafast-manifest.json` and a slower writer's save
/// can clobber a faster writer's already-persisted, higher version — the
/// exact "monotonic, anti-rollback" ledger DESIGN.md §6.1 requires would then
/// move backwards. Only this process ever touches this file (no other tool in
/// the repo references `STORE_FILE`), so an in-process mutex is enough; no OS
/// advisory lock is needed unless that stops being true.
fn store_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// The highest `manifest_version` this install has ever accepted for this
/// token — the anti-rollback floor a fresh fetch must clear.
pub(crate) fn highest_known_version(token_hash: &str) -> Option<u64> {
    // Same lock as `record_verified`: reading while a concurrent record is
    // mid-(load, mutate, save) must not observe a half-updated on-disk state.
    let _guard = store_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    load_store_from(&dir_candidates()).highest_for(token_hash)
}

/// The last envelope that verified, and which host served it — for a fetch
/// that fails to fall back to, and for its `hosts` to widen the ladder with.
/// `None` costs nothing extra: it is exactly today's behaviour.
pub(crate) fn cached_envelope() -> Option<(String, String)> {
    let _guard = store_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    load_store_from(&dir_candidates())
        .last_good
        .map(|c| (c.envelope, c.source_host))
}

/// Record a freshly verified envelope. Best effort, like `netmem.rs`: losing
/// this costs one missed cache on the next failure, never a wrong one — the
/// anti-rollback check re-derives from whatever IS on disk, and a write that
/// silently failed simply means the ledger did not move forward this time.
///
/// The whole load-mutate-save sequence runs under `store_lock()`, and reloads
/// the store from disk INSIDE the lock rather than trusting a value the
/// caller may have read before some other, concurrent call finished writing.
/// That reload is what makes `bump()` (already forward-only) actually
/// forward-only across concurrent callers, not just within one: whichever
/// call takes the lock second sees the first call's write and can only move
/// the ledger further forward, never back. The same guard applies to
/// `last_good` — a validly signed but older envelope (for example a replayed
/// one from a compromised or desynced host, verified against a rollback floor
/// this call read before a concurrent, newer write landed) must not overwrite
/// a `last_good` that a concurrent call already advanced past; comparing
/// against the highest version on disk before this call's own bump answers
/// that regardless of which caller's in-memory read happened first.
pub(crate) fn record_verified(token_hash: &str, version: u64, envelope: String, source_host: String) {
    record_verified_in(&dir_candidates(), token_hash, version, envelope, source_host);
}

/// The actual body of `record_verified`, taking `dirs` explicitly — same
/// split as `load_store_from`/`save_store_to` — so a test can point it at a
/// temp directory instead of this install's real, OS-specific paths.
fn record_verified_in(dirs: &[PathBuf], token_hash: &str, version: u64, envelope: String, source_host: String) {
    let _guard = store_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut store = load_store_from(dirs);
    let previously_known = store.highest_for(token_hash).unwrap_or(0);
    store.bump(token_hash, version);
    if version >= previously_known {
        store.last_good = Some(CachedEnvelope { envelope, source_host });
    }
    save_store_to(dirs, &store);
}

// ─────────────────────────────────────────────────────────────────────────
// The merge decision — how a fetched manifest changes what the app connects
// to. Pure: `subscription.rs` does the fetching, this decides.
// ─────────────────────────────────────────────────────────────────────────

/// MANIFEST-v1.md's four outcomes, as one function:
///
///   • subscription OK, manifest usable  → manifest's servers, subscription's
///     routing/meta/source (only `.servers` changes);
///   • subscription OK, no usable manifest → subscription untouched;
///   • subscription failed, manifest usable → a `Subscription` built from the
///     manifest alone (default meta, no routing profile — the seed applies,
///     same as any subscription response that carried none);
///   • both failed → the subscription's own error, verbatim.
///
/// `manifest` already reflects every precondition (`account.status ==
/// "active"`, at least one usable location) by the time it reaches here —
/// see `subscription::fetch_manifest_servers` — so this function's only job
/// is the four-way branch above, not re-checking them.
pub(crate) fn merge(
    subscription: Result<Subscription, AppError>,
    manifest: Option<(Vec<ServerConfig>, String)>,
) -> Result<Subscription, AppError> {
    match (subscription, manifest) {
        (Ok(mut sub), Some((servers, _))) if !servers.is_empty() => {
            sub.servers = servers;
            Ok(sub)
        }
        (Ok(sub), _) => Ok(sub),
        (Err(_), Some((servers, host))) if !servers.is_empty() => Ok(Subscription {
            servers,
            meta: SubMeta::default(),
            routing: None,
            unreadable_lines: 0,
            source_host: host,
        }),
        (Err(err), _) => Err(err),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// A fixed 32-byte seed — not a real key, never used outside this file.
    const TEST_SEED: [u8; 32] = [7u8; 32];
    const TEST_KID: &str = "test";
    const TEST_TOKEN: &str = "tok_abc123";

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&TEST_SEED)
    }

    fn test_keys() -> Vec<(&'static str, [u8; 32])> {
        vec![(TEST_KID, test_signing_key().verifying_key().to_bytes())]
    }

    fn sample_payload(overrides: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut value = serde_json::json!({
            "manifest_version": 10,
            "issued_at": 1_790_000_000_000u64,
            "expires_at": 1_800_000_000_000u64,
            "token_hash": token_hash_hex(TEST_TOKEN),
            "account": { "status": "active", "plan": "base", "notice": null },
            "hosts": ["proksya.xyz"],
            "locations": [
                {
                    "id": "lon-xhttp",
                    "label": "Британия",
                    "flag": "🇬🇧",
                    "priority": 1,
                    "note": null,
                    "candidates": [
                        {
                            "id": "lon-xhttp",
                            "protocol": "xhttp",
                            "address": "203.0.113.10",
                            "port": 2053,
                            "uuid": "11111111-1111-1111-1111-111111111111",
                            "sni": "www.nhs.uk",
                            "pbk": "pbk-value",
                            "sid": "f983b97800fa83a9",
                            "fp": "firefox",
                            "spx": "/",
                            "xhttp": { "path": "/87588135c873", "mode": "stream-one" }
                        },
                        {
                            "id": "lon-xhttp~vision",
                            "protocol": "vision",
                            "address": "203.0.113.10",
                            "port": 8443,
                            "uuid": "11111111-1111-1111-1111-111111111111",
                            "sni": "www.nhs.uk",
                            "pbk": "pbk-value",
                            "sid": "f983b97800fa83a9",
                            "fp": "firefox",
                            "spx": "/",
                            "flow": "xtls-rprx-vision"
                        }
                    ]
                }
            ]
        });
        overrides(&mut value);
        serde_json::to_vec(&value).expect("payload serialises")
    }

    fn sign_envelope(payload_bytes: &[u8], kid: &str, signing_key: &SigningKey) -> Vec<u8> {
        let mut message = Vec::with_capacity(SIGNED_PREFIX.len() + payload_bytes.len());
        message.extend_from_slice(SIGNED_PREFIX);
        message.extend_from_slice(payload_bytes);
        let signature: Signature = signing_key.sign(&message);
        let envelope = serde_json::json!({
            "v": 1,
            "kid": kid,
            "payload": URL_SAFE_NO_PAD.encode(payload_bytes),
            "sig": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        });
        serde_json::to_vec(&envelope).expect("envelope serialises")
    }

    fn good_envelope() -> Vec<u8> {
        let payload = sample_payload(|_| {});
        sign_envelope(&payload, TEST_KID, &test_signing_key())
    }

    const NOW: u64 = 1_790_500_000_000;

    #[test]
    fn a_good_envelope_verifies() {
        let verified = verify_envelope(&good_envelope(), &test_keys(), TEST_TOKEN, Some(10), NOW)
            .expect("should verify");
        assert_eq!(verified.manifest_version, 10);
        assert!(verified.account_active);
        assert_eq!(verified.hosts, vec!["proksya.xyz".to_string()]);
    }

    #[test]
    fn a_tampered_payload_fails_signature() {
        let payload = sample_payload(|_| {});
        let mut envelope = sign_envelope(&payload, TEST_KID, &test_signing_key());
        // Flip a byte inside the base64url `payload` field's value, well past
        // the JSON scaffolding, so the document still parses as an envelope.
        let marker = b"\"payload\":\"";
        let start = find(&envelope, marker).expect("payload field present") + marker.len();
        envelope[start] = if envelope[start] == b'A' { b'B' } else { b'A' };
        let result = verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(10), NOW);
        assert_eq!(result.err(), Some(RejectReason::BadSignature));
    }

    #[test]
    fn a_tampered_signature_fails() {
        let payload = sample_payload(|_| {});
        let mut envelope = sign_envelope(&payload, TEST_KID, &test_signing_key());
        let marker = b"\"sig\":\"";
        let start = find(&envelope, marker).expect("sig field present") + marker.len();
        envelope[start] = if envelope[start] == b'A' { b'B' } else { b'A' };
        let result = verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(10), NOW);
        assert_eq!(result.err(), Some(RejectReason::BadSignature));
    }

    #[test]
    fn an_unknown_kid_is_rejected() {
        let payload = sample_payload(|_| {});
        let envelope = sign_envelope(&payload, "wf9-never-shipped", &test_signing_key());
        let result = verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(10), NOW);
        assert_eq!(result.err(), Some(RejectReason::UnknownKid));
    }

    #[test]
    fn an_unsupported_version_is_rejected() {
        let payload = sample_payload(|_| {});
        let signing_key = test_signing_key();
        let mut message = Vec::new();
        message.extend_from_slice(SIGNED_PREFIX);
        message.extend_from_slice(&payload);
        let signature: Signature = signing_key.sign(&message);
        let envelope = serde_json::json!({
            "v": 2,
            "kid": TEST_KID,
            "payload": URL_SAFE_NO_PAD.encode(&payload),
            "sig": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        });
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let result = verify_envelope(&bytes, &test_keys(), TEST_TOKEN, Some(10), NOW);
        assert_eq!(result.err(), Some(RejectReason::UnsupportedVersion));
    }

    #[test]
    fn an_oversize_envelope_is_rejected_without_parsing() {
        let huge = vec![b'a'; MAX_ENVELOPE_BYTES + 1];
        let result = verify_envelope(&huge, &test_keys(), TEST_TOKEN, Some(10), NOW);
        assert_eq!(result.err(), Some(RejectReason::TooLarge));
    }

    #[test]
    fn expiry_allows_the_skew_window_and_rejects_past_it() {
        let payload = sample_payload(|v| {
            v["expires_at"] = serde_json::json!(1_000_000u64);
        });
        let envelope = sign_envelope(&payload, TEST_KID, &test_signing_key());

        let just_inside = 1_000_000 + EXPIRY_SKEW_MS;
        assert!(verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(10), just_inside).is_ok());

        let just_outside = 1_000_000 + EXPIRY_SKEW_MS + 1;
        let result = verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(10), just_outside);
        assert_eq!(result.err(), Some(RejectReason::Expired));
    }

    #[test]
    fn a_rollback_is_rejected_and_an_equal_version_is_accepted() {
        let envelope = good_envelope(); // manifest_version: 10
        let rollback = verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(11), NOW);
        assert_eq!(rollback.err(), Some(RejectReason::Rollback));

        let equal = verify_envelope(&envelope, &test_keys(), TEST_TOKEN, Some(10), NOW);
        assert!(equal.is_ok(), "equal version is not a rollback");
    }

    #[test]
    fn a_token_hash_mismatch_is_rejected() {
        let envelope = good_envelope();
        let result = verify_envelope(&envelope, &test_keys(), "a-different-token", Some(10), NOW);
        assert_eq!(result.err(), Some(RejectReason::TokenMismatch));
    }

    /// Byte offset of the first occurrence of `needle` in `haystack`.
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    // ── Candidate validation ────────────────────────────────────────────

    fn valid_xhttp_candidate() -> CandidateWire {
        CandidateWire {
            protocol: "xhttp".to_string(),
            address: "203.0.113.10".to_string(),
            port: 2053,
            uuid: "11111111-1111-1111-1111-111111111111".to_string(),
            sni: "www.nhs.uk".to_string(),
            pbk: "pbk-value".to_string(),
            sid: "f983b97800fa83a9".to_string(),
            fp: "firefox".to_string(),
            spx: "/".to_string(),
            flow: String::new(),
            xhttp: Some(XhttpWire {
                path: "/87588135c873".to_string(),
                mode: Some("stream-one".to_string()),
                host: None,
            }),
            pin_sha256: None,
            insecure: false,
        }
    }

    fn valid_vision_candidate() -> CandidateWire {
        CandidateWire {
            protocol: "vision".to_string(),
            flow: "xtls-rprx-vision".to_string(),
            xhttp: None,
            port: 8443,
            ..valid_xhttp_candidate()
        }
    }

    fn valid_hysteria2_candidate() -> CandidateWire {
        CandidateWire {
            protocol: "hysteria2".to_string(),
            address: "203.0.113.20".to_string(),
            port: 443,
            uuid: "hy2-auth-token".to_string(),
            sni: "hy2.example".to_string(),
            pbk: String::new(),
            sid: String::new(),
            fp: String::new(),
            spx: String::new(),
            flow: String::new(),
            xhttp: None,
            pin_sha256: Some("a".repeat(64)),
            insecure: false,
        }
    }

    #[test]
    fn a_valid_xhttp_candidate_maps_to_a_vless_config() {
        let cfg = candidate_to_vless(&valid_xhttp_candidate(), false).expect("valid");
        assert_eq!(cfg.port, 2053);
        assert!(matches!(cfg.transport, VlessTransport::Xhttp { .. }));
        assert!(cfg.flow.is_empty(), "flow only on vision");
    }

    #[test]
    fn a_valid_vision_candidate_maps_to_a_vless_config() {
        let cfg = candidate_to_vless(&valid_vision_candidate(), true).expect("valid");
        assert_eq!(cfg.transport, VlessTransport::Tcp);
        assert_eq!(cfg.flow, "xtls-rprx-vision");
    }

    #[test]
    fn a_vision_candidate_without_flow_is_dropped() {
        let mut candidate = valid_vision_candidate();
        candidate.flow.clear();
        assert!(candidate_to_vless(&candidate, true).is_none());
    }

    #[test]
    fn a_missing_public_key_drops_the_candidate() {
        let mut candidate = valid_xhttp_candidate();
        candidate.pbk.clear();
        assert!(candidate_to_vless(&candidate, false).is_none());
    }

    #[test]
    fn a_bad_xhttp_path_drops_the_candidate() {
        let mut candidate = valid_xhttp_candidate();
        candidate.xhttp.as_mut().unwrap().path = "no-leading-slash".to_string();
        assert!(candidate_to_vless(&candidate, false).is_none());
    }

    #[test]
    fn an_unknown_xhttp_mode_drops_the_candidate() {
        let mut candidate = valid_xhttp_candidate();
        candidate.xhttp.as_mut().unwrap().mode = Some("teleport".to_string());
        assert!(candidate_to_vless(&candidate, false).is_none());
    }

    #[test]
    fn an_empty_address_or_port_drops_the_candidate() {
        let mut no_address = valid_xhttp_candidate();
        no_address.address.clear();
        assert!(candidate_to_vless(&no_address, false).is_none());

        let mut no_port = valid_xhttp_candidate();
        no_port.port = 0;
        assert!(candidate_to_vless(&no_port, false).is_none());
    }

    #[test]
    fn a_valid_hysteria2_candidate_maps_and_a_pin_forces_insecure() {
        let cfg = candidate_to_hy2(&valid_hysteria2_candidate()).expect("valid");
        assert_eq!(cfg.password, "hy2-auth-token");
        assert!(cfg.insecure, "a pin makes insecure safe, same as parse_hy2_url");
    }

    #[test]
    fn an_empty_hysteria2_password_drops_the_candidate() {
        let mut candidate = valid_hysteria2_candidate();
        candidate.uuid.clear();
        assert!(candidate_to_hy2(&candidate).is_none());
    }

    #[test]
    fn an_unknown_protocol_is_skipped_not_an_error() {
        let mut candidate = valid_xhttp_candidate();
        candidate.protocol = "quic-mystery".to_string();
        assert!(candidate_to_server(&candidate).is_none());
    }

    #[test]
    fn a_location_prefers_xhttp_but_falls_back_when_the_engine_cannot_run_it() {
        let location = LocationWire {
            label: "Британия".to_string(),
            flag: "🇬🇧".to_string(),
            note: None,
            candidates: vec![valid_xhttp_candidate(), valid_vision_candidate()],
        };
        let desktop = build_location(&location, false, TransportPref::Auto, false).expect("xhttp usable on desktop");
        assert!(matches!(desktop, ServerConfig::Vless(c) if matches!(c.transport, VlessTransport::Xhttp { .. })));

        let ios = build_location(&location, true, TransportPref::Auto, false).expect("vision usable on sing-box");
        assert!(matches!(ios, ServerConfig::Vless(c) if c.transport == VlessTransport::Tcp));
    }

    #[test]
    fn a_location_with_both_transports_honours_the_chosen_one() {
        let location = LocationWire {
            label: "Британия".to_string(),
            flag: "🇬🇧".to_string(),
            note: None,
            candidates: vec![valid_xhttp_candidate(), valid_vision_candidate()],
        };
        let xhttp_pref = build_location(&location, false, TransportPref::XhttpOnly, false).expect("xhttp present");
        assert!(matches!(xhttp_pref, ServerConfig::Vless(ref c) if matches!(c.transport, VlessTransport::Xhttp { .. })));
        assert_eq!(xhttp_pref.remark(), "🇬🇧 Британия", "no fallback happened, no note added");

        let vision_pref = build_location(&location, false, TransportPref::VisionOnly, false).expect("vision present");
        assert!(matches!(vision_pref, ServerConfig::Vless(ref c) if c.transport == VlessTransport::Tcp));
        assert_eq!(vision_pref.remark(), "🇬🇧 Британия");
    }

    #[test]
    fn a_location_without_the_chosen_transport_falls_back_and_says_so() {
        // Only XHTTP at this location — «Vision only» cannot be honoured.
        let location = LocationWire {
            label: "Британия".to_string(),
            flag: "🇬🇧".to_string(),
            note: None,
            candidates: vec![valid_xhttp_candidate()],
        };
        let server = build_location(&location, false, TransportPref::VisionOnly, false)
            .expect("the location is not dropped just because it lacks the preferred transport");
        assert!(matches!(server, ServerConfig::Vless(ref c) if matches!(c.transport, VlessTransport::Xhttp { .. })));
        assert_eq!(
            server.remark(),
            "🇬🇧 Британия · Vision недоступен",
            "the fallback must be visible in the location list, not silent"
        );
    }

    /// Review finding (28.09.2026): the fallback note was always the literal
    /// Russian word "недоступен", even with the UI language set to English —
    /// an otherwise-English screen reading "Britain · Vision недоступен".
    /// `lang_en` must switch the word, not just the location's own label
    /// (which stays exactly as the service sent it either way).
    #[test]
    fn the_fallback_note_follows_the_ui_language_not_just_the_locations_own_label() {
        let location = LocationWire {
            label: "Британия".to_string(),
            flag: "🇬🇧".to_string(),
            note: None,
            candidates: vec![valid_xhttp_candidate()],
        };
        let server = build_location(&location, false, TransportPref::VisionOnly, true)
            .expect("xhttp is still usable even though vision was asked for");
        assert_eq!(
            server.remark(),
            "🇬🇧 Британия · Vision unavailable",
            "an English window must not get a bare Russian word stitched into the row"
        );
    }

    #[test]
    fn a_fallback_note_is_added_next_to_the_services_own_badge_not_instead_of_it() {
        let location = LocationWire {
            label: "США".to_string(),
            flag: "🇺🇸".to_string(),
            note: Some("12,4 из 50 ГБ".to_string()),
            candidates: vec![valid_vision_candidate()],
        };
        let server = build_location(&location, false, TransportPref::XhttpOnly, false).expect("vision usable");
        assert_eq!(server.remark(), "🇺🇸 США · 12,4 из 50 ГБ, XHTTP недоступен");
    }

    #[test]
    fn a_hysteria2_only_location_is_unaffected_by_auto_but_still_reports_a_vless_preference() {
        let location = LocationWire {
            label: "Швеция".to_string(),
            flag: "🇸🇪".to_string(),
            note: None,
            candidates: vec![valid_hysteria2_candidate()],
        };
        // `Auto` never triggers a fallback note — this is its only mode.
        let auto = build_location(&location, false, TransportPref::Auto, false).expect("hy2 usable");
        assert_eq!(auto.remark(), "🇸🇪 Швеция");
        // A VLESS-only preference honestly says this location cannot give it
        // XHTTP or Vision either — it only has Hysteria2 to offer.
        let xhttp_pref = build_location(&location, false, TransportPref::XhttpOnly, false).expect("hy2 usable");
        assert_eq!(xhttp_pref.remark(), "🇸🇪 Швеция · XHTTP недоступен");
    }

    #[test]
    fn a_location_with_no_usable_candidate_is_dropped() {
        let mut broken = valid_xhttp_candidate();
        broken.pbk.clear();
        let location = LocationWire {
            label: "Мертво".to_string(),
            flag: String::new(),
            note: None,
            candidates: vec![broken],
        };
        assert!(build_location(&location, false, TransportPref::Auto, false).is_none());
    }

    #[test]
    fn build_remark_matches_the_flag_label_note_shape_split_label_expects() {
        assert_eq!(build_remark("🇬🇧", "Британия", None), "🇬🇧 Британия");
        assert_eq!(
            build_remark("🇺🇸", "США", Some("12,4 из 50 ГБ")),
            "🇺🇸 США · 12,4 из 50 ГБ"
        );
        assert_eq!(build_remark("", "Нидерланды", None), "Нидерланды");
    }

    /// A manifest location noted "XHTTP" (found live 27.09.2026 on
    /// "Британия") must not come out the other end as "Британия · Watafast
    /// · Watafast": `build_location` writes the note into `remark` exactly
    /// as `subscription.rs` would, and `lib.rs::locations`/`start_on` read it
    /// back with the same `split_label` + `display_note` — this is that
    /// whole path, not just the string builder.
    #[test]
    fn a_manifest_location_noted_xhttp_does_not_double_its_own_protocol_name() {
        let location = LocationWire {
            label: "Британия".to_string(),
            flag: "🇬🇧".to_string(),
            note: Some("XHTTP".to_string()),
            candidates: vec![valid_xhttp_candidate()],
        };
        let server = build_location(&location, false, TransportPref::Auto, false).expect("xhttp usable on desktop");
        assert_eq!(server.remark(), "🇬🇧 Британия · XHTTP");
        assert_eq!(server.protocol_label(), crate::subscription::WATAFAST_NAME);

        let (_, label, note) = crate::split_label(server.remark());
        assert_eq!(label, "Британия");
        assert_eq!(
            crate::subscription::display_note(note),
            None,
            "the note is only the transport word protocol_label already shows"
        );
    }

    // ── Hosts validation ─────────────────────────────────────────────────

    #[test]
    fn host_validation_rejects_scheme_port_path_and_ip_literals() {
        assert!(valid_manifest_host("proksya.xyz"));
        assert!(!valid_manifest_host("https://proksya.xyz"));
        assert!(!valid_manifest_host("proksya.xyz:443"));
        assert!(!valid_manifest_host("proksya.xyz/api"));
        assert!(!valid_manifest_host("203.0.113.10"));
        assert!(!valid_manifest_host("::1"));
        assert!(!valid_manifest_host(""));
    }

    #[test]
    fn extra_hosts_are_deduplicated_and_capped_at_eight() {
        let mut raw: Vec<String> = (0..10).map(|i| format!("mirror{i}.example")).collect();
        raw.push("mirror0.example".to_string()); // duplicate
        raw.push("http://bad.example".to_string()); // invalid, dropped
        let out = sanitize_extra_hosts(&raw);
        assert_eq!(out.len(), 8);
        assert_eq!(out[0], "mirror0.example");
    }

    // ── Persistence ──────────────────────────────────────────────────────

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "proxysvpn-manifest-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_store_survives_a_round_trip_through_a_temp_dir() {
        let dir = temp_dir("roundtrip");
        let dirs = vec![dir.clone()];

        let mut store = load_store_from(&dirs);
        assert_eq!(store, Store::default(), "nothing written yet");

        store.bump("hash-a", 5);
        store.last_good = Some(CachedEnvelope {
            envelope: "{\"v\":1}".to_string(),
            source_host: "proksya.xyz".to_string(),
        });
        assert!(save_store_to(&dirs, &store));

        let back = load_store_from(&dirs);
        assert_eq!(back, store);
        assert_eq!(back.highest_for("hash-a"), Some(5));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dir.join(STORE_FILE)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bump_never_moves_the_ledger_backwards() {
        let mut store = Store::default();
        store.bump("h", 10);
        store.bump("h", 3);
        assert_eq!(store.highest_for("h"), Some(10));
        store.bump("h", 10);
        assert_eq!(store.highest_for("h"), Some(10));
    }

    // ── record_verified: safe under a lost-update race ──────────────────────
    //
    // The scenario these two tests pin down is a review finding: two
    // concurrent `fetch_subscription` calls (one under `Core::operation`, one
    // from the "Проверить подключение" diagnostics card, which never takes
    // that lock — see `store_lock`'s doc comment) can each call
    // `record_verified` with a DIFFERENT version, in either order. Whichever
    // one lands second on disk must never regress `highest_version` or
    // `last_good` below what the other one already persisted.

    #[test]
    fn an_older_arrival_landing_second_does_not_undo_a_newer_one() {
        let dir = temp_dir("record-order-newer-first");
        let dirs = vec![dir.clone()];

        // The legitimate refresh reaches version 21 first...
        record_verified_in(&dirs, "hash-a", 21, "{\"v\":21}".to_string(), "proxysvpn.com".to_string());
        // ...then a slower, concurrent fetch that read an older rollback
        // floor (e.g. it raced a desynced or malicious ladder host serving a
        // replayed, but validly-signed, version 20) finishes second.
        record_verified_in(&dirs, "hash-a", 20, "{\"v\":20}".to_string(), "evil-mirror.example".to_string());

        let store = load_store_from(&dirs);
        assert_eq!(store.highest_for("hash-a"), Some(21), "the ledger must not regress");
        assert_eq!(
            store.last_good.map(|c| c.envelope),
            Some("{\"v\":21}".to_string()),
            "an older, later-arriving write must not evict the newer cached envelope"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_equal_version_arriving_second_is_harmless() {
        let dir = temp_dir("record-order-equal-second");
        let dirs = vec![dir.clone()];

        record_verified_in(&dirs, "hash-a", 20, "{\"v\":20,\"src\":\"first\"}".to_string(), "proxysvpn.com".to_string());
        // MANIFEST-v1.md treats an equal version as "not a rollback" — this
        // must still leave the ledger and the cache exactly where they were,
        // not flip them to whichever copy of the same version wrote last.
        record_verified_in(&dirs, "hash-a", 20, "{\"v\":20,\"src\":\"second\"}".to_string(), "proksya.xyz".to_string());

        let store = load_store_from(&dirs);
        assert_eq!(store.highest_for("hash-a"), Some(20));
        assert_eq!(
            store.last_good.map(|c| c.envelope),
            Some("{\"v\":20,\"src\":\"second\"}".to_string()),
            "an equal version is not a rollback, so it is allowed to refresh the cache"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_record_verified_calls_never_lose_the_highest_version() {
        // Real concurrency, not just call order: several threads race to
        // record versions 1..=N for the same token against the same temp
        // store. `store_lock()` must serialize their load-mutate-save cycles
        // so the highest version always wins regardless of scheduling.
        let dir = temp_dir("record-concurrent");
        let dirs = std::sync::Arc::new(vec![dir.clone()]);
        const N: u64 = 20;

        let handles: Vec<_> = (1..=N)
            .map(|version| {
                let dirs = std::sync::Arc::clone(&dirs);
                std::thread::spawn(move || {
                    record_verified_in(
                        &dirs,
                        "hash-a",
                        version,
                        format!("{{\"v\":{version}}}"),
                        "proxysvpn.com".to_string(),
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        let store = load_store_from(&dirs);
        assert_eq!(store.highest_for("hash-a"), Some(N), "the highest version must survive the race");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── The merge decision ───────────────────────────────────────────────

    fn sample_subscription(host: &str) -> Subscription {
        Subscription {
            servers: vec![ServerConfig::Hy2(Hy2Config {
                password: "sub-password".to_string(),
                host: "sub.example".to_string(),
                port: 443,
                sni: "sub.example".to_string(),
                pin_sha256: String::new(),
                insecure: false,
                remark: "🇩🇪 Германия".to_string(),
            })],
            meta: SubMeta::default(),
            routing: None,
            unreadable_lines: 0,
            source_host: host.to_string(),
        }
    }

    fn manifest_servers() -> Vec<ServerConfig> {
        vec![ServerConfig::Hy2(Hy2Config {
            password: "manifest-password".to_string(),
            host: "manifest.example".to_string(),
            port: 443,
            sni: "manifest.example".to_string(),
            pin_sha256: String::new(),
            insecure: false,
            remark: "🇬🇧 Британия".to_string(),
        })]
    }

    #[test]
    fn a_usable_manifest_replaces_servers_but_not_routing_or_meta() {
        let sub = sample_subscription("proxysvpn.com");
        let merged = merge(Ok(sub.clone()), Some((manifest_servers(), "proksya.xyz".to_string())))
            .expect("ok");
        assert_eq!(merged.servers, manifest_servers());
        assert_eq!(merged.source_host, "proxysvpn.com", "subscription's own source, unchanged");
    }

    #[test]
    fn no_usable_manifest_leaves_the_subscription_untouched() {
        let sub = sample_subscription("proxysvpn.com");
        let merged = merge(Ok(sub.clone()), None).expect("ok");
        assert_eq!(merged.servers, sub.servers);
        assert_eq!(merged.source_host, sub.source_host);
    }

    #[test]
    fn an_empty_manifest_server_list_does_not_replace_anything() {
        let sub = sample_subscription("proxysvpn.com");
        let merged = merge(Ok(sub.clone()), Some((Vec::new(), "proksya.xyz".to_string()))).expect("ok");
        assert_eq!(merged.servers, sub.servers);
    }

    #[test]
    fn a_failed_subscription_with_a_usable_manifest_returns_the_manifest_servers() {
        let merged = merge(
            Err(AppError::new(crate::errors::ErrorCode::SubUnreachable)),
            Some((manifest_servers(), "proksya.xyz".to_string())),
        )
        .expect("ok — the manifest saves this person");
        assert_eq!(merged.servers, manifest_servers());
        assert_eq!(merged.source_host, "proksya.xyz");
        assert!(merged.routing.is_none(), "the seed applies, same as no profile at all");
    }

    #[test]
    fn both_failing_returns_the_subscriptions_own_error() {
        let err = AppError::new(crate::errors::ErrorCode::SubUnreachable);
        let merged = merge(Err(err.clone()), None);
        assert_eq!(merged.unwrap_err().code, err.code);
    }

    #[test]
    fn both_failing_even_with_an_empty_manifest_list_returns_the_error() {
        let err = AppError::new(crate::errors::ErrorCode::SubUnreachable);
        let merged = merge(Err(err.clone()), Some((Vec::new(), "proksya.xyz".to_string())));
        assert_eq!(merged.unwrap_err().code, err.code);
    }

    #[test]
    fn the_production_key_table_holds_wf1_as_a_valid_ed25519_key() {
        // The path is on exactly while a key is compiled in. Every kid is
        // unique (a duplicate would make the lookup order decide which key
        // verifies) and every entry is a real curve point, so a typo in the
        // bytes fails here and not as "every manifest rejected" in the field.
        assert!(manifest_enabled());
        let kids: std::collections::HashSet<&str> = MANIFEST_KEYS.iter().map(|(kid, _)| *kid).collect();
        assert_eq!(kids.len(), MANIFEST_KEYS.len(), "duplicate kid");
        assert!(kids.contains("wf1"));
        for (kid, key) in MANIFEST_KEYS {
            assert!(VerifyingKey::from_bytes(key).is_ok(), "{kid} is not a valid Ed25519 public key");
        }
        let wf1 = MANIFEST_KEYS.iter().find(|(kid, _)| *kid == "wf1").map(|(_, k)| *k).expect("wf1");
        use base64::Engine;
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(wf1),
            "wq2wAT6NwZZpsyz8LmW7wUkTYOR/Eg6hpzKX9h/M2/w="
        );
    }
}
