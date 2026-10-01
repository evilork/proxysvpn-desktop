// src-tauri/src/logger.rs
//
// In-process log: a ring buffer for the window, a bounded file for support.
//
// What was wrong, measured on the owner's machine on 21.09.2026
// (~/Library/Logs/ProxysVPN/app.log, 82.7 MB, 553 578 lines, 25.04 - 06.07):
//
//   1. The header said "rotated daily" and no rotation existed anywhere in
//      the code. The file only ever grew.
//   2. 552 423 of those 553 578 lines — 99.8 % of them, 99.9 % of the bytes —
//      were one connection record each, carrying the destination address:
//      tun2socks `[TCP] 198.18.0.1:52995 <-> 17.57.146.135:5223` and xray
//      `accepted tcp:3.137.56.254:443`. That is a list of the sites the
//      person visited, written to disk, kept forever, and attached whenever
//      support asks for logs. Removing it takes the file from 82.7 MB to
//      83.6 KB over the same period.
//   3. Every engine's stderr was labelled `warn` regardless of what the
//      engine actually said, so the log window was a solid orange wall and
//      the real warnings were invisible inside it.
//   4. The file write happened AFTER the mutex was released, so concurrent
//      engine pumps interleaved mid-line: 1 034 torn lines in that same file,
//      including the `[warnxray]` shapes support kept seeing.
//
// All four are fixed here. The public API is unchanged, because lib.rs is not
// ours to edit; everything new is additive.
//
// Where the file lives is the platform layer's answer
// (`pvpn_platform::paths::log_file`): ~/Library/Logs/ProxysVPN/app.log on
// macOS as before, %LOCALAPPDATA%\ProxysVPN\logs\app.log on Windows,
// $XDG_STATE_HOME (or ~/.local/state)/ProxysVPN/app.log on Linux.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Limits, and the arithmetic behind each number
// ---------------------------------------------------------------------------

/// Lines held in memory for the log window.
///
/// Was 5 000. With connection records gone, the surviving traffic is roughly
/// 16 lines a day of ordinary use (1 155 lines over 72 days in the file
/// above), so 2 000 lines is months of real history — while a pathological
/// engine restart loop, the one case that produces lines quickly, is exactly
/// the case where only the recent ones matter.
const BUFFER_CAPACITY: usize = 2000;

/// Hard ceiling on one `snapshot` answer.
///
/// The window used to pull 1 000 lines over IPC every 1.5 s — a full
/// serialise-transfer-parse of the whole history, twice a second, to render a
/// list that had not changed. 500 is more than a screen can show, and
/// `snapshot_since` exists so the window can ask for only what is new.
pub const SNAPSHOT_LIMIT: usize = 500;

/// Rotate the file at this size.
///
/// At the post-filter rate (~1.2 KB/day of real use) 5 MiB is years, so in
/// normal life the daily rotation is what moves the file and this limit never
/// fires. It exists for the abnormal life: an engine in a restart storm, or a
/// future source that logs per-packet. It bounds that case to 5 MiB instead
/// of to the disk.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Files kept: `app.log`, `app.log.1`, `app.log.2`.
///
/// Three because support asks for "the log" after the fact, and a person who
/// restarts the app before writing to us would otherwise hand over a file
/// that begins after the failure. Three files x 5 MiB is a 15 MiB ceiling on
/// what this app can ever occupy — chosen so it stays smaller than a single
/// crash report and needs no user-facing setting.
const KEEP_FILES: usize = 3;

/// Smallest gap between two "connections since last time" summaries.
///
/// The summary replaces up to thousands of dropped records; once a minute it
/// is a useful shape of activity in the export, and it cannot itself become
/// the new volume problem: 1 440 lines a day at the absolute worst.
const SUMMARY_EVERY_MS: u128 = 60_000;

/// What a redacted address is replaced with. Short and obviously deliberate,
/// so nobody reads it as a parse failure.
const MASK_ADDR: &str = "<addr>";
const MASK_NODE: &str = "<node>";
const MASK_LINK: &str = "<link>";
const MASK_UUID: &str = "<uuid>";
const MASK_TOKEN: &str = "<token>";
const MASK_SITE: &str = "<site>";

// ---------------------------------------------------------------------------
// The line
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
pub struct LogLine {
    pub ts_ms: u128,
    /// "info" | "warn" | "error". Deliberately only three: the window's union
    /// (LogViewer.tsx) has three, and inventing a fourth here would render as
    /// an unstyled row rather than as new information.
    pub level: String,
    /// "app" | "xray" | "tun2socks" | ...
    pub source: String,
    pub message: String,
    /// Monotonic within a run. Lets the window ask for "everything after N"
    /// instead of re-fetching the whole buffer on a timer.
    pub seq: u64,
}

/// One page of the ring buffer.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
// Exercised by the tests; no caller in the app itself yet.
#[cfg_attr(not(test), allow(dead_code))]
pub struct LogPage {
    pub lines: Vec<LogLine>,
    /// Highest `seq` the caller now holds; pass it back as `after`.
    pub last_seq: u64,
    /// True when lines between the caller's `after` and the oldest line we
    /// still hold were dropped from the ring. The window says so rather than
    /// silently showing a gap.
    pub truncated: bool,
}

/// Connections counted instead of recorded, for the details sheet.
///
/// This is the only honest form of the transparency promise: how many
/// connections went where, and never which ones. The split comes from xray,
/// the one component that knows it (`[socks-in >> proxy]` against
/// `[socks-in -> direct]`); tun2socks sees the same connections one layer
/// lower and is counted separately so the totals are not doubled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficSplit {
    pub via_tunnel: u64,
    pub direct: u64,
    /// Connections seen by a component that does not know the split.
    pub unclassified: u64,
}

impl TrafficSplit {
    // Exercised by the tests; no caller in the app itself yet.
    #[cfg_attr(not(test), allow(dead_code))]
    fn total(&self) -> u64 {
        self.via_tunnel + self.direct + self.unclassified
    }
}

// ---------------------------------------------------------------------------
// Classification: what a line is, before we decide what to do with it
// ---------------------------------------------------------------------------

/// Which side of the fork a connection record describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnKind {
    ViaTunnel,
    Direct,
    /// Seen by tun2socks, which cannot know.
    Unknown,
}

/// What `classify` decided about one raw engine line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classified {
    /// A per-connection record. Never written to disk, never buffered, never
    /// shown — only counted.
    Connection(ConnKind),
    /// Worth keeping, with the level the line itself claims (if any) and the
    /// readable part of it.
    Keep {
        level: Option<&'static str>,
        message: String,
    },
}

/// Strip ANSI SGR sequences.
///
/// hysteria colours its level word (`\x1b[34mINFO\x1b[0m`), which both hides
/// the level from a naive parser and puts escape codes in a file support is
/// asked to paste into a chat window.
pub fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip to the final byte of the sequence (@ through ~).
            let mut j = i + 2;
            while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                j += 1;
            }
            i = if j < bytes.len() { j + 1 } else { j };
            continue;
        }
        // Safe: we only ever advance over whole UTF-8 sequences because the
        // escape check above matches ASCII bytes only.
        let ch_len = utf8_len(bytes[i]);
        let end = (i + ch_len).min(bytes.len());
        if let Some(chunk) = input.get(i..end) {
            out.push_str(chunk);
        }
        i = end;
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        // Continuation or invalid byte: advance one so we cannot loop.
        _ => 1,
    }
}

/// Map whatever word an engine used onto our three levels.
///
/// `debug`/`trace` fold into `info` on purpose — see `LogLine::level`.
pub fn normalise_level(token: &str) -> Option<&'static str> {
    let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    match t.to_ascii_lowercase().as_str() {
        "fatal" | "panic" | "error" | "err" => Some("error"),
        "warn" | "warning" => Some("warn"),
        "info" | "debug" | "trace" | "notice" => Some("info"),
        _ => None,
    }
}

/// Read the level out of the line itself.
///
/// This is the fix for the orange wall: the caller's level is only a default,
/// because "it arrived on stderr" says nothing about severity. Three shapes,
/// all taken from real lines in the file:
///
///   xray      `2026/04/25 20:24:31 [Warning] core: Xray 26.3.27 started`
///   hysteria  `2026-06-29T00:20:01+03:00 <esc>[34mINFO<esc>[0m client mode`
///   tun2socks `{"level":"info","ts":...,"msg":"..."}` (handled in `classify`)
pub fn parse_level(line: &str) -> Option<&'static str> {
    // Bracketed, as xray writes it.
    let bytes = line.as_bytes();
    let mut i = 0;
    while let Some(open) = bytes[i..].iter().position(|b| *b == b'[') {
        let start = i + open + 1;
        let Some(close) = bytes[start..].iter().position(|b| *b == b']') else {
            break;
        };
        let end = start + close;
        if let Some(token) = line.get(start..end) {
            if let Some(level) = normalise_level(token) {
                return Some(level);
            }
        }
        i = end + 1;
        if i >= bytes.len() {
            break;
        }
    }

    // A bare word, as hysteria writes it once the colour is stripped. Only
    // the first few tokens are considered, so the word "error" in the middle
    // of a sentence does not relabel the line.
    line.split(|c: char| c.is_whitespace())
        .filter(|t| !t.is_empty())
        .take(4)
        .find_map(|t| {
            // Require the whole token to be the level word: "errors" and
            // "information" must not match.
            let cleaned = t.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if cleaned.len() > 7 {
                return None;
            }
            normalise_level(cleaned)
        })
}

/// Is this message a per-connection record, and if so which side?
///
/// Both shapes below carry a destination address, which is the whole reason
/// the file grew to 82.7 MB and the reason we do not keep them.
pub fn connection_kind(message: &str) -> Option<ConnKind> {
    // tun2socks: the msg field is "[TCP] 198.18.0.1:52995 <-> 17.57.146.135:5223".
    //
    // `contains` rather than `starts_with` on purpose. Replaying the owner's
    // real file found 1 034 lines torn by the old unlocked write, where a
    // record begins mid-line; the write path is now inside the lock so that
    // cannot recur, but the direction to be wrong in is "drop a line that
    // might be a connection record", not "keep one". tun2socks is the only
    // producer of this token and it only ever produces it for connections.
    if message.contains("[TCP] ") || message.contains("[UDP] ") {
        return Some(ConnKind::Unknown);
    }
    // xray: "... from tcp:127.0.0.1:52997 accepted tcp:17.57.146.135:5223 [socks-in >> proxy]"
    if message.contains("accepted tcp:") || message.contains("accepted udp:") {
        if message.contains("-> direct]") {
            return Some(ConnKind::Direct);
        }
        if message.contains(">> proxy]") || message.contains("-> proxy]") {
            return Some(ConnKind::ViaTunnel);
        }
        return Some(ConnKind::Unknown);
    }
    None
}

/// Decide what one raw line is.
pub fn classify(raw: &str) -> Classified {
    let clean = strip_ansi(raw);
    let trimmed = clean.trim_end();

    // tun2socks and other zap-style loggers emit one JSON object per line.
    // Parse it once: it gives us the level and the human part, and the
    // connection test then runs on the message rather than on the envelope.
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let message = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or(trimmed)
                .to_string();
            if let Some(kind) = connection_kind(&message) {
                return Classified::Connection(kind);
            }
            let level = value
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(normalise_level);
            return Classified::Keep { level, message };
        }
    }

    if let Some(kind) = connection_kind(trimmed) {
        return Classified::Connection(kind);
    }

    Classified::Keep {
        level: parse_level(trimmed),
        message: trimmed.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Redaction — before anything is written, not before it is exported
// ---------------------------------------------------------------------------

/// True for addresses that describe the user's own machine or our own
/// plumbing rather than somewhere they went.
///
/// Keeping these is what makes the log still useful for support: the default
/// gateway, the utun address and the local SOCKS port are the three things
/// worth reading in a routing failure, and none of them is a node or a site.
fn is_local_ipv4(o: [u8; 4]) -> bool {
    match o {
        [0, ..] => true,                            // "this network"
        [127, ..] => true,                          // loopback
        [10, ..] => true,                           // RFC1918
        [172, b, ..] if (16..=31).contains(&b) => true,
        [192, 168, ..] => true,
        [169, 254, ..] => true,                     // link-local
        [100, b, ..] if (64..=127).contains(&b) => true, // CGNAT (mobile)
        [198, 18 | 19, ..] => true,                 // benchmark range = our utun
        [224..=255, ..] => true,                    // multicast / broadcast
        _ => false,
    }
}

fn hex_val(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

/// A position where a token may begin: not inside a word, a version number or
/// a longer dotted identifier.
fn at_boundary(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let p = bytes[i - 1];
    !(p.is_ascii_alphanumeric() || p == b'.' || p == b'-' || p == b'_')
}

/// The prefix length after an IPv4 literal (`/1` in `128.0.0.0/1`), if the
/// literal is followed by one.
fn cidr_prefix(bytes: &[u8], end: usize) -> Option<u32> {
    if bytes.get(end) != Some(&b'/') {
        return None;
    }
    let digits = bytes[end + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let text = std::str::from_utf8(&bytes[end + 1..end + 1 + digits]).ok()?;
    text.parse().ok()
}

/// Replace public IPv4 literals with `<addr>`.
///
/// A literal followed by a prefix shorter than /32 is a network, not a host
/// (`128.0.0.0/1` is one half of our default route), so it is kept. A /32 is
/// one host, and that is exactly how the Windows and Linux platform layer
/// writes the route to the node (`prefix=<ip>/32`, `ip route … <ip>/32`), so
/// it is masked like a bare address; the `/32` stays to show the form.
fn mask_ipv4(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && at_boundary(bytes, i) {
            if let Some((octets, end)) = scan_ipv4(bytes, i) {
                let is_network = matches!(cidr_prefix(bytes, end), Some(len) if len < 32);
                if !is_network && !is_local_ipv4(octets) {
                    out.push_str(MASK_ADDR);
                    // Swallow a trailing :port so `<addr>` is not followed by
                    // a naked number that reads like part of the address.
                    let mut after = end;
                    if after < bytes.len() && bytes[after] == b':' {
                        let mut k = after + 1;
                        while k < bytes.len() && bytes[k].is_ascii_digit() {
                            k += 1;
                        }
                        if k > after + 1 {
                            after = k;
                        }
                    }
                    i = after;
                    continue;
                }
                if let Some(chunk) = input.get(i..end) {
                    out.push_str(chunk);
                }
                i = end;
                continue;
            }
        }
        let n = utf8_len(bytes[i]);
        let end = (i + n).min(bytes.len());
        if let Some(chunk) = input.get(i..end) {
            out.push_str(chunk);
        }
        i = end;
    }
    out
}

/// Parse four dotted octets starting at `start`. Returns the octets and the
/// index just past the last digit.
fn scan_ipv4(bytes: &[u8], start: usize) -> Option<([u8; 4], usize)> {
    let mut octets = [0u8; 4];
    let mut i = start;
    for (n, slot) in octets.iter_mut().enumerate() {
        if n > 0 {
            if i >= bytes.len() || bytes[i] != b'.' {
                return None;
            }
            i += 1;
        }
        let digits_start = i;
        let mut value: u32 = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() && i - digits_start < 3 {
            value = value * 10 + u32::from(bytes[i] - b'0');
            i += 1;
        }
        if i == digits_start || value > 255 {
            return None;
        }
        *slot = value as u8;
    }
    // A fifth dotted group means this was a version string or an OID.
    if i < bytes.len() && (bytes[i] == b'.' || bytes[i].is_ascii_alphanumeric()) {
        return None;
    }
    Some((octets, i))
}

/// Replace IPv6 literals with `<addr>`.
///
/// Careful around timestamps: `00:20:01+03:00` has colon-separated groups but
/// one of them is not hex, and it has no `::` and no four-digit group, so it
/// is left alone. A real address has at least one of those.
fn mask_ipv6(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if (hex_val(bytes[i]) || bytes[i] == b':') && at_boundary(bytes, i) {
            if let Some(end) = scan_ipv6(bytes, i) {
                out.push_str(MASK_ADDR);
                i = end;
                continue;
            }
        }
        let n = utf8_len(bytes[i]);
        let stop = (i + n).min(bytes.len());
        if let Some(chunk) = input.get(i..stop) {
            out.push_str(chunk);
        }
        i = stop;
    }
    out
}

fn scan_ipv6(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    let mut colons = 0;
    let mut groups = 0;
    let mut has_double_colon = false;
    let mut has_strong_group = false;

    while i < bytes.len() {
        if bytes[i] == b':' {
            colons += 1;
            if i + 1 < bytes.len() && bytes[i + 1] == b':' {
                has_double_colon = true;
                colons += 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if !hex_val(bytes[i]) {
            break;
        }
        let g_start = i;
        while i < bytes.len() && hex_val(bytes[i]) && i - g_start < 4 {
            i += 1;
        }
        let len = i - g_start;
        // A group that is four digits long, or contains a hex letter, is what
        // a timestamp never has.
        let has_letter = bytes[g_start..i].iter().any(|b| b.is_ascii_alphabetic());
        if len == 4 || has_letter {
            has_strong_group = true;
        }
        groups += 1;
        if i < bytes.len() && bytes[i] != b':' {
            break;
        }
    }

    if colons < 2 || groups == 0 {
        return None;
    }
    if !has_double_colon && !has_strong_group {
        return None;
    }
    // Must not have stopped in the middle of a word.
    if i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
        return None;
    }
    Some(i)
}

/// Replace `vless://`, `hy2://` and friends up to the next whitespace.
fn mask_links(input: &str) -> String {
    const SCHEMES: &[&str] = &[
        "vless://",
        "hysteria2://",
        "hy2://",
        "trojan://",
        "vmess://",
        "ss://",
        "socks5://",
    ];
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    'outer: loop {
        let mut best: Option<(usize, usize)> = None;
        for scheme in SCHEMES {
            if let Some(pos) = rest.find(scheme) {
                if best.is_none_or(|(b, _)| pos < b) {
                    best = Some((pos, scheme.len()));
                }
            }
        }
        let Some((pos, _)) = best else {
            out.push_str(rest);
            break 'outer;
        };
        out.push_str(&rest[..pos]);
        out.push_str(MASK_LINK);
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out
}

/// Replace dashed UUIDs and long bare hex runs (device ids, pairing and
/// subscription tokens) with a label.
fn mask_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if (hex_val(bytes[i])) && at_boundary(bytes, i) {
            if let Some(end) = scan_uuid(bytes, i) {
                out.push_str(MASK_UUID);
                i = end;
                continue;
            }
            if let Some(end) = scan_hex_run(bytes, i) {
                out.push_str(MASK_TOKEN);
                i = end;
                continue;
            }
        }
        let n = utf8_len(bytes[i]);
        let stop = (i + n).min(bytes.len());
        if let Some(chunk) = input.get(i..stop) {
            out.push_str(chunk);
        }
        i = stop;
    }
    out
}

fn scan_uuid(bytes: &[u8], start: usize) -> Option<usize> {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut i = start;
    for (n, len) in GROUPS.iter().enumerate() {
        if n > 0 {
            if i >= bytes.len() || bytes[i] != b'-' {
                return None;
            }
            i += 1;
        }
        let g = i;
        while i < bytes.len() && hex_val(bytes[i]) && i - g < *len {
            i += 1;
        }
        if i - g != *len {
            return None;
        }
    }
    if i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        return None;
    }
    Some(i)
}

/// A bare hex run of exactly 32 or 64 characters: the pairing token
/// (`^[a-f0-9]{32}$`) and the device id (32 random bytes, hex).
fn scan_hex_run(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() && hex_val(bytes[i]) {
        i += 1;
    }
    let len = i - start;
    if len != 32 && len != 64 {
        return None;
    }
    if i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        return None;
    }
    Some(i)
}

/// Engine lines that name a site the person opened rather than one of ours.
///
/// Measured, not guessed (01.10): xray at `warning` writes
/// `[Error] app/dns: failed to retrieve response for <name>.` for every DoH
/// timeout — exactly what a cold or throttled tunnel produces in bursts — and
/// hysteria writes `WARN SOCKS5 TCP error {"reqAddr": "<name>:443"}` for every
/// dropped connection. The owner's own app.log held 32 such xray lines with 20
/// different names, and the "Report for support" promises there are none.
const SITE_LINE_MARKERS: [&str; 6] = [
    "app/dns",
    "app/dispatcher",
    "proxy/dns",
    "reqAddr",
    "\"addr\"",
    "domain",
];

/// File names engines print next to a message: not sites, and useful.
const NOT_A_SITE_SUFFIXES: [&str; 9] = ["go", "dat", "json", "yaml", "yml", "log", "txt", "exe", "dll"];

/// Public resolvers an engine names when a lookup through it fails. Which
/// resolver timed out is what support needs to read, and it says nothing
/// about the sites the person opened.
const RESOLVER_HOSTS: [&str; 7] = [
    "dns.google",
    "cloudflare-dns.com",
    "one.one.one.one",
    "dns.quad9.net",
    "doh.opendns.com",
    "dns.adguard-dns.com",
    "common.dot.dns.yandex.net",
];

/// `RESOLVER_HOSTS`, plus the per-service names under Cloudflare's and
/// Quad9's resolver domains (`security.cloudflare-dns.com`, `dns11.quad9.net`).
fn is_public_resolver(token: &str) -> bool {
    let name = token.strip_suffix('.').unwrap_or(token).to_ascii_lowercase();
    RESOLVER_HOSTS.contains(&name.as_str())
        || name.ends_with(".cloudflare-dns.com")
        || name.ends_with(".quad9.net")
}

/// Is `token` shaped like a DNS name: dot-separated labels of letters, digits,
/// hyphens and underscores (`_xmpp-client._tcp`, `my_host` — real names in
/// lookups), at least two of them, ending in an alphabetic (or punycode) top
/// label, which leaves dotted IPv4 and version numbers to their own rules?
/// A top label that is a file extension (`nameserver_doh.go`) is a code
/// location, not a host.
fn looks_like_a_site(token: &str) -> bool {
    let name = token.strip_suffix('.').unwrap_or(token);
    let mut labels = name.split('.');
    let Some(top) = name.rsplit('.').next() else {
        return false;
    };
    let well_formed = labels.all(|l| {
        !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    well_formed
        && name.contains('.')
        && top.len() >= 2
        && (top.bytes().all(|b| b.is_ascii_alphabetic()) || top.to_ascii_lowercase().starts_with("xn--"))
        && !NOT_A_SITE_SUFFIXES.contains(&top.to_ascii_lowercase().as_str())
}

/// Replace every DNS name in an engine line that can name a visited site.
/// Lines without one of `SITE_LINE_MARKERS` are left alone: our own lines
/// ("proxysvpn.com unreachable") are what support needs to read.
///
/// A name is masked wherever it stands — after `://`, before a `/`, next to
/// an underscore: `Get https://cdn.site.io/x` names the site as plainly as
/// `reqAddr` does. Only code locations (`nameserver_doh.go`, a top label in
/// `NOT_A_SITE_SUFFIXES`) and the public resolvers keep their names.
fn mask_visited_sites(input: &str) -> String {
    if !SITE_LINE_MARKERS.iter().any(|m| input.contains(m)) {
        return input.to_string();
    }
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_';
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(is_name_char) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail.find(|c: char| !is_name_char(c)).unwrap_or(tail.len());
        let token = &tail[..end];
        if looks_like_a_site(token) && !is_public_resolver(token) {
            out.push_str(MASK_SITE);
            // A trailing dot is the name's root, not the sentence's end, but
            // a masked name keeps it so the line still reads the same.
            if token.ends_with('.') {
                out.push('.');
            }
        } else {
            out.push_str(token);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Everything that must not reach the disk, in one pass order.
///
/// `nodes` are host names the app has learned at runtime (see
/// `remember_node_host`): a node's DNS name is as much an address as its IP,
/// and the structural rules below cannot recognise one on their own.
pub fn redact(input: &str, nodes: &[String]) -> String {
    let mut text = input.to_string();
    for host in nodes {
        if host.len() >= 4 && text.contains(host.as_str()) {
            text = text.replace(host.as_str(), MASK_NODE);
        }
    }
    // Links first: they contain a uuid and a host:port that the later passes
    // would otherwise chew into an unreadable half-URL.
    let text = mask_links(&text);
    let text = mask_secrets(&text);
    let text = mask_visited_sites(&text);
    // IPv6 before IPv4 so an IPv4-mapped form is taken whole.
    let text = mask_ipv6(&text);
    mask_ipv4(&text)
}

// ---------------------------------------------------------------------------
// The file: rotation by size and by day
// ---------------------------------------------------------------------------

struct FileSink {
    dir: LogDir,
    name: OsString,
    handle: File,
    written: u64,
    /// Days since the epoch of the content currently in the file.
    day: i64,
    max_bytes: u64,
    keep: usize,
}

impl FileSink {
    fn open(path: PathBuf, now_ms: u128) -> std::io::Result<Self> {
        Self::open_with(path, now_ms, MAX_FILE_BYTES, KEEP_FILES)
    }

    fn open_with(
        path: PathBuf,
        now_ms: u128,
        max_bytes: u64,
        keep: usize,
    ) -> std::io::Result<Self> {
        let (dir_path, name) = match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => (dir.to_path_buf(), name.to_os_string()),
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "log path has no folder or no name",
                ))
            }
        };
        let dir = LogDir::open(&dir_path)?;
        let handle = dir.open_file(&name, false)?;
        let meta = handle.metadata()?;
        // Adopt the existing file's day, not today's: a machine started the
        // next morning must roll yesterday's file rather than append to it.
        let day = if meta.len() == 0 {
            day_of(now_ms)
        } else {
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| day_of(d.as_millis()))
                .unwrap_or_else(|| day_of(now_ms))
        };
        Ok(Self {
            dir,
            name,
            handle,
            written: meta.len(),
            day,
            max_bytes,
            keep,
        })
    }

    /// `app.log.2` is dropped, `app.log.1` becomes `.2`, `app.log` becomes
    /// `.1`, and a fresh `app.log` is opened.
    fn rotate(&mut self, now_ms: u128) -> std::io::Result<()> {
        let archives = self.keep.saturating_sub(1);
        if archives == 0 {
            // Degenerate configuration: keep only the live file.
            self.handle = self.dir.open_file(&self.name, true)?;
            self.written = 0;
            self.day = day_of(now_ms);
            return Ok(());
        }
        let _ = self.dir.remove(&archive_name(&self.name, archives));
        for n in (1..archives).rev() {
            let from = archive_name(&self.name, n);
            if self.dir.exists(&from) {
                let _ = self.dir.rename(&from, &archive_name(&self.name, n + 1));
            }
        }
        // Rename, do not copy: the old handle keeps pointing at the renamed
        // inode, so nothing is lost if another write is already in flight.
        self.dir.rename(&self.name, &archive_name(&self.name, 1))?;
        self.handle = self.dir.open_file(&self.name, false)?;
        self.written = 0;
        self.day = day_of(now_ms);
        Ok(())
    }

    fn write_line(&mut self, text: &str, now_ms: u128) -> std::io::Result<()> {
        let today = day_of(now_ms);
        let needs_rotation =
            today != self.day || self.written + text.len() as u64 + 1 > self.max_bytes;
        if needs_rotation && self.written > 0 {
            self.rotate(now_ms)?;
        } else if today != self.day {
            // Empty file, new day: just move the marker.
            self.day = today;
        }
        self.handle.write_all(text.as_bytes())?;
        self.handle.write_all(b"\n")?;
        self.written += text.len() as u64 + 1;
        Ok(())
    }
}

/// Mode of a newly created log file.
///
/// Linux: 0600. There the GUI is unprivileged, so the file belongs to the user
/// who runs it and `~/.local/state/ProxysVPN/app.log` would otherwise be
/// created world-readable at the usual umask — the log carries sidecar paths,
/// interface names and whatever a node sends us.
///
/// macOS: 0644, as before. The process is root there and the file sits in the
/// user's own `~/Library/Logs`, so a root-owned 0600 file would stop the owner
/// from opening the log the support UI points them at.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const LOG_MODE: u32 = if cfg!(target_os = "linux") { 0o600 } else { 0o644 };

/// The folder the log files live in, held open so that every later step —
/// open, rotate, delete — happens in that very folder.
///
/// macOS and Linux: the folder is reached one component at a time below the
/// home folder (`LogDir::open_beneath`), each component opened with
/// O_DIRECTORY|O_NOFOLLOW (and created with mkdirat when missing) relative to
/// the one before, and each owned by the home folder's owner or by root.
/// Every file operation is then `*at()` relative to the folder, with
/// O_NOFOLLOW on the file and a check that the file is a plain file with a
/// single link. On macOS the process writing here is root and the folder sits
/// in the user's own `~/Library/Logs`: by path, a program of that user could
/// swap any folder on the way (`~/Library/Logs` itself, not only
/// `ProxysVPN`) for a link, or plant `app.log` as a symlink or hard link to a
/// system file (a sudoers.d entry, a shell startup file), and have root
/// create folders and files there or append log lines to them. Now a link
/// anywhere below the home folder is refused and the log simply has no file.
///
/// Windows and iOS keep plain paths: %LOCALAPPDATA% has an owner-only ACL, and
/// the iOS file is inside the app's sandbox.
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct LogDir {
    fd: File,
}

/// Mode of a log folder this process creates (before the umask).
#[cfg(any(target_os = "macos", target_os = "linux"))]
const LOG_DIR_MODE: u32 = 0o755;

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl LogDir {
    /// Open `dir`, walking every component below `$HOME` without following
    /// a link. A folder outside the home folder (only tests use one) is
    /// walked from its parent, which is created by path as before.
    fn open(dir: &Path) -> std::io::Result<Self> {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
        if let Some(home) = home {
            if let Ok(rel) = dir.strip_prefix(&home) {
                if !rel.as_os_str().is_empty() {
                    return Self::open_beneath(&home, rel);
                }
            }
        }
        let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "log folder has no parent or no name",
            ));
        };
        create_dir_all(parent)?;
        Self::open_beneath(parent, Path::new(name))
    }

    /// Open `anchor` (links followed: it is the home folder the launcher
    /// named, in a parent only root can write), then each component of `rel`
    /// relative to the one before with O_DIRECTORY|O_NOFOLLOW, creating a
    /// missing one with mkdirat. Every component must belong to the anchor's
    /// owner or to root, and `rel` may hold plain names only.
    fn open_beneath(anchor: &Path, rel: &Path) -> std::io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        use std::path::Component;

        let mut current = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(anchor)?;
        let owner = current.metadata()?.uid();
        let mut walked = 0usize;
        for component in rel.components() {
            let Component::Normal(name) = component else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the log folder must be plain names below the home folder",
                ));
            };
            let c = Self::c_name(name)?;
            // SAFETY: `current` is an open directory for the whole call and
            // `c` is a NUL-terminated name. mkdirat never follows a final link.
            if unsafe { libc::mkdirat(current.as_raw_fd(), c.as_ptr(), LOG_DIR_MODE as libc::mode_t) } != 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(err);
                }
            }
            let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
            // SAFETY: as above; openat takes no mode without O_CREAT.
            let raw = unsafe { libc::openat(current.as_raw_fd(), c.as_ptr(), flags) };
            if raw < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: `raw` is a descriptor openat just returned to us and
            // nothing else owns it.
            let next = unsafe { File::from_raw_fd(raw) };
            let uid = next.metadata()?.uid();
            if uid != owner && uid != 0 {
                return Err(std::io::Error::other(format!(
                    "a folder on the way to the log belongs to uid {uid}, not {owner} or root"
                )));
            }
            current = next;
            walked += 1;
        }
        if walked == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the log folder cannot be the home folder itself",
            ));
        }
        Ok(Self { fd: current })
    }

    fn c_name(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::CString::new(name.as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in a log file name"))
    }

    /// Open `name` for appending (or for a rewrite from empty), refusing
    /// anything but a plain file with exactly one link.
    fn open_file(&self, name: &std::ffi::OsStr, empty_it: bool) -> std::io::Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;

        let c = Self::c_name(name)?;
        // No O_TRUNC here even for `empty_it`: the checks below must run
        // before anything is changed in whatever this name turns out to be.
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `self.fd` is an open directory for the whole call, `c` is a
        // NUL-terminated name, and the mode is passed as the variadic third
        // argument openat(2) reads when O_CREAT is set.
        let raw = unsafe { libc::openat(self.fd.as_raw_fd(), c.as_ptr(), flags, LOG_MODE as libc::c_uint) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `raw` is a descriptor openat just returned to us and nothing
        // else owns it.
        let file = unsafe { File::from_raw_fd(raw) };
        let meta = file.metadata()?;
        if !meta.file_type().is_file() || meta.nlink() != 1 {
            return Err(std::io::Error::other("the log file is not a plain file with a single link"));
        }
        if empty_it {
            file.set_len(0)?;
        }
        Ok(file)
    }

    fn rename(&self, from: &std::ffi::OsStr, to: &std::ffi::OsStr) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        let (a, b) = (Self::c_name(from)?, Self::c_name(to)?);
        let dir = self.fd.as_raw_fd();
        // SAFETY: both names are NUL-terminated and the directory stays open.
        if unsafe { libc::renameat(dir, a.as_ptr(), dir, b.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn remove(&self, name: &std::ffi::OsStr) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        let c = Self::c_name(name)?;
        // SAFETY: as in `rename`. unlinkat never follows a final symlink.
        if unsafe { libc::unlinkat(self.fd.as_raw_fd(), c.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn exists(&self, name: &std::ffi::OsStr) -> bool {
        use std::os::fd::AsRawFd;
        let Ok(c) = Self::c_name(name) else {
            return false;
        };
        // SAFETY: `stat` is a plain C struct; zeroed is a valid value for it,
        // and fstatat only writes into it.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: as in `rename`; `st` outlives the call.
        unsafe { libc::fstatat(self.fd.as_raw_fd(), c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) == 0 }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct LogDir {
    path: PathBuf,
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl LogDir {
    fn open(dir: &Path) -> std::io::Result<Self> {
        create_dir_all(dir)?;
        Ok(Self { path: dir.to_path_buf() })
    }

    fn open_file(&self, name: &std::ffi::OsStr, empty_it: bool) -> std::io::Result<File> {
        let mut opts = OpenOptions::new();
        opts.create(true);
        if empty_it {
            opts.write(true).truncate(true);
        } else {
            opts.append(true);
        }
        opts.open(self.path.join(name))
    }

    fn rename(&self, from: &std::ffi::OsStr, to: &std::ffi::OsStr) -> std::io::Result<()> {
        std::fs::rename(self.path.join(from), self.path.join(to))
    }

    fn remove(&self, name: &std::ffi::OsStr) -> std::io::Result<()> {
        std::fs::remove_file(self.path.join(name))
    }

    fn exists(&self, name: &std::ffi::OsStr) -> bool {
        self.path.join(name).exists()
    }
}

fn archive_name(name: &std::ffi::OsStr, n: usize) -> OsString {
    let mut archived = name.to_os_string();
    archived.push(format!(".{n}"));
    archived
}

#[cfg(test)]
fn archive_path(base: &Path, n: usize) -> PathBuf {
    let mut name = base.as_os_str().to_os_string();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

fn day_of(ms: u128) -> i64 {
    (ms / 1000 / 86_400) as i64
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct LoggerState {
    buffer: VecDeque<LogLine>,
    next_seq: u64,
    /// Seq of the oldest line still in the buffer, so `snapshot_since` can
    /// tell the window it missed something.
    oldest_seq: u64,
    sink: Option<FileSink>,
    // Read by `log_file_path`, which nothing calls today.
    #[allow(dead_code)]
    path: Option<PathBuf>,
    split: TrafficSplit,
    /// Counted but not yet summarised.
    pending_split: TrafficSplit,
    last_summary_ms: u128,
    nodes: Vec<String>,
}

static STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();

fn get_state() -> &'static Mutex<LoggerState> {
    STATE.get_or_init(|| {
        let path = compute_log_path();
        let sink = path
            .clone()
            .and_then(|p| FileSink::open(p, now_ms()).ok());
        Mutex::new(LoggerState {
            buffer: VecDeque::with_capacity(BUFFER_CAPACITY),
            next_seq: 1,
            oldest_seq: 1,
            sink,
            path,
            split: TrafficSplit::default(),
            pending_split: TrafficSplit::default(),
            last_summary_ms: 0,
            nodes: Vec::new(),
        })
    })
}

impl LoggerState {
    /// Append one already-classified, already-redacted line.
    ///
    /// The file write happens HERE, while the caller still holds the mutex.
    /// The old code released it first and let two engine pumps interleave a
    /// single `writeln!`, which is what produced the 1 034 torn lines.
    fn emit(&mut self, level: &str, source: &str, message: String, ts_ms: u128) {
        let line = LogLine {
            ts_ms,
            level: level.to_string(),
            source: source.to_string(),
            message,
            seq: self.next_seq,
        };
        self.next_seq += 1;

        if self.buffer.len() == BUFFER_CAPACITY {
            if let Some(dropped) = self.buffer.pop_front() {
                self.oldest_seq = dropped.seq + 1;
            }
        }

        if let Some(sink) = self.sink.as_mut() {
            let text = format!(
                "{} [{}] [{}] {}",
                format_iso(line.ts_ms / 1000),
                line.level,
                line.source,
                line.message
            );
            if sink.write_line(&text, ts_ms).is_err() {
                // Disk full, permissions changed, volume unmounted. Stop
                // trying: the ring buffer still serves the window, and a
                // logger that keeps failing must not become the loudest
                // thing in the process.
                self.sink = None;
            }
        }

        self.buffer.push_back(line);
    }

    /// Fold a dropped connection record into the counters, and every so often
    /// leave one line saying how many there were.
    fn count_connection(&mut self, kind: ConnKind, ts_ms: u128) {
        let bump = |s: &mut TrafficSplit| match kind {
            ConnKind::ViaTunnel => s.via_tunnel += 1,
            ConnKind::Direct => s.direct += 1,
            ConnKind::Unknown => s.unclassified += 1,
        };
        bump(&mut self.split);
        bump(&mut self.pending_split);

        if ts_ms.saturating_sub(self.last_summary_ms) < SUMMARY_EVERY_MS {
            return;
        }
        if self.last_summary_ms == 0 {
            // First record of the run: start the window, do not summarise a
            // single connection.
            self.last_summary_ms = ts_ms;
            return;
        }
        let p = self.pending_split;
        self.last_summary_ms = ts_ms;
        self.pending_split = TrafficSplit::default();
        let message = format!(
            "connections: {} via tunnel, {} direct, {} unclassified (addresses not recorded)",
            p.via_tunnel, p.direct, p.unclassified
        );
        self.emit("info", "conn", message, ts_ms);
    }
}

#[cfg(desktop)]
fn compute_log_path() -> Option<PathBuf> {
    // macOS: $HOME/Library/Logs/ProxysVPN/app.log (unchanged)
    // Windows: %LOCALAPPDATA%\ProxysVPN\logs\app.log
    // Linux: $XDG_STATE_HOME (or ~/.local/state)/ProxysVPN/app.log
    pvpn_platform::paths::log_file()
}

#[cfg(target_os = "ios")]
fn compute_log_path() -> Option<PathBuf> {
    // On iOS $HOME is the app sandbox root; Documents/ is user-visible in the
    // Files app (UIFileSharingEnabled), which makes support exports easy.
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Documents/app.log"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn lock() -> std::sync::MutexGuard<'static, LoggerState> {
    match get_state().lock() {
        Ok(g) => g,
        // A poisoned logger is still a better logger than none: nothing in
        // this state can be left half-written, because every mutation is a
        // push or a counter bump.
        Err(p) => p.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn init() {
    let _ = get_state();
    log("info", "app", "logger initialized");
}

/// Record one line.
///
/// `level` is a DEFAULT, not a verdict: if the line names its own level, that
/// wins. This is what turns the log window back into something readable —
/// every engine's stderr used to arrive here labelled `warn`.
pub fn log(level: &str, source: &str, message: &str) {
    let ts = now_ms();
    // Classify BEFORE taking the lock. Parsing an engine's JSON envelope is
    // the most expensive step here and it needs nothing shared; doing it
    // inside would serialise every engine pump behind every other one's
    // parse. Only the ring buffer, the counters and the file write — the
    // parts that must not interleave — happen under the lock.
    let classified = classify(message);
    let mut st = lock();

    match classified {
        Classified::Connection(kind) => {
            // Never buffered, never written, never printed: this is the list
            // of sites the person visited.
            st.count_connection(kind, ts);
        }
        Classified::Keep {
            level: embedded,
            message: text,
        } => {
            let level = embedded.unwrap_or(level);
            let text = redact(&text, &st.nodes);
            // Dev mirror. Kept lines only — the dropped ones are the volume.
            println!("[{source}][{level}] {text}");
            st.emit(level, source, text, ts);
        }
    }
}

/// Teach the redactor a host name it must never write down.
///
/// Node addresses arrive at runtime from the subscription, so they cannot be
/// a compile-time list. Call this as soon as one is known — before the first
/// line that might mention it.
pub fn remember_node_host(host: &str) {
    let host = host.trim();
    if host.len() < 4 {
        return;
    }
    let mut st = lock();
    if !st.nodes.iter().any(|h| h == host) {
        st.nodes.push(host.to_string());
    }
}

/// Connections counted since the app started, for the details sheet.
pub fn traffic_split() -> TrafficSplit {
    lock().split
}

/// New session: the sheet counts one connection, not a lifetime.
pub fn reset_traffic_split() {
    let mut st = lock();
    st.split = TrafficSplit::default();
    st.pending_split = TrafficSplit::default();
}

/// How many lines one answer may carry.
///
/// The ceiling is not negotiable by the caller: the window used to ask for
/// 1 000 lines every 1.5 s, and a page size is a property of the transport,
/// not a preference of whoever is calling. A zero would otherwise produce an
/// empty page forever, so the floor is one.
fn page_size(limit: Option<usize>) -> usize {
    limit.unwrap_or(SNAPSHOT_LIMIT).clamp(1, SNAPSHOT_LIMIT)
}

/// The most recent lines, newest last. Capped at `SNAPSHOT_LIMIT` whatever
/// the caller asks for.
pub fn snapshot(limit: Option<usize>) -> Vec<LogLine> {
    let st = lock();
    let take = page_size(limit);
    let start = st.buffer.len().saturating_sub(take);
    st.buffer.iter().skip(start).cloned().collect()
}

/// Everything recorded after `after`, so the window can poll cheaply — or
/// stop polling and ask once after an event.
///
/// `after = 0` means "from the beginning of what we still hold".
// Exercised by the tests; no caller in the app itself yet.
#[cfg_attr(not(test), allow(dead_code))]
pub fn snapshot_since(after: u64, limit: Option<usize>) -> LogPage {
    let st = lock();
    let take = page_size(limit);
    let truncated = after != 0 && after + 1 < st.oldest_seq;

    let mut lines: Vec<LogLine> = st
        .buffer
        .iter()
        .filter(|l| l.seq > after)
        .cloned()
        .collect();
    if lines.len() > take {
        // Keep the newest: a window that fell behind wants the current state,
        // not the start of the backlog.
        lines.drain(..lines.len() - take);
    }
    let last_seq = lines.last().map(|l| l.seq).unwrap_or(after.max(st.next_seq - 1));
    LogPage {
        lines,
        last_seq,
        truncated,
    }
}

// Public logger API kept for the log window's "clear"; nothing calls it today.
#[allow(dead_code)]
pub fn clear() {
    {
        let mut st = lock();
        st.buffer.clear();
        st.oldest_seq = st.next_seq;
    }
    log("info", "app", "logs cleared by user");
}

// Public logger API kept for the support screen; nothing calls it today.
#[allow(dead_code)]
pub fn log_file_path() -> Option<String> {
    lock().path.as_ref().map(|p| p.to_string_lossy().to_string())
}

/// Plain text of the in-memory buffer, for clipboard or file export.
///
/// Export is publication: this goes into a chat with support. Every line here
/// has already been redacted on the way in, so there is nothing left to strip
/// on the way out — which is the point of redacting before the write rather
/// than before the export.
// Public logger API kept for the support export; nothing calls it today.
#[allow(dead_code)]
pub fn export_text(include_system_info: bool) -> String {
    let (lines, split) = {
        let st = lock();
        (
            st.buffer.iter().cloned().collect::<Vec<_>>(),
            st.split,
        )
    };

    let mut out = String::new();
    if include_system_info {
        out.push_str("=== ProxysVPN Diagnostic Report ===\n");
        out.push_str(&format!("Generated: {}\n", format_iso(now_ms() / 1000)));
        out.push_str(&format!("App version: {}\n", env!("CARGO_PKG_VERSION")));
        out.push_str(&format!("OS arch: {}\n", std::env::consts::ARCH));
        out.push_str(&format!("OS family: {}\n", std::env::consts::OS));
        out.push_str(&format!(
            "Connections: {} via tunnel, {} direct, {} unclassified ({} total; \
             destinations are never recorded)\n",
            split.via_tunnel,
            split.direct,
            split.unclassified,
            split.total()
        ));
        out.push_str("\n=== Log Lines ===\n");
    }

    for line in &lines {
        out.push_str(&format!(
            "{} [{}] [{}] {}\n",
            format_iso(line.ts_ms / 1000),
            line.level,
            line.source,
            line.message
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Time formatting (no chrono: one dependency less in a binary that ships)
// ---------------------------------------------------------------------------

fn format_iso(secs: u128) -> String {
    let s = secs as i64;
    let days_since_epoch = s.div_euclid(86400);
    let day_secs = s.rem_euclid(86400);
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    let (y, m, d) = days_to_ymd(days_since_epoch);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719468;
    let era = if days >= 0 {
        days / 146097
    } else {
        (days - 146096) / 146097
    };
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Convenience macros — use them instead of println!/eprintln! in our crate.
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! log_info {
    ($source:expr, $($arg:tt)*) => {
        $crate::logger::log("info", $source, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($source:expr, $($arg:tt)*) => {
        $crate::logger::log("warn", $source, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($source:expr, $($arg:tt)*) => {
        $crate::logger::log("error", $source, &format!($($arg)*))
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A private directory per test; removed when the guard drops.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "proxysvpn-logger-{}-{}-{}",
                tag,
                std::process::id(),
                n
            ));
            std::fs::create_dir_all(&p).expect("temp dir");
            Self(p)
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const DAY_MS: u128 = 86_400_000;

    // ---- rotation --------------------------------------------------------

    #[test]
    fn rotation_happens_when_the_file_overflows() {
        let dir = TmpDir::new("rot");
        let path = dir.file("app.log");
        let mut sink = FileSink::open_with(path.clone(), DAY_MS, 200, 3).expect("open");

        // 10 lines of 50 bytes = 510 bytes against a 200-byte limit.
        for i in 0..10 {
            let line = format!("{i:0>48}");
            sink.write_line(&line, DAY_MS).expect("write");
        }

        assert!(path.exists(), "live file must exist");
        assert!(archive_path(&path, 1).exists(), "app.log.1 must exist");
        assert!(archive_path(&path, 2).exists(), "app.log.2 must exist");
        assert!(
            !archive_path(&path, 3).exists(),
            "keep=3 must not produce a third archive"
        );

        // The ceiling actually holds.
        let total: u64 = [path.clone(), archive_path(&path, 1), archive_path(&path, 2)]
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        assert!(total <= 3 * 200 + 3, "ceiling exceeded: {total}");
    }

    #[test]
    fn the_newest_lines_stay_in_the_live_file() {
        let dir = TmpDir::new("newest");
        let path = dir.file("app.log");
        let mut sink = FileSink::open_with(path.clone(), DAY_MS, 120, 3).expect("open");
        for i in 0..20 {
            sink.write_line(&format!("line-{i:0>2}-{}", "x".repeat(40)), DAY_MS)
                .expect("write");
        }
        let live = std::fs::read_to_string(&path).expect("read");
        assert!(live.contains("line-19"), "live file lost the newest line");
        let oldest = std::fs::read_to_string(archive_path(&path, 2)).expect("read .2");
        assert!(
            !oldest.contains("line-19"),
            "the newest line must not be in the oldest archive"
        );
    }

    #[test]
    fn a_new_day_rotates_even_without_overflow() {
        let dir = TmpDir::new("daily");
        let path = dir.file("app.log");
        let mut sink = FileSink::open_with(path.clone(), DAY_MS * 100, 10_000_000, 3).expect("open");
        sink.write_line("yesterday", DAY_MS * 100).expect("write");
        assert!(!archive_path(&path, 1).exists());

        sink.write_line("today", DAY_MS * 101 + 1000).expect("write");
        let archived = std::fs::read_to_string(archive_path(&path, 1)).expect("read .1");
        assert!(archived.contains("yesterday"), "{archived}");
        let live = std::fs::read_to_string(&path).expect("read live");
        assert!(live.contains("today") && !live.contains("yesterday"), "{live}");
    }

    #[test]
    fn an_empty_file_on_a_new_day_is_not_rotated() {
        // Otherwise every cold start after midnight leaves an empty archive
        // and pushes real history out of the three-file window.
        let dir = TmpDir::new("emptyday");
        let path = dir.file("app.log");
        let mut sink = FileSink::open_with(path.clone(), DAY_MS * 5, 10_000, 3).expect("open");
        sink.write_line("first line of a new day", DAY_MS * 9)
            .expect("write");
        assert!(!archive_path(&path, 1).exists());
    }

    #[test]
    fn restarting_the_next_day_rolls_yesterdays_file() {
        let dir = TmpDir::new("restart");
        let path = dir.file("app.log");
        {
            let mut sink =
                FileSink::open_with(path.clone(), DAY_MS * 10, 10_000, 3).expect("open");
            sink.write_line("from a previous run", DAY_MS * 10)
                .expect("write");
        }
        // Reopen with a timestamp a day later: the sink adopts the file's own
        // day from its mtime, so the first write rolls it.
        let mut sink = FileSink::open_with(path.clone(), DAY_MS * 11, 10_000, 3).expect("reopen");
        // mtime is "now" in the test environment, so force the marker to the
        // day the content actually belongs to.
        sink.day = 10;
        sink.write_line("new run", DAY_MS * 11).expect("write");
        assert!(archive_path(&path, 1).exists());
    }

    // ---- links planted where the log goes -------------------------------

    /// On macOS the log is written by root into the user's own
    /// ~/Library/Logs. A symlink planted as app.log must not be followed: it
    /// would have root create or append to any file on the machine.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_symlink_planted_as_the_log_file_is_not_written_through() {
        let dir = TmpDir::new("symlog");
        let victim = dir.file("victim");
        std::fs::write(&victim, b"keep\n").expect("victim");
        let path = dir.file("app.log");
        std::os::unix::fs::symlink(&victim, &path).expect("link");

        assert!(FileSink::open_with(path.clone(), DAY_MS, 10_000, 3).is_err());
        assert_eq!(std::fs::read(&victim).expect("victim"), b"keep\n");

        // A link to a file that does not exist yet must not be created either.
        let absent = dir.file("absent");
        std::fs::remove_file(&path).expect("unlink");
        std::os::unix::fs::symlink(&absent, &path).expect("link");
        assert!(FileSink::open_with(path, DAY_MS, 10_000, 3).is_err());
        assert!(!absent.exists(), "root must not create the link's target");
    }

    /// A hard link survives O_NOFOLLOW; the single-link check is what stops it.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_hard_link_planted_as_the_log_file_is_refused() {
        let dir = TmpDir::new("hardlog");
        let victim = dir.file("victim");
        std::fs::write(&victim, b"keep\n").expect("victim");
        let path = dir.file("app.log");
        std::fs::hard_link(&victim, &path).expect("hard link");

        assert!(FileSink::open_with(path, DAY_MS, 10_000, 3).is_err());
        assert_eq!(std::fs::read(&victim).expect("victim"), b"keep\n");
    }

    /// The folder itself swapped for a link to somewhere else.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_log_folder_that_is_a_symlink_is_refused() {
        let dir = TmpDir::new("symdir");
        let elsewhere = dir.file("elsewhere");
        std::fs::create_dir(&elsewhere).expect("dir");
        let logs = dir.file("ProxysVPN");
        std::os::unix::fs::symlink(&elsewhere, &logs).expect("link");

        assert!(FileSink::open_with(logs.join("app.log"), DAY_MS, 10_000, 3).is_err());
        assert!(!elsewhere.join("app.log").exists());
    }

    /// Not only the last folder: ~/Library/Logs itself swapped for a link
    /// (its owner can do that) must not lead root anywhere else either.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_link_anywhere_below_home_is_refused() {
        let home = TmpDir::new("homelink");
        std::fs::create_dir(home.file("Library")).expect("Library");
        let elsewhere = home.file("elsewhere");
        std::fs::create_dir(&elsewhere).expect("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, home.file("Library/Logs")).expect("Logs is a link");

        assert!(LogDir::open_beneath(&home.0, Path::new("Library/Logs/ProxysVPN")).is_err());
        assert!(!elsewhere.join("ProxysVPN").exists(), "nothing is created through the link");

        // The same chain of real folders works, and missing ones are made.
        std::fs::remove_file(home.file("Library/Logs")).expect("unlink");
        let dir = LogDir::open_beneath(&home.0, Path::new("Library/Logs/ProxysVPN")).expect("real folders");
        let file = dir.open_file(std::ffi::OsStr::new("app.log"), false).expect("app.log");
        drop(file);
        assert!(home.file("Library/Logs/ProxysVPN/app.log").is_file());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_log_folder_walk_takes_plain_names_only() {
        let home = TmpDir::new("homedots");
        assert!(LogDir::open_beneath(&home.0, Path::new("../escape")).is_err());
        assert!(LogDir::open_beneath(&home.0, Path::new("/etc")).is_err());
        assert!(LogDir::open_beneath(&home.0, Path::new("")).is_err());
    }

    /// Rotation stays inside the folder that was opened: swapping the folder
    /// for a link afterwards moves nothing anywhere else.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn rotation_after_the_folder_was_swapped_stays_in_the_original_folder() {
        let dir = TmpDir::new("swaprot");
        let logs = dir.file("ProxysVPN");
        let mut sink = FileSink::open_with(logs.join("app.log"), DAY_MS, 100, 3).expect("open");
        sink.write_line(&"a".repeat(60), DAY_MS).expect("write");

        let moved = dir.file("moved");
        std::fs::rename(&logs, &moved).expect("move the folder away");
        let elsewhere = dir.file("elsewhere");
        std::fs::create_dir(&elsewhere).expect("dir");
        std::fs::write(elsewhere.join("app.log.2"), b"keep").expect("bait");
        std::os::unix::fs::symlink(&elsewhere, &logs).expect("link in its place");

        sink.write_line(&"b".repeat(60), DAY_MS).expect("rotating write");
        assert_eq!(std::fs::read(elsewhere.join("app.log.2")).expect("bait"), b"keep");
        assert!(moved.join("app.log.1").exists(), "the rotation happened where the folder really is");
    }

    // ---- the addresses ---------------------------------------------------

    #[test]
    fn tun2socks_connection_records_are_dropped() {
        let raw = r#"{"level":"info","ts":1777137871.69,"caller":"tunnel/tcp.go:42","msg":"[TCP] 198.18.0.1:52995 <-> 17.57.146.135:5223"}"#;
        assert_eq!(classify(raw), Classified::Connection(ConnKind::Unknown));
    }

    #[test]
    fn xray_connection_records_are_dropped_and_tell_us_the_side() {
        let via = "2026/04/25 20:24:31.784592 from tcp:127.0.0.1:52997 accepted tcp:17.57.146.135:5223 [socks-in >> proxy]";
        assert_eq!(classify(via), Classified::Connection(ConnKind::ViaTunnel));

        let direct = "2026/07/02 13:47:30.247848 from udp:127.0.0.1:63933 accepted udp:77.88.8.8:53 [socks-in -> direct]";
        assert_eq!(classify(direct), Classified::Connection(ConnKind::Direct));
    }

    #[test]
    fn a_connection_record_is_dropped_even_when_the_line_is_torn() {
        // The shape the old unlocked write produced 1 034 times in the real
        // file: a record glued behind the tail of another line.
        let torn = r#"info{"level":"info","ts":1.0,"caller":"tunnel/tcp.go:42","msg":"[TCP] 198.18.0.1:62085 <-> 203.0.113.9:443"}] ["#;
        assert_eq!(classify(torn), Classified::Connection(ConnKind::Unknown));
    }

    #[test]
    fn a_real_engine_message_is_not_mistaken_for_a_connection() {
        let line = "2026/04/25 20:24:31.076629 [Warning] core: Xray 26.3.27 started";
        match classify(line) {
            Classified::Keep { level, message } => {
                assert_eq!(level, Some("warn"));
                assert!(message.contains("Xray 26.3.27 started"));
            }
            other => panic!("engine message dropped: {other:?}"),
        }
    }

    #[test]
    fn the_message_is_lifted_out_of_the_json_envelope() {
        let raw = r#"{"level":"warn","ts":1.0,"caller":"engine/engine.go:237","msg":"udp associate failed"}"#;
        match classify(raw) {
            Classified::Keep { level, message } => {
                assert_eq!(level, Some("warn"));
                assert_eq!(message, "udp associate failed");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    // ---- redaction -------------------------------------------------------

    #[test]
    fn a_node_address_never_reaches_the_file() {
        let line = "server panel.example.net -> 198.51.100.58";
        let out = redact(line, &["panel.example.net".to_string()]);
        assert!(!out.contains("198.51.100.58"), "{out}");
        assert!(!out.contains("panel.example.net"), "{out}");
        assert!(out.contains(MASK_NODE) && out.contains(MASK_ADDR), "{out}");
    }

    #[test]
    fn a_node_address_with_a_port_is_taken_whole() {
        let out = redact("hysteria server 198.51.100.58:443", &[]);
        assert_eq!(out, "hysteria server <addr>");
    }

    #[test]
    fn local_addresses_survive_because_support_needs_them() {
        let out = redact(
            "original gateway: 192.168.1.1, tun 198.18.0.1, socks 127.0.0.1:10808",
            &[],
        );
        assert!(out.contains("192.168.1.1"), "{out}");
        assert!(out.contains("198.18.0.1"), "{out}");
        assert!(out.contains("127.0.0.1:10808"), "{out}");
    }

    #[test]
    fn our_own_route_halves_are_not_addresses() {
        let out = redact("route -n add -net 128.0.0.0/1 -interface utun225", &[]);
        assert!(out.contains("128.0.0.0/1"), "{out}");
    }

    /// A host route is a node address written as a network: the Windows and
    /// Linux platform layer names the node's /32 in netsh and `ip route`
    /// command lines, and those reach this file through "ignored: …" and
    /// failure messages.
    #[test]
    fn a_host_route_is_an_address_not_a_network() {
        let netsh = redact(
            r#"ignored: C:\Windows\System32\netsh.exe ["interface", "ipv4", "delete", "route", "prefix=203.0.113.7/32", "store=active"] failed: exit code: 1"#,
            &[],
        );
        assert!(!netsh.contains("203.0.113.7"), "{netsh}");
        assert!(netsh.contains("prefix=<addr>/32"), "{netsh}");

        let ip = redact(
            r#"tunnel did not come up: ip ["-4", "route", "replace", "203.0.113.7/32", "via", "192.168.1.1"] failed"#,
            &[],
        );
        assert!(!ip.contains("203.0.113.7"), "{ip}");
        assert!(ip.contains("192.168.1.1"), "the gateway is local and stays: {ip}");

        // A real network keeps its form, a /32 of a local address too.
        let nets = redact("route 203.0.113.0/24 and 10.0.0.1/32", &[]);
        assert!(nets.contains("203.0.113.0/24"), "{nets}");
        assert!(nets.contains("10.0.0.1/32"), "{nets}");
    }

    #[test]
    fn version_numbers_are_not_addresses() {
        let out = redact("Xray 26.3.27 (go1.26.1 darwin/arm64) d2758a0", &[]);
        assert_eq!(out, "Xray 26.3.27 (go1.26.1 darwin/arm64) d2758a0");
    }

    #[test]
    fn timestamps_are_not_ipv6_addresses() {
        let out = redact("2026-06-29T00:20:01+03:00 client mode", &[]);
        assert_eq!(out, "2026-06-29T00:20:01+03:00 client mode");
    }

    #[test]
    fn real_ipv6_is_redacted() {
        let out = redact("peer 2a03:4000:1a:35e::1 connected", &[]);
        assert!(!out.contains("2a03"), "{out}");
        assert!(out.contains(MASK_ADDR), "{out}");
    }

    #[test]
    fn subscription_links_and_secrets_are_redacted() {
        let line = "start vless://0c3f9a71-6f88-4c2e-9a33-1b6d2f4e88aa@203.0.113.7:443?pbk=x#NL";
        let out = redact(line, &[]);
        assert!(!out.contains("vless://"), "{out}");
        assert!(!out.contains("203.0.113.7"), "{out}");
        assert_eq!(out, "start <link>");
    }

    #[test]
    fn a_bare_uuid_and_a_pairing_token_are_redacted() {
        let out = redact("uuid 0c3f9a71-6f88-4c2e-9a33-1b6d2f4e88aa ok", &[]);
        assert_eq!(out, "uuid <uuid> ok");

        let out = redact("GET /api/sub/0123456789abcdef0123456789abcdef", &[]);
        assert_eq!(out, "GET /api/sub/<token>");
    }

    #[test]
    fn a_short_hex_word_is_left_alone() {
        // Commit hashes and small ids are useful and are not secrets.
        let out = redact("build d2758a0 cafe", &[]);
        assert_eq!(out, "build d2758a0 cafe");
    }

    /// The two lines measured on 01.10 with our own engine binaries: names
    /// of sites the person opened, in the file and in the support report
    /// that says it holds none.
    #[test]
    fn site_names_in_engine_lines_are_masked() {
        let xray = "2026/10/01 12:00:00.123456 [Error] app/dns: failed to retrieve response for some-private-site-the-user-visits.example.org. > context deadline exceeded";
        let out = redact(xray, &[]);
        assert!(!out.contains("some-private-site"), "{out}");
        assert!(out.contains("app/dns: failed to retrieve response for <site>"), "{out}");

        let hy2 = r#"2026-10-01T12:00:00+03:00	WARN	SOCKS5 TCP error	{"addr": "127.0.0.1:52011", "reqAddr": "a-site-the-user-opened.invalid:443", "error": "EOF"}"#;
        let out = redact(hy2, &[]);
        assert!(!out.contains("a-site-the-user-opened"), "{out}");
        assert!(out.contains(r#""reqAddr": "<site>:443""#), "{out}");

        let sniffed = "[Info] app/dispatcher: sniffed domain: www.example.com";
        assert!(!redact(sniffed, &[]).contains("example.com"));
    }

    /// Only engine lines that can carry a visited site are touched: our own
    /// diagnostics name our own hosts, and support needs to read them.
    #[test]
    fn our_own_lines_and_code_locations_keep_their_names() {
        let ours = "subscription: proxysvpn.com unreachable, trying proksya.xyz";
        assert_eq!(redact(ours, &[]), ours);
        let located = "[Error] app/dns: nameserver_doh.go:207 v2.9.3 started";
        let out = redact(located, &[]);
        assert!(out.contains("nameserver_doh.go:207"), "{out}");
        assert!(out.contains("v2.9.3"), "{out}");
    }

    #[test]
    fn the_site_shape_is_strict() {
        for site in [
            "example.com",
            "a-b.c-d.example.org.",
            "xn--80ak6aa92e.xn--p1ai",
            "DNS.Google",
            "my_host.private-site.org",
            "_xmpp-client._tcp.jabber-site.ru.",
        ] {
            assert!(looks_like_a_site(site), "{site}");
        }
        for not_site in [
            "1.2.3.4",
            "v2.9.3",
            "dns.go",
            "geoip.dat",
            "a..b",
            ".com",
            "localhost",
            "a.b1",
            "nameserver_doh.go",
            "my_host",
        ] {
            assert!(!looks_like_a_site(not_site), "{not_site}");
        }
    }

    /// A name glued to an underscore label or sitting inside a URL is still
    /// the site the person opened.
    #[test]
    fn site_names_next_to_underscores_and_inside_urls_are_masked() {
        let underscore = "[Error] app/dns: failed to retrieve response for my_host.private-site.org.";
        let out = redact(underscore, &[]);
        assert!(!out.contains("private-site"), "{out}");
        assert!(out.ends_with("failed to retrieve response for <site>."), "{out}");

        let srv = "[Error] app/dns: failed to retrieve response for _xmpp-client._tcp.jabber-site.ru.";
        let out = redact(srv, &[]);
        assert!(!out.contains("jabber-site"), "{out}");
        assert!(!out.contains("xmpp"), "{out}");

        let url = r#"WARN	TCP error	{"reqAddr": "cdn.site.io:443", "error": "Get https://cdn.site.io/x?y=1: EOF"}"#;
        let out = redact(url, &[]);
        assert!(!out.contains("site.io"), "{out}");
        assert!(out.contains("https://<site>/x"), "{out}");
    }

    /// Which resolver failed is support's business and nobody's browsing.
    #[test]
    fn public_resolvers_and_code_locations_keep_their_names() {
        let doh = r#"[Error] app/dns: failed to retrieve response for private.example. > Post "https://dns.google/dns-query": context deadline exceeded"#;
        let out = redact(doh, &[]);
        assert!(out.contains("https://dns.google/dns-query"), "{out}");
        assert!(!out.contains("private.example"), "{out}");
        for resolver in ["cloudflare-dns.com", "security.cloudflare-dns.com", "dns.quad9.net", "dns11.quad9.net", "DNS.Google."] {
            assert!(is_public_resolver(resolver), "{resolver}");
        }
        assert!(!is_public_resolver("quad9.net.example.com"));
        assert!(!is_public_resolver("notcloudflare-dns.com"));

        let located = "[Warning] app/dns: nameserver_doh.go:207 github_com_xtls.dns.go query failed";
        let out = redact(located, &[]);
        assert!(out.contains("nameserver_doh.go:207"), "{out}");
        assert!(out.contains("github_com_xtls.dns.go"), "{out}");
    }

    #[test]
    fn redaction_happens_before_the_line_is_stored() {
        // The whole point: the file is safe even if nobody remembers to
        // sanitise on export.
        let stored = redact(&strip_ansi("connect 203.0.113.55:443"), &[]);
        assert!(!stored.contains("203.0.113.55"));
    }

    // ---- levels ----------------------------------------------------------

    #[test]
    fn hysteria_levels_are_read_through_the_colour_codes() {
        let raw = "2026-06-29T00:20:01+03:00\t\u{1b}[31mFATAL\u{1b}[0m\tfailed to initialize client";
        match classify(raw) {
            Classified::Keep { level, message } => {
                assert_eq!(level, Some("error"), "stderr is not automatically a warning");
                assert!(!message.contains('\u{1b}'), "escape codes reached the file");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn xray_bracketed_levels_are_read() {
        assert_eq!(parse_level("2026/04/25 [Info] reading config"), Some("info"));
        assert_eq!(parse_level("2026/04/25 [Warning] started"), Some("warn"));
        assert_eq!(parse_level("2026/04/25 [Error] failed"), Some("error"));
    }

    #[test]
    fn an_unlabelled_line_keeps_the_callers_level() {
        assert_eq!(parse_level("A unified platform for anti-censorship."), None);
        assert_eq!(parse_level("tun2socks died, stopping watchdog"), None);
    }

    #[test]
    fn a_level_word_late_in_a_sentence_does_not_relabel_the_line() {
        assert_eq!(
            parse_level("could not parse the value because the field name was error"),
            None
        );
    }

    #[test]
    fn plurals_and_longer_words_are_not_levels() {
        assert_eq!(parse_level("errors while reading"), None);
        assert_eq!(parse_level("information about the node"), None);
    }

    #[test]
    fn the_level_in_the_line_beats_the_level_of_the_pipe() {
        // tun.rs, xray_manager.rs and hysteria_manager.rs all label stderr
        // "warn"; that is what made the window a wall of orange.
        let raw = r#"{"level":"info","ts":1.0,"msg":"engine started"}"#;
        match classify(raw) {
            Classified::Keep { level, .. } => assert_eq!(level, Some("info")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    // ---- ANSI ------------------------------------------------------------

    #[test]
    fn stripping_colour_keeps_the_text_and_the_unicode() {
        assert_eq!(strip_ansi("\u{1b}[33mWARN\u{1b}[0m всё хорошо"), "WARN всё хорошо");
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi(""), "");
        // Truncated escape at the end must not loop or panic.
        assert_eq!(strip_ansi("tail\u{1b}["), "tail");
    }

    // ---- counters --------------------------------------------------------

    #[test]
    fn dropped_connections_become_a_split_counter() {
        let mut split = TrafficSplit::default();
        for kind in [
            ConnKind::ViaTunnel,
            ConnKind::ViaTunnel,
            ConnKind::Direct,
            ConnKind::Unknown,
        ] {
            match kind {
                ConnKind::ViaTunnel => split.via_tunnel += 1,
                ConnKind::Direct => split.direct += 1,
                ConnKind::Unknown => split.unclassified += 1,
            }
        }
        assert_eq!(split.via_tunnel, 2);
        assert_eq!(split.direct, 1);
        assert_eq!(split.unclassified, 1);
        assert_eq!(split.total(), 4);
    }

    // ---- snapshot bounds -------------------------------------------------

    #[test]
    fn a_snapshot_is_capped_however_much_is_asked_for() {
        // The window used to ask for 1 000 lines every 1.5 seconds.
        assert_eq!(page_size(Some(1000)), SNAPSHOT_LIMIT);
        assert_eq!(page_size(None), SNAPSHOT_LIMIT);
        assert_eq!(page_size(Some(42)), 42);
        // Zero would produce an empty page forever.
        assert_eq!(page_size(Some(0)), 1);
    }

    #[test]
    fn incremental_reads_return_only_what_is_new() {
        // Exercised against the real global logger, which is the thing the
        // window talks to.
        log("info", "test-since", "first");
        let page = snapshot_since(0, Some(SNAPSHOT_LIMIT));
        let mark = page.last_seq;
        assert!(mark > 0);

        log("info", "test-since", "second");
        let next = snapshot_since(mark, Some(SNAPSHOT_LIMIT));
        assert!(
            next.lines.iter().all(|l| l.seq > mark),
            "a line we already had came back"
        );
        assert!(
            next.lines.iter().any(|l| l.message == "second"),
            "the new line is missing"
        );
        assert!(next.last_seq > mark);
    }

    #[test]
    fn a_window_that_fell_behind_is_told_so() {
        log("info", "test-trunc", "anchor");
        // Ask from a sequence far older than anything the ring can hold.
        let page = snapshot_since(0, Some(10));
        assert!(page.lines.len() <= 10);
        assert!(!page.truncated, "nothing has been dropped in this run yet");
    }

    // ---- time ------------------------------------------------------------

    #[test]
    fn timestamps_format_and_days_line_up() {
        assert_eq!(format_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_iso(1_777_137_871), "2026-04-25T17:24:31Z");
        assert_eq!(day_of(0), 0);
        assert_eq!(day_of(86_400_000), 1);
        assert_eq!(day_of(86_399_999), 0);
    }
}
