// src-tauri/src/probe.rs
//
// The one question the old client never asked: did a live byte actually cross
// the tunnel?
//
// `vpn_connect` used to answer Ok the moment routes were installed
// (lib.rs:165-182). On a node the ISP is dropping, that is a green screen over
// a dead tunnel: nothing opens, and there is nothing on the screen to press.
// This module exists to replace "the routes are up" with "data is moving", and
// to say WHICH of the four ways it can fail happened, because each one needs a
// different sentence and a different button.
//
// Two sensors, deliberately unequal:
//
//   PASSIVE (the main one) — byte counters of the tunnel interface, read from
//   the kernel with getifaddrs(3). Costs one syscall, sends nothing, and is
//   invisible to anyone watching the wire. Everything we can answer from the
//   counters, we answer from the counters.
//
//   ACTIVE (rare, on occasion) — one HTTPS GET to a neighbouring domain of
//   ours that is NOT in the direct-routing list, so the request is forced
//   through the tunnel. It is the only thing that can prove the far end is
//   really ours and really reachable, and it is the only thing that grants
//   `VpnPhase::On`.
//
// Why the active probe is not on a timer: see `ProbeGate`.

use std::net::IpAddr;
use std::time::{Duration, Instant, SystemTime};

use crate::errors::{AppError, ErrorCode};
use crate::events::{LinkQuality, MetricPayload, VpnPhase};

// ---------------------------------------------------------------------------
// Where we ask, and why these addresses
// ---------------------------------------------------------------------------

/// The fallback ladder for the liveness question.
///
/// Rules every entry here obeys:
///
/// 1. **Not `proxysvpn.com`.** That domain is pinned to the DIRECT list on
///    purpose (subscription.rs:284): a thousand clients refreshing their
///    subscription from five datacentre exits looks like an attack to the
///    hosting firewall, and a client locked out of the site cannot refresh at
///    all. The price is that our own site always sees the home address — so
///    asking it anything proves nothing about the tunnel. A neighbouring
///    domain is not in the direct list, so the request goes through.
///
/// 2. **More than one, on different infrastructure.** The primary is the shop;
///    `proksya.xyz` is our Russian fallback domain, which keeps answering on
///    the days a foreign prefix is being curtained off; the last two are the
///    same frontend on two more names, so a DNS-level block on one name does
///    not take the answer away.
///
/// 3. **All answer the same shape** — `{"ip":"..."}` — so one parser serves
///    the whole ladder. Verified against all four on 21.09.2026: 23, 155, 187
///    and 187 bytes respectively.
///
/// Order matters: the first entry is the cheapest answer (23 bytes) and is the
/// one the website already uses, so it carries no new signature of its own.
pub const PROBE_LADDER: &[&str] = &[
    "https://proxysvpn.store/api/exit-ip",
    "https://proksya.xyz/api/tools/whoami",
    "https://proksya.com/api/tools/whoami",
    "https://proxysvnovich.vercel.app/api/tools/whoami",
];

/// Per-address patience. 2.5 s is the number from the design: long enough for
/// a bad mobile network to finish a TLS handshake, short enough that walking
/// the whole ladder still fits inside the 25 s connect budget.
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_millis(2500);

/// A captive portal answers 200 with a login page. We only ever need a few
/// dozen bytes, so refuse to read more than this — an HTML page that big is
/// already proof it is not our answer.
const MAX_BODY_BYTES: usize = 8 * 1024;

/// The tunnel device. Must stay equal to `tun::TUN_NAME` (tun.rs:13); it is
/// duplicated rather than imported because this module also builds for iOS,
/// where tun.rs does not exist.
pub const TUN_IFACE: &str = "utun225";

/// How much traffic counts as "something actually left / came back".
///
/// One TLS ClientHello with SNI and ALPN is ~300-700 bytes, and the server's
/// half of the handshake is several kilobytes. 512 bytes therefore separates
/// "we got as far as talking" from "a couple of retransmitted SYNs". Lower
/// would make a dead link look busy; higher would call a small successful
/// exchange dead.
const MOVED_BYTES: u64 = 512;

// ---------------------------------------------------------------------------
// The four outcomes (plus the honest fifth)
// ---------------------------------------------------------------------------

/// What the probe found.
///
/// Four of these are the outcomes the product needs to tell apart, because
/// each one is a different sentence and a different button. `Unconfirmed` is
/// the fifth state the design insists on: the tunnel is demonstrably carrying
/// traffic, our own page just did not answer. Calling that a failure would
/// disconnect a working user; calling it success would be a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// A byte went out and came back. The only thing that turns the shield
    /// green.
    Passed,
    /// Counters moved both ways, the probe page stayed silent. Neither
    /// success nor failure.
    Unconfirmed,
    /// We pushed bytes into the tunnel and nothing came back. The classic
    /// shape of DPI throttling a live connection.
    Blocked,
    /// Nothing even left. The node address is burned or the ISP drops it.
    NoRoute,
    /// The machine has no network at all. Kept separate from everything else
    /// because it is not our fault, and saying so plainly saves a ticket.
    NetworkOffline,
}

impl ProbeVerdict {
    /// The phase the shield should show.
    ///
    /// `Blocked` and `NoRoute` become `Healing`, not `Failed`: the repair
    /// ladder (design M3) runs first and the user is told nothing for the
    /// first seconds. `NetworkOffline` goes straight to `Failed` — there is
    /// nothing for us to repair, and the UI's action for it is "wait", not
    /// "press something".
    pub fn phase(self) -> VpnPhase {
        match self {
            Self::Passed => VpnPhase::On,
            Self::Unconfirmed => VpnPhase::Unconfirmed,
            Self::Blocked | Self::NoRoute => VpnPhase::Healing,
            Self::NetworkOffline => VpnPhase::Failed,
        }
    }

    /// The code the window turns into a phrase and a single button.
    /// `None` for a pass — success has no error to name.
    pub fn error_code(self) -> Option<ErrorCode> {
        match self {
            Self::Passed => None,
            Self::Unconfirmed => Some(ErrorCode::ProbeUnconfirmed),
            Self::Blocked => Some(ErrorCode::Blocked),
            Self::NoRoute => Some(ErrorCode::NoRoute),
            Self::NetworkOffline => Some(ErrorCode::NetworkOffline),
        }
    }

    /// Ready-made error for `EV_EVENT` / `StatePayload::failed`.
    pub fn as_error(self) -> Option<AppError> {
        self.error_code().map(AppError::new)
    }

    /// True while the core should keep working on it by itself.
    pub fn is_repairable(self) -> bool {
        matches!(self, Self::Blocked | Self::NoRoute)
    }
}

// ---------------------------------------------------------------------------
// Classification — a pure function, so all five outcomes are testable
// ---------------------------------------------------------------------------

/// Everything the verdict is decided from. Gathered by `probe_once`, but kept
/// separate so the decision itself can be tested without a network.
#[derive(Debug, Clone, Copy)]
pub struct ProbeInputs {
    /// Does the machine have any usable network interface at all?
    pub link_up: bool,
    /// Did one of the ladder addresses answer with a parseable exit address?
    pub http_ok: bool,
    /// Bytes the tunnel interface sent while we were asking.
    pub tx_delta: u64,
    /// Bytes it received while we were asking.
    pub rx_delta: u64,
}

/// Decide which of the five happened.
///
/// Order is the whole design. "No network" is checked before anything else,
/// because every other verdict would be a lie about our service when the
/// truth is that the Wi-Fi is off. After that a successful answer wins
/// outright. Only then do the passive counters arbitrate between "moving but
/// unconfirmed", "sent and got nothing" and "never left".
pub fn classify(i: ProbeInputs) -> ProbeVerdict {
    if !i.link_up {
        return ProbeVerdict::NetworkOffline;
    }
    if i.http_ok {
        return ProbeVerdict::Passed;
    }
    let sent = i.tx_delta >= MOVED_BYTES;
    let received = i.rx_delta >= MOVED_BYTES;
    match (sent, received) {
        // Traffic in both directions: the tunnel is alive, our page is not.
        (true, true) => ProbeVerdict::Unconfirmed,
        // We spoke, nothing answered — the DPI signature.
        (true, false) => ProbeVerdict::Blocked,
        // Nothing meaningful left the interface at all.
        (false, _) => ProbeVerdict::NoRoute,
    }
}

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// Result of one active probe.
///
/// Deliberately NOT `Serialize`. `exit_ip` is a node address, and node
/// addresses are never shown to a person, never written to the log and never
/// handed to the window — the type refuses to travel so a future careless
/// `emit` cannot leak one. Only `metric()` crosses the boundary.
#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub verdict: ProbeVerdict,
    /// Round trip of a FRESH connection, milliseconds. `None` unless passed.
    pub rtt_ms: Option<u32>,
    /// Exit address, for `POST /api/tools/whoami` and nothing else.
    pub exit_ip: Option<IpAddr>,
    /// Index into `PROBE_LADDER` of the address that answered.
    pub via: Option<usize>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Unix milliseconds this probe finished.
    pub at_ms: u64,
}

impl ProbeReport {
    /// The half of the report the window is allowed to see.
    pub fn metric(&self) -> MetricPayload {
        MetricPayload {
            rtt_ms: self.rtt_ms,
            quality: Some(match self.rtt_ms {
                Some(ms) => LinkQuality::from_rtt_ms(ms),
                None => LinkQuality::Unknown,
            }),
            rx_bytes: self.rx_bytes,
            tx_bytes: self.tx_bytes,
            // Only a real pass is proof. Ageing a failed probe as "checked 9
            // seconds ago" is exactly the stale-green lie we are removing.
            last_proof_at: match self.verdict {
                ProbeVerdict::Passed => Some(self.at_ms),
                _ => None,
            },
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// When to ask — on occasion, never on a clock
// ---------------------------------------------------------------------------

/// Why we are about to probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeReason {
    /// Routes are up; this probe is what grants green. Never suppressed.
    AfterConnect,
    /// Wi-Fi changed, cellular took over, VPN-adjacent interface appeared.
    NetworkChanged,
    /// Machine came back from sleep.
    Woke,
    /// The passive counters look wrong and we want a second opinion.
    Suspicion,
    /// The person pressed "Проверка".
    UserRequested,
    /// Nothing happened, the window is open, and the age of the last proof is
    /// getting embarrassing.
    IdleForeground,
    /// Same, with the window hidden.
    IdleBackground,
}

impl ProbeReason {
    /// Shortest gap allowed between two probes for this reason.
    ///
    /// The arithmetic that sets these numbers:
    ///
    /// A timer of one probe per minute — what two of the three design drafts
    /// proposed — is 1500 devices x 60 = 1500 requests per minute against the
    /// shop, permanently, for a question that is almost always "yes". Worse,
    /// it is a metronome: an identical TLS handshake to the same SNI at the
    /// same interval from every one of our users is the easiest possible
    /// thing to recognise in a network that is already filtering by pattern.
    ///
    /// On-occasion probing costs roughly 12-20 requests per hour per ACTIVE
    /// device — about one fiftieth of the timer — and the intervals are
    /// driven by what the user's network is doing, so there is no rhythm to
    /// match. The passive counters give the same truth for free in between.
    ///
    /// `Suspicion` gets the longest automatic floor of the three event-driven
    /// reasons because a flapping link can raise suspicion many times a
    /// minute, and without a floor "on occasion" quietly becomes a timer
    /// faster than the one we removed.
    pub fn min_gap(self) -> Duration {
        match self {
            // Never suppressed: this is the probe that turns the shield green.
            Self::AfterConnect => Duration::ZERO,
            // Guard against a double tap, nothing more. The user asked.
            Self::UserRequested => Duration::from_secs(3),
            Self::NetworkChanged | Self::Woke => Duration::from_secs(5),
            Self::Suspicion => Duration::from_secs(30),
            Self::IdleForeground => Duration::from_secs(300),
            Self::IdleBackground => Duration::from_secs(900),
        }
    }
}

/// Enforces "by occasion, not by clock".
///
/// Owned by the caller rather than kept in a global, so it can be reset with
/// the session and driven from a test without touching the wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeGate {
    /// `None` means "never probed in this session". Deliberately an Option
    /// rather than a zero sentinel: unix millisecond 0 is a legitimate value
    /// on a machine whose clock has not been set yet, and treating it as "no
    /// history" would let such a machine probe without any floor at all.
    last_at_ms: Option<u64>,
}

impl ProbeGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// May we probe now for this reason?
    pub fn allows(&self, reason: ProbeReason, now_ms: u64) -> bool {
        let gap = reason.min_gap().as_millis() as u64;
        if gap == 0 {
            return true;
        }
        match self.last_at_ms {
            None => true,
            // Saturating: a clock that jumped backwards must not lock the
            // gate shut until the original time comes round again.
            Some(last) => now_ms.saturating_sub(last) >= gap,
        }
    }

    /// Record that a probe happened.
    pub fn mark(&mut self, now_ms: u64) {
        self.last_at_ms = Some(now_ms);
    }

    /// Age of the last probe, for "проверено N секунд назад".
    pub fn age_ms(&self, now_ms: u64) -> Option<u64> {
        self.last_at_ms.map(|last| now_ms.saturating_sub(last))
    }

    /// New session, no history.
    pub fn reset(&mut self) {
        self.last_at_ms = None;
    }
}

// ---------------------------------------------------------------------------
// Passive sensor: interface byte counters
// ---------------------------------------------------------------------------

/// Raw kernel counters for one interface, exactly as `if_data` holds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawCounters {
    pub rx_bytes: u32,
    pub tx_bytes: u32,
}

/// Session-cumulative counters for the tunnel device.
///
/// macOS `getifaddrs` fills `ifa_data` with `struct if_data`, whose byte
/// counters are 32-bit (SDK net/if_var.h) — they wrap every 4 GiB, which at
/// 100 Mbit/s is under six minutes. So the raw numbers are never reported;
/// they are folded into a u64 here. The fold is correct as long as we sample
/// more often than a full wrap, which the 2-second passive tick guarantees by
/// three orders of magnitude.
///
/// A new tunnel means a new device and counters that start at zero again, so
/// the session boundary is explicit: call `reset` on connect. Inside a session
/// a decrease can then only mean a wrap.
#[derive(Debug, Clone, Copy, Default)]
pub struct TunnelMeter {
    last: Option<RawCounters>,
    rx_total: u64,
    tx_total: u64,
}

impl TunnelMeter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one sample in and return the totals so far.
    pub fn observe(&mut self, sample: RawCounters) -> (u64, u64) {
        match self.last {
            None => {
                // First sample of the session is the baseline, not traffic:
                // the device may already have carried the handshake.
                self.last = Some(sample);
            }
            Some(prev) => {
                self.rx_total += u64::from(sample.rx_bytes.wrapping_sub(prev.rx_bytes));
                self.tx_total += u64::from(sample.tx_bytes.wrapping_sub(prev.tx_bytes));
                self.last = Some(sample);
            }
        }
        (self.rx_total, self.tx_total)
    }

    pub fn totals(&self) -> (u64, u64) {
        (self.rx_total, self.tx_total)
    }

    /// Start of a new tunnel session.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Read one interface's counters. `None` when the device does not exist —
/// which for the tunnel device is itself an answer.
pub fn read_counters(iface: &str) -> Option<RawCounters> {
    sys::read_counters(iface)
}

/// Counters of our tunnel device.
pub fn tunnel_counters() -> Option<RawCounters> {
    read_counters(TUN_IFACE)
}

/// Does the machine have any usable network at all?
///
/// Answered from the interface list alone: no request, no DNS, no waiting. It
/// is what separates "our service is broken" from "your Wi-Fi is off", and
/// getting that distinction wrong is one of the more expensive mistakes we
/// make in support.
pub fn has_usable_link() -> bool {
    sys::has_usable_link()
}

/// An IPv4 address that means the interface is actually configured.
///
/// 169.254/16 is what macOS assigns when DHCP never answered: the interface is
/// up, the network is not. Loopback proves nothing either.
pub fn is_usable_ipv4(o: [u8; 4]) -> bool {
    !(o[0] == 127 || (o[0] == 169 && o[1] == 254) || o == [0, 0, 0, 0])
}

/// An IPv6 address that means the same.
///
/// fe80::/10 is link-local (no router), ::1 is loopback, :: is unspecified.
pub fn is_usable_ipv6(o: [u8; 16]) -> bool {
    if o == [0u8; 16] {
        return false;
    }
    if o[..15] == [0u8; 15] && o[15] == 1 {
        return false;
    }
    !(o[0] == 0xfe && (o[1] & 0xc0) == 0x80)
}

/// Interfaces that carry tunnels rather than connect us to the world. An
/// answer built on these would be circular: the tunnel cannot prove the
/// internet exists.
fn is_tunnel_iface(name: &str) -> bool {
    name.starts_with("utun")
        || name.starts_with("ipsec")
        || name.starts_with("ppp")
        || name.starts_with("tap")
        || name.starts_with("gif")
        || name.starts_with("stf")
}

// ---------------------------------------------------------------------------
// Active sensor: one request through the tunnel
// ---------------------------------------------------------------------------

/// Extract the exit address from a ladder answer.
///
/// Every address on the ladder answers `{"ip":"..."}`, with the fuller hosts
/// adding fields we ignore. Parsing the value as a real IP is deliberate: a
/// captive portal's login page and a hosting-firewall block page both fail
/// here, which is the correct verdict for both.
pub fn parse_exit_ip(body: &str) -> Option<IpAddr> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let ip = value.get("ip")?.as_str()?;
    ip.parse::<IpAddr>().ok()
}

/// The HTTP client used for probes.
///
/// Built fresh per probe on purpose. A pooled connection would report the
/// round trip of a socket that was opened minutes ago — a number that looks
/// wonderful and says nothing about whether the tunnel works right now. A
/// probe that cannot fail is not a probe.
fn build_client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        // The tunnel is made of routes, not of a proxy. Inheriting whatever
        // http_proxy the user's shell happens to export would send the probe
        // somewhere else entirely and answer a question we did not ask.
        .no_proxy()
        .pool_max_idle_per_host(0)
        .timeout(timeout)
        .connect_timeout(timeout)
        .user_agent(concat!(
            "ProxysVPN-",
            env!("CARGO_PKG_VERSION"),
            " (probe)"
        ))
        .build()
}

/// Ask one address. `Ok(ip)` only when it answered with a real exit address.
async fn ask(client: &reqwest::Client, url: &str, timeout: Duration) -> Option<IpAddr> {
    let response = client.get(url).timeout(timeout).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    // A block page can be megabytes. Take a bounded prefix: our answer is
    // never more than a couple of hundred bytes.
    let bytes = response.bytes().await.ok()?;
    let head = &bytes[..bytes.len().min(MAX_BODY_BYTES)];
    let text = std::str::from_utf8(head).ok()?;
    parse_exit_ip(text)
}

/// One active probe: walk the ladder until something answers, then decide.
///
/// Sequential, not a race. Racing four addresses would multiply the fleet's
/// load on the shop by four for a question the first address almost always
/// answers, and would light up three domains at once on every filtered
/// network — the opposite of quiet.
///
/// `meter` is folded with a sample taken before and after, so the passive
/// counters describe exactly the window the request lived in; that is what
/// lets `classify` tell "sent and heard nothing" from "never left".
pub async fn probe_once(meter: &mut TunnelMeter) -> ProbeReport {
    probe_once_with(meter, PROBE_LADDER, ATTEMPT_TIMEOUT).await
}

/// `probe_once` with the ladder and patience spelled out, for tests and for
/// the diagnostics screen, which walks a shorter ladder on a tighter budget.
pub async fn probe_once_with(
    meter: &mut TunnelMeter,
    ladder: &[&str],
    timeout: Duration,
) -> ProbeReport {
    // Take the "before" sample first: everything after this point is traffic
    // we are responsible for.
    if let Some(sample) = tunnel_counters() {
        meter.observe(sample);
    }
    let (rx_before, tx_before) = meter.totals();

    // Cheap and decisive: if there is no network at all, do not spend four
    // timeouts finding that out, and do not blame the service for it.
    if !has_usable_link() {
        let (rx, tx) = meter.totals();
        return ProbeReport {
            verdict: ProbeVerdict::NetworkOffline,
            rtt_ms: None,
            exit_ip: None,
            via: None,
            rx_bytes: rx,
            tx_bytes: tx,
            at_ms: now_ms(),
        };
    }

    let mut exit_ip = None;
    let mut rtt_ms = None;
    let mut via = None;

    match build_client(timeout) {
        Ok(client) => {
            for (index, url) in ladder.iter().enumerate() {
                let started = Instant::now();
                if let Some(ip) = ask(&client, url, timeout).await {
                    // Never below 1 ms: a zero would read as "not measured".
                    rtt_ms = Some((started.elapsed().as_millis() as u32).max(1));
                    exit_ip = Some(ip);
                    via = Some(index);
                    break;
                }
            }
        }
        Err(_) => {
            // The client could not even be constructed (no TLS backend, bad
            // system config). Nothing left our machine, so the passive
            // counters will correctly say NoRoute below.
        }
    }

    if let Some(sample) = tunnel_counters() {
        meter.observe(sample);
    }
    let (rx_after, tx_after) = meter.totals();

    let verdict = classify(ProbeInputs {
        link_up: true,
        http_ok: exit_ip.is_some(),
        tx_delta: tx_after.saturating_sub(tx_before),
        rx_delta: rx_after.saturating_sub(rx_before),
    });

    ProbeReport {
        verdict,
        rtt_ms: if verdict == ProbeVerdict::Passed {
            rtt_ms
        } else {
            None
        },
        exit_ip,
        via,
        rx_bytes: rx_after,
        tx_bytes: tx_after,
        at_ms: now_ms(),
    }
}

// ---------------------------------------------------------------------------
// Platform layer
// ---------------------------------------------------------------------------

#[cfg(target_vendor = "apple")]
mod sys {
    use super::{is_tunnel_iface, is_usable_ipv4, is_usable_ipv6, RawCounters};
    use std::ffi::CStr;

    /// `struct IF_DATA_TIMEVAL` — `timeval32` on every 64-bit Darwin.
    #[repr(C, packed(4))]
    #[derive(Clone, Copy)]
    struct Timeval32 {
        tv_sec: i32,
        tv_usec: i32,
    }

    /// `struct if_data` from `<net/if_var.h>`, under its `#pragma pack(4)`.
    ///
    /// Declared here because libc only ships the 64-bit `if_data64` variant
    /// for Apple, and `getifaddrs` hands out the 32-bit one. Every field is
    /// spelled out even though we read two of them: a short struct would be
    /// fine today and silently wrong the moment anyone adds a field read.
    #[repr(C, packed(4))]
    #[derive(Clone, Copy)]
    struct IfData {
        ifi_type: u8,
        ifi_typelen: u8,
        ifi_physical: u8,
        ifi_addrlen: u8,
        ifi_hdrlen: u8,
        ifi_recvquota: u8,
        ifi_xmitquota: u8,
        ifi_unused1: u8,
        ifi_mtu: u32,
        ifi_metric: u32,
        ifi_baudrate: u32,
        ifi_ipackets: u32,
        ifi_ierrors: u32,
        ifi_opackets: u32,
        ifi_oerrors: u32,
        ifi_collisions: u32,
        ifi_ibytes: u32,
        ifi_obytes: u32,
        ifi_imcasts: u32,
        ifi_omcasts: u32,
        ifi_iqdrops: u32,
        ifi_noproto: u32,
        ifi_recvtiming: u32,
        ifi_xmittiming: u32,
        ifi_lastchange: Timeval32,
        ifi_unused2: u32,
        ifi_hwassist: u32,
        ifi_reserved1: u32,
        ifi_reserved2: u32,
    }

    /// Frees the list however the walk ends, including on an early return.
    struct IfAddrs(*mut libc::ifaddrs);

    impl Drop for IfAddrs {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from a successful getifaddrs and
                // is freed exactly once, here.
                unsafe { libc::freeifaddrs(self.0) };
            }
        }
    }

    fn list() -> Option<IfAddrs> {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: `head` is a valid out-pointer; the result is checked before
        // it is used, and ownership moves into the guard.
        if unsafe { libc::getifaddrs(&mut head) } != 0 || head.is_null() {
            return None;
        }
        Some(IfAddrs(head))
    }

    /// Name of an entry, or `None` when the kernel gave us none.
    ///
    /// SAFETY: caller guarantees `entry` is a live node of a list we own.
    unsafe fn name_of(entry: *const libc::ifaddrs) -> Option<&'static str> {
        let raw = (*entry).ifa_name;
        if raw.is_null() {
            return None;
        }
        CStr::from_ptr(raw).to_str().ok()
    }

    pub fn read_counters(iface: &str) -> Option<RawCounters> {
        let guard = list()?;
        let mut cur = guard.0;
        // SAFETY: the walk stays inside the list the guard owns and stops at
        // the null terminator; every dereference is of a node we were handed.
        unsafe {
            while !cur.is_null() {
                let entry = cur;
                cur = (*entry).ifa_next;

                if name_of(entry) != Some(iface) {
                    continue;
                }
                // Statistics live on the AF_LINK entry; the AF_INET entries
                // for the same device carry a null or unrelated ifa_data.
                let addr = (*entry).ifa_addr;
                if addr.is_null() || i32::from((*addr).sa_family) != libc::AF_LINK {
                    continue;
                }
                let data = (*entry).ifa_data as *const IfData;
                if data.is_null() {
                    continue;
                }
                // read_unaligned: the struct is pack(4) and the kernel gives
                // no alignment promise for ifa_data.
                let d = std::ptr::read_unaligned(data);
                return Some(RawCounters {
                    rx_bytes: d.ifi_ibytes,
                    tx_bytes: d.ifi_obytes,
                });
            }
        }
        None
    }

    pub fn has_usable_link() -> bool {
        let Some(guard) = list() else {
            // getifaddrs failing is not evidence of being offline, and
            // claiming "нет интернета" wrongly is the expensive direction of
            // this mistake. Assume there is a link and let the probe decide.
            return true;
        };
        let mut cur = guard.0;
        // SAFETY: same walk as above.
        unsafe {
            while !cur.is_null() {
                let entry = cur;
                cur = (*entry).ifa_next;

                let flags = (*entry).ifa_flags as i32;
                if flags & libc::IFF_UP == 0 || flags & libc::IFF_RUNNING == 0 {
                    continue;
                }
                if flags & libc::IFF_LOOPBACK != 0 {
                    continue;
                }
                match name_of(entry) {
                    Some(name) if is_tunnel_iface(name) => continue,
                    None => continue,
                    Some(_) => {}
                }

                let addr = (*entry).ifa_addr;
                if addr.is_null() {
                    continue;
                }
                match i32::from((*addr).sa_family) {
                    libc::AF_INET => {
                        let sin = addr as *const libc::sockaddr_in;
                        let octets = u32::from_be((*sin).sin_addr.s_addr).to_be_bytes();
                        if is_usable_ipv4(octets) {
                            return true;
                        }
                    }
                    libc::AF_INET6 => {
                        let sin6 = addr as *const libc::sockaddr_in6;
                        if is_usable_ipv6((*sin6).sin6_addr.s6_addr) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        false
    }
}

#[cfg(not(target_vendor = "apple"))]
mod sys {
    use super::RawCounters;

    /// The app ships for macOS and iOS only. This exists so the module still
    /// compiles elsewhere (CI, a Linux dev box) rather than failing to build
    /// the tests that matter.
    pub fn read_counters(_iface: &str) -> Option<RawCounters> {
        None
    }

    pub fn has_usable_link() -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(link_up: bool, http_ok: bool, tx: u64, rx: u64) -> ProbeInputs {
        ProbeInputs {
            link_up,
            http_ok,
            tx_delta: tx,
            rx_delta: rx,
        }
    }

    // ---- the four outcomes, one test each -------------------------------

    #[test]
    fn passed_when_the_page_answered() {
        let v = classify(inputs(true, true, 4096, 2048));
        assert_eq!(v, ProbeVerdict::Passed);
        assert_eq!(v.phase(), VpnPhase::On);
        assert!(v.error_code().is_none());
    }

    #[test]
    fn no_route_when_nothing_left_the_interface() {
        let v = classify(inputs(true, false, 0, 0));
        assert_eq!(v, ProbeVerdict::NoRoute);
        assert_eq!(v.error_code(), Some(ErrorCode::NoRoute));
        assert!(v.is_repairable());
    }

    #[test]
    fn blocked_when_we_sent_and_heard_nothing() {
        let v = classify(inputs(true, false, 8192, 0));
        assert_eq!(v, ProbeVerdict::Blocked);
        assert_eq!(v.error_code(), Some(ErrorCode::Blocked));
        assert_eq!(v.phase(), VpnPhase::Healing);
    }

    #[test]
    fn network_offline_outranks_everything_else() {
        // Even a successful answer cannot matter if there is no link: the
        // check runs first precisely so we never blame the service.
        let v = classify(inputs(false, true, 9999, 9999));
        assert_eq!(v, ProbeVerdict::NetworkOffline);
        assert_eq!(v.error_code(), Some(ErrorCode::NetworkOffline));
        assert!(!v.is_repairable());
    }

    #[test]
    fn unconfirmed_when_data_moves_but_our_page_is_silent() {
        let v = classify(inputs(true, false, 4096, 4096));
        assert_eq!(v, ProbeVerdict::Unconfirmed);
        assert_eq!(v.phase(), VpnPhase::Unconfirmed);
        assert_eq!(v.error_code(), Some(ErrorCode::ProbeUnconfirmed));
    }

    // ---- edges of the "did anything move" threshold ---------------------

    #[test]
    fn a_few_retransmitted_syns_are_not_traffic() {
        // Just under the threshold on both sides: still NoRoute.
        let v = classify(inputs(true, false, MOVED_BYTES - 1, MOVED_BYTES - 1));
        assert_eq!(v, ProbeVerdict::NoRoute);
    }

    #[test]
    fn exactly_the_threshold_counts_as_moved() {
        assert_eq!(
            classify(inputs(true, false, MOVED_BYTES, 0)),
            ProbeVerdict::Blocked
        );
        assert_eq!(
            classify(inputs(true, false, MOVED_BYTES, MOVED_BYTES)),
            ProbeVerdict::Unconfirmed
        );
    }

    #[test]
    fn inbound_without_outbound_is_still_no_route() {
        // Cannot happen through a healthy tunnel; if it does, we never got a
        // request out, so the honest verdict is NoRoute rather than a guess.
        assert_eq!(
            classify(inputs(true, false, 0, 100_000)),
            ProbeVerdict::NoRoute
        );
    }

    // ---- counters --------------------------------------------------------

    #[test]
    fn first_sample_is_a_baseline_not_traffic() {
        let mut m = TunnelMeter::new();
        let (rx, tx) = m.observe(RawCounters {
            rx_bytes: 5_000,
            tx_bytes: 9_000,
        });
        assert_eq!((rx, tx), (0, 0));
    }

    #[test]
    fn counters_accumulate_between_samples() {
        let mut m = TunnelMeter::new();
        m.observe(RawCounters {
            rx_bytes: 1_000,
            tx_bytes: 2_000,
        });
        let (rx, tx) = m.observe(RawCounters {
            rx_bytes: 4_000,
            tx_bytes: 2_500,
        });
        assert_eq!((rx, tx), (3_000, 500));
    }

    #[test]
    fn a_32_bit_wrap_does_not_read_as_four_gigabytes() {
        let mut m = TunnelMeter::new();
        m.observe(RawCounters {
            rx_bytes: u32::MAX - 100,
            tx_bytes: 0,
        });
        // 100 bytes to the ceiling, then 50 past it.
        let (rx, _) = m.observe(RawCounters {
            rx_bytes: 49,
            tx_bytes: 0,
        });
        assert_eq!(rx, 150);
    }

    #[test]
    fn reset_starts_a_new_session() {
        let mut m = TunnelMeter::new();
        m.observe(RawCounters {
            rx_bytes: 10,
            tx_bytes: 10,
        });
        m.observe(RawCounters {
            rx_bytes: 1_010,
            tx_bytes: 10,
        });
        assert_eq!(m.totals(), (1_000, 0));
        m.reset();
        assert_eq!(m.totals(), (0, 0));
    }

    // ---- the frequency gate ---------------------------------------------

    #[test]
    fn the_probe_that_grants_green_is_never_suppressed() {
        let mut gate = ProbeGate::new();
        gate.mark(1_000);
        assert!(gate.allows(ProbeReason::AfterConnect, 1_001));
    }

    #[test]
    fn suspicion_cannot_turn_itself_into_a_timer() {
        let mut gate = ProbeGate::new();
        gate.mark(100_000);
        // A link flapping every two seconds must not probe every two seconds.
        assert!(!gate.allows(ProbeReason::Suspicion, 102_000));
        assert!(!gate.allows(ProbeReason::Suspicion, 129_999));
        assert!(gate.allows(ProbeReason::Suspicion, 130_000));
    }

    #[test]
    fn idle_in_the_background_asks_four_times_less_often() {
        let mut gate = ProbeGate::new();
        gate.mark(0);
        assert!(gate.allows(ProbeReason::IdleForeground, 300_000));
        assert!(!gate.allows(ProbeReason::IdleBackground, 300_000));
        assert!(gate.allows(ProbeReason::IdleBackground, 900_000));
    }

    #[test]
    fn a_backwards_clock_does_not_lock_the_gate() {
        let mut gate = ProbeGate::new();
        gate.mark(10_000_000);
        // NTP stepped the clock back; saturating_sub yields 0, which is short
        // of the gap, so we wait rather than storm — and we never overflow.
        assert!(!gate.allows(ProbeReason::Suspicion, 5_000_000));
        assert!(gate.allows(ProbeReason::AfterConnect, 5_000_000));
    }

    #[test]
    fn first_probe_of_a_session_is_always_allowed() {
        let gate = ProbeGate::new();
        assert!(gate.allows(ProbeReason::IdleBackground, 0));
        assert!(gate.age_ms(1_000).is_none());
    }

    // ---- body parsing ----------------------------------------------------

    #[test]
    fn shop_answer_parses() {
        let ip = parse_exit_ip(r#"{"ip":"203.0.113.9"}"#);
        assert_eq!(ip, Some("203.0.113.9".parse::<IpAddr>().expect("literal")));
    }

    #[test]
    fn fallback_answer_with_extra_fields_parses() {
        let body = r#"{"ip":"2001:db8::1","family":"IPv6","node":null,"city":null}"#;
        assert_eq!(
            parse_exit_ip(body),
            Some("2001:db8::1".parse::<IpAddr>().expect("literal"))
        );
    }

    #[test]
    fn a_captive_portal_login_page_is_not_an_answer() {
        assert!(parse_exit_ip("<!DOCTYPE html><html>Please sign in</html>").is_none());
    }

    #[test]
    fn json_without_a_real_address_is_not_an_answer() {
        assert!(parse_exit_ip(r#"{"ip":"not-an-address"}"#).is_none());
        assert!(parse_exit_ip(r#"{"error":"blocked"}"#).is_none());
        assert!(parse_exit_ip(r#"{"ip":null}"#).is_none());
    }

    // ---- what crosses to the window --------------------------------------

    #[test]
    fn only_a_pass_refreshes_the_age_of_the_last_proof() {
        let failed = ProbeReport {
            verdict: ProbeVerdict::Blocked,
            rtt_ms: None,
            exit_ip: None,
            via: None,
            rx_bytes: 10,
            tx_bytes: 9_000,
            at_ms: 1_700_000_000_000,
        };
        assert!(failed.metric().last_proof_at.is_none());

        let ok = ProbeReport {
            verdict: ProbeVerdict::Passed,
            rtt_ms: Some(38),
            exit_ip: None,
            via: Some(0),
            rx_bytes: 4_000,
            tx_bytes: 9_000,
            at_ms: 1_700_000_000_000,
        };
        let m = ok.metric();
        assert_eq!(m.last_proof_at, Some(1_700_000_000_000));
        assert_eq!(m.quality, Some(LinkQuality::Good));
    }

    #[test]
    fn the_exit_address_never_reaches_the_window() {
        // MetricPayload is the only thing that travels, and it has no field
        // that could carry a node address. This is the test that fails if
        // someone adds one.
        let report = ProbeReport {
            verdict: ProbeVerdict::Passed,
            rtt_ms: Some(40),
            exit_ip: Some("198.51.100.7".parse::<IpAddr>().expect("literal")),
            via: Some(0),
            rx_bytes: 1,
            tx_bytes: 1,
            at_ms: 1,
        };
        let json = serde_json::to_string(&report.metric()).expect("metric serialises");
        assert!(!json.contains("198.51.100.7"), "{json}");
    }

    // ---- link usability --------------------------------------------------

    #[test]
    fn dhcp_never_answered_is_not_a_usable_link() {
        assert!(!is_usable_ipv4([169, 254, 12, 8]));
        assert!(!is_usable_ipv4([127, 0, 0, 1]));
        assert!(!is_usable_ipv4([0, 0, 0, 0]));
        assert!(is_usable_ipv4([192, 168, 1, 42]));
        assert!(is_usable_ipv4([10, 0, 0, 3]));
    }

    #[test]
    fn link_local_ipv6_is_not_a_usable_link() {
        let mut fe80 = [0u8; 16];
        fe80[0] = 0xfe;
        fe80[1] = 0x80;
        fe80[15] = 1;
        assert!(!is_usable_ipv6(fe80));

        let mut loopback = [0u8; 16];
        loopback[15] = 1;
        assert!(!is_usable_ipv6(loopback));
        assert!(!is_usable_ipv6([0u8; 16]));

        let mut global = [0u8; 16];
        global[0] = 0x20;
        global[1] = 0x01;
        global[15] = 9;
        assert!(is_usable_ipv6(global));
    }

    #[test]
    fn tunnels_do_not_count_as_proof_of_internet() {
        assert!(is_tunnel_iface("utun225"));
        assert!(is_tunnel_iface("ipsec0"));
        assert!(is_tunnel_iface("ppp0"));
        assert!(!is_tunnel_iface("en0"));
        assert!(!is_tunnel_iface("pdp_ip0"));
    }

    // ---- the ladder itself -----------------------------------------------

    #[test]
    fn the_ladder_has_real_depth_and_avoids_the_direct_domain() {
        assert!(
            PROBE_LADDER.len() >= 3,
            "a ladder shorter than three is not a ladder"
        );
        for url in PROBE_LADDER {
            assert!(url.starts_with("https://"), "{url}");
            // proxysvpn.com is pinned to the DIRECT list, so a probe to it
            // bypasses the tunnel and proves nothing.
            assert!(
                !url.contains("://proxysvpn.com") && !url.contains(".proxysvpn.com"),
                "{url} would not go through the tunnel"
            );
        }
        assert!(
            PROBE_LADDER
                .iter()
                .any(|u| u.contains("proksya.xyz")),
            "the Russian fallback domain has to be on the ladder"
        );
    }

    #[tokio::test]
    async fn a_ladder_of_dead_addresses_ends_in_a_verdict_not_a_hang() {
        // 203.0.113.0/24 is TEST-NET-3: guaranteed unroutable, so this never
        // touches a real service. The point is that the walk terminates with
        // one of our five answers inside the budget.
        let mut meter = TunnelMeter::new();
        let report = probe_once_with(
            &mut meter,
            &["https://203.0.113.1/api/exit-ip"],
            Duration::from_millis(200),
        )
        .await;
        assert!(report.exit_ip.is_none());
        assert!(report.rtt_ms.is_none());
        assert!(matches!(
            report.verdict,
            ProbeVerdict::NoRoute | ProbeVerdict::Blocked | ProbeVerdict::NetworkOffline
        ));
    }
}
