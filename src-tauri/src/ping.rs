// src-tauri/src/ping.rs
//
// Latency to the node, measured ON DEMAND and never again as an indicator.
//
// What this file used to be, and why it is not that any more.
//
// It opened three TCP connections to the node every five seconds
// (App.tsx:30,177 drove it) and took the fastest. That is 36 connections a
// minute from one device; across ~1500 paying devices, roughly 54 000 a
// minute against our own nodes, to render a number that never told anyone
// anything actionable. Three separate problems came out of it:
//
//   1. It was the WRONG SENSOR. A node can answer a TCP handshake instantly
//      while the tunnel carries nothing — that is exactly what DPI throttling
//      looks like. "Connected, 38 ms, no internet" was a real, common, and
//      completely unactionable screen. Liveness now comes from probe.rs,
//      which asks whether a byte actually crossed.
//   2. It COULD NOT WORK on Hysteria2 at all. hy2 is pure UDP; a TCP SYN to
//      its port is simply never answered, every probe timed out, and the
//      Netherlands node sat on "измерение…" forever. There was no way for the
//      window to tell that apart from "still measuring".
//   3. The numbers were not even comparable to each other. On macOS the
//      measurement goes around the tunnel, on iOS through it.
//
// What survives: a single honest measurement, taken when a person asks for
// one — the "Проверка" screen and the server list. Numbers on screen come
// from probe.rs's round trip through the tunnel, which is comparable with
// itself; this one is a diagnostic.

use anyhow::{anyhow, Context, Result};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::errors::{AppError, ErrorCode};
use crate::events::LinkQuality;

/// One attempt's patience.
///
/// 1500 ms rather than the old 2000: this is now a foreground action with a
/// person waiting, and a node that has not completed a TCP handshake in a
/// second and a half is not going to produce a number worth showing. The
/// whole measurement is therefore bounded by this, not by three times it.
const TIMEOUT: Duration = Duration::from_millis(1500);

/// How long a measurement stays good enough to reuse.
///
/// The file no longer polls, but nothing stops a caller from calling in a
/// loop — that is how the old behaviour arrived in the first place. With this
/// cache the worst case any single caller can produce is two connections a
/// minute per device instead of thirty-six, and the number a person sees does
/// not flicker between two redraws of the same screen.
const CACHE_TTL: Duration = Duration::from_secs(30);

/// Consecutive silent timeouts before we stop trying.
///
/// "Silent" means the SYN drew no answer at all — neither a handshake nor a
/// refusal. A port that swallows TCP is either pure UDP (hysteria) or being
/// filtered; either way the honest answer is "this cannot be measured", and
/// the one thing we must not do is keep the window spinning. Two rather than
/// one so a single lost packet on a bad mobile link does not latch it.
const SILENT_ATTEMPTS_TO_GIVE_UP: u8 = 2;

// ---------------------------------------------------------------------------
// What the caller gets
// ---------------------------------------------------------------------------

/// Result of one measurement.
///
/// Four outcomes rather than `Result<u32>` because three different things
/// were previously collapsed into one error, and the window could only render
/// them as "измерение…".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum PingOutcome {
    /// A handshake completed. The only outcome carrying a number.
    ///
    /// The field is renamed explicitly: `rename_all` on an enum renames the
    /// VARIANTS, not the fields inside them, so without this the window would
    /// silently receive `rtt_ms` and read `rttMs` as undefined.
    Measured {
        #[serde(rename = "rttMs")]
        rtt_ms: u32,
    },
    /// This endpoint has no latency figure and never will: pure UDP, or a
    /// port that silently swallows TCP. Say "не применимо", not "измеряю".
    NotApplicable,
    /// Reachable enough to answer, but not with a handshake — refused, or the
    /// route is gone.
    Unreachable,
    /// Nothing is connected, so there is nothing to measure.
    NoTarget,
}

impl PingOutcome {
    /// Quality in words, for the server list and the details sheet.
    pub fn quality(self) -> LinkQuality {
        match self {
            Self::Measured { rtt_ms } => LinkQuality::from_rtt_ms(rtt_ms),
            // The distinction the old code could not make.
            Self::NotApplicable => LinkQuality::NotApplicable,
            Self::Unreachable | Self::NoTarget => LinkQuality::Unknown,
        }
    }

    pub fn rtt_ms(self) -> Option<u32> {
        match self {
            Self::Measured { rtt_ms } => Some(rtt_ms),
            _ => None,
        }
    }
}

/// What crosses to the window. Mirrored by `PingPayload` in src/types.ts.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingPayload {
    #[serde(flatten)]
    pub outcome: PingOutcome,
    pub quality: LinkQuality,
    /// True when this answer came from the cache and no socket was opened.
    pub cached: bool,
}

impl PingPayload {
    fn new(outcome: PingOutcome, cached: bool) -> Self {
        Self {
            outcome,
            quality: outcome.quality(),
            cached,
        }
    }
}

// ---------------------------------------------------------------------------
// The target
// ---------------------------------------------------------------------------

/// Whether this endpoint can answer a TCP handshake at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// VLESS / Trojan / anything over TCP: measurable.
    Tcp,
    /// Hysteria2 and friends: pure UDP, not measurable this way, ever.
    Udp,
    /// Nobody told us. We find out by trying, once.
    Unknown,
}

/// Map the protocol string the subscription used onto what we can measure.
///
/// The names come from `ServerConfig::proto()`; anything unrecognised stays
/// `Unknown` rather than being guessed as TCP, because guessing wrong in that
/// direction is what produced the endless spinner.
pub fn kind_of_proto(proto: &str) -> TargetKind {
    match proto.trim().to_ascii_lowercase().as_str() {
        "vless" | "vmess" | "trojan" | "ss" | "shadowsocks" => TargetKind::Tcp,
        "hy2" | "hysteria" | "hysteria2" | "tuic" | "wireguard" => TargetKind::Udp,
        _ => TargetKind::Unknown,
    }
}

struct PingTarget {
    host: String,
    port: u16,
    kind: TargetKind,
    /// Cached resolved address — no DNS lookup per measurement.
    addr: Option<SocketAddr>,
    /// Last answer and when it was taken.
    cached: Option<(PingOutcome, Instant)>,
    /// Consecutive attempts where the SYN drew no answer at all.
    silent: u8,
}

impl PingTarget {
    fn new(host: String, port: u16, kind: TargetKind) -> Self {
        Self {
            host,
            port,
            kind,
            addr: None,
            cached: None,
            silent: 0,
        }
    }

    /// Have we learned that this endpoint cannot be measured?
    fn latched_not_applicable(&self) -> bool {
        self.kind == TargetKind::Udp || self.silent >= SILENT_ATTEMPTS_TO_GIVE_UP
    }
}

static TARGET: Mutex<Option<PingTarget>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<PingTarget>> {
    match TARGET.lock() {
        Ok(g) => g,
        // Nothing in here can be left half-written; a poisoned lock must not
        // take the measurement feature down with it.
        Err(p) => p.into_inner(),
    }
}

/// Point the measurement at a node whose protocol we do not know.
///
/// Kept for the existing call sites in lib.rs. Prefer `set_target_proto`:
/// without the protocol the first Hysteria2 measurement has to spend two
/// timeouts discovering what `server.proto()` already knew.
pub fn set_target(host: String, port: u16) {
    set_target_proto(host, port, "");
}

/// Point the measurement at a node, naming its protocol.
///
/// `proto` is `ServerConfig::proto()` — "vless" or "hy2".
pub fn set_target_proto(host: String, port: u16, proto: &str) {
    *lock() = Some(PingTarget::new(host, port, kind_of_proto(proto)));
}

pub fn clear_target() {
    *lock() = None;
}

/// What we currently know about the target, without measuring anything.
pub fn target_kind() -> Option<TargetKind> {
    lock().as_ref().map(|t| t.kind)
}

// ---------------------------------------------------------------------------
// Measuring
// ---------------------------------------------------------------------------

fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    format!("{host}:{port}")
        .to_socket_addrs()
        .with_context(|| format!("resolve {host}:{port}"))?
        .next()
        .ok_or_else(|| anyhow!("no addresses for {host}:{port}"))
}

/// Outcome of a single connect attempt, kept apart from a plain error so the
/// caller can tell "no answer" from "answered, refused".
enum Attempt {
    Ok(u32),
    /// Refused, unreachable, or DNS failed: the endpoint spoke, just not the
    /// way we wanted.
    Refused,
    /// No answer at all within the budget.
    Silent,
}

fn one_attempt(addr: SocketAddr) -> Attempt {
    let start = Instant::now();
    match TcpStream::connect_timeout(&addr, TIMEOUT) {
        Ok(stream) => {
            let elapsed = start.elapsed();
            // Close immediately: we wanted the handshake, not a session.
            drop(stream);
            // Never report 0 — a zero reads as "no measurement".
            Attempt::Ok((elapsed.as_millis() as u32).max(1))
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::TimedOut => Attempt::Silent,
            // macOS surfaces a connect timeout as WouldBlock on some paths.
            std::io::ErrorKind::WouldBlock => Attempt::Silent,
            _ => Attempt::Refused,
        },
    }
}

/// Measure now, or answer from what we already know.
///
/// One connection, not three. The old three-sample minimum existed to smooth
/// a number that was redrawn twice a second; a number a person asks for once
/// does not need smoothing, and the quality buckets it feeds are 150 and
/// 400 ms wide — far wider than the jitter two extra samples would remove.
pub fn measure() -> PingPayload {
    let (addr, kind) = {
        let mut guard = lock();
        let Some(target) = guard.as_mut() else {
            return PingPayload::new(PingOutcome::NoTarget, false);
        };

        if target.latched_not_applicable() {
            // Zero sockets: this is the answer that used to be an endless
            // spinner.
            return PingPayload::new(PingOutcome::NotApplicable, true);
        }

        if let Some((outcome, at)) = target.cached {
            if at.elapsed() < CACHE_TTL {
                return PingPayload::new(outcome, true);
            }
        }

        let addr = match target.addr {
            Some(a) => a,
            None => match resolve(&target.host, target.port) {
                Ok(a) => {
                    target.addr = Some(a);
                    a
                }
                Err(_) => {
                    // A node name that does not resolve is not an
                    // unmeasurable protocol — it is a broken route, and the
                    // repair ladder, not this file, decides what to do.
                    let outcome = PingOutcome::Unreachable;
                    target.cached = Some((outcome, Instant::now()));
                    return PingPayload::new(outcome, false);
                }
            },
        };
        (addr, target.kind)
    };

    // The lock is NOT held across the connect: a 1.5 s blocking syscall under
    // a global mutex would stall every other caller, including `clear_target`
    // on disconnect.
    let attempt = one_attempt(addr);

    let mut guard = lock();
    let Some(target) = guard.as_mut() else {
        // Disconnected while we were measuring. The answer is stale by
        // definition; do not cache it against a target that no longer exists.
        return PingPayload::new(PingOutcome::NoTarget, false);
    };
    // Same check after the fact: a reconnect may have replaced the target.
    if target.addr != Some(addr) && target.addr.is_some() {
        return PingPayload::new(PingOutcome::NoTarget, false);
    }

    let outcome = match attempt {
        Attempt::Ok(rtt_ms) => {
            target.silent = 0;
            PingOutcome::Measured { rtt_ms }
        }
        Attempt::Refused => {
            target.silent = 0;
            PingOutcome::Unreachable
        }
        Attempt::Silent => {
            target.silent = target.silent.saturating_add(1);
            if target.silent >= SILENT_ATTEMPTS_TO_GIVE_UP {
                // Learned the hard way what `set_target_proto` would have
                // told us for free.
                let _ = kind;
                PingOutcome::NotApplicable
            } else {
                PingOutcome::Unreachable
            }
        }
    };
    target.cached = Some((outcome, Instant::now()));
    PingPayload::new(outcome, false)
}

/// `measure` off the async runtime's worker threads.
pub async fn measure_async() -> PingPayload {
    tokio::task::spawn_blocking(measure)
        .await
        // A panic inside a blocking task must not take the command down; the
        // honest answer is that we have no measurement.
        .unwrap_or_else(|_| PingPayload::new(PingOutcome::NoTarget, false))
}

// ---------------------------------------------------------------------------
// Compatibility shim for the existing `vpn_ping` command
// ---------------------------------------------------------------------------

/// The shape lib.rs:276 still calls.
///
/// The error side carries an `AppError` payload rather than a sentence, so
/// `parseAppError` on the window side gets a code it can render — in
/// particular `PING_NOT_APPLICABLE`, whose action is to offer nothing.
/// Replace the call site with `measure_async` when lib.rs is next touched:
/// this shim throws away `cached`, and the window has no way to tell a fresh
/// measurement from a reused one.
pub fn tcp_ping() -> Result<u32> {
    match measure().outcome {
        PingOutcome::Measured { rtt_ms } => Ok(rtt_ms),
        PingOutcome::NotApplicable => {
            Err(AppError::new(ErrorCode::PingNotApplicable).into())
        }
        PingOutcome::Unreachable => Err(AppError::new(ErrorCode::NoRoute).into()),
        PingOutcome::NoTarget => Err(AppError::new(ErrorCode::NoSubscription).into()),
    }
}

pub async fn tcp_ping_async() -> Result<u32> {
    match measure_async().await.outcome {
        PingOutcome::Measured { rtt_ms } => Ok(rtt_ms),
        PingOutcome::NotApplicable => {
            Err(AppError::new(ErrorCode::PingNotApplicable).into())
        }
        PingOutcome::Unreachable => Err(AppError::new(ErrorCode::NoRoute).into()),
        PingOutcome::NoTarget => Err(AppError::new(ErrorCode::NoSubscription).into()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// The global target is shared, so the tests that touch it run one at a
    /// time. Cheaper and clearer than threading state through every function
    /// for a module the app has exactly one of.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        match SERIAL.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    // ---- protocol knowledge ---------------------------------------------

    #[test]
    fn hysteria_is_known_to_be_unmeasurable_by_tcp() {
        assert_eq!(kind_of_proto("hy2"), TargetKind::Udp);
        assert_eq!(kind_of_proto("Hysteria2"), TargetKind::Udp);
        assert_eq!(kind_of_proto("vless"), TargetKind::Tcp);
        // An unknown name is NOT optimistically called TCP: guessing that way
        // is what produced the endless spinner.
        assert_eq!(kind_of_proto("something-new"), TargetKind::Unknown);
        assert_eq!(kind_of_proto(""), TargetKind::Unknown);
    }

    #[test]
    fn a_hy2_target_answers_immediately_without_opening_a_socket() {
        let _g = serial();
        // 203.0.113.0/24 is TEST-NET-3 and unroutable: if this opened a
        // socket the test would take the full timeout instead of no time.
        set_target_proto("203.0.113.9".to_string(), 443, "hy2");
        let started = Instant::now();
        let answer = measure();
        assert_eq!(answer.outcome, PingOutcome::NotApplicable);
        assert_eq!(answer.quality, LinkQuality::NotApplicable);
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "a UDP target must not be dialled at all"
        );
        clear_target();
    }

    #[test]
    fn no_target_is_its_own_answer() {
        let _g = serial();
        clear_target();
        let answer = measure();
        assert_eq!(answer.outcome, PingOutcome::NoTarget);
        assert_eq!(answer.quality, LinkQuality::Unknown);
        assert!(target_kind().is_none());
    }

    // ---- measuring -------------------------------------------------------

    #[test]
    fn a_listening_port_is_measured_and_then_cached() {
        let _g = serial();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        set_target_proto("127.0.0.1".to_string(), port, "vless");

        let first = measure();
        match first.outcome {
            PingOutcome::Measured { rtt_ms } => {
                assert!(rtt_ms >= 1, "a measurement of 0 reads as 'not measured'");
                assert_eq!(first.quality, LinkQuality::Good);
            }
            other => panic!("expected a measurement, got {other:?}"),
        }
        assert!(!first.cached);

        // The second call inside the TTL must not open another socket.
        let second = measure();
        assert!(second.cached, "a repeated call opened a new connection");
        assert_eq!(second.outcome, first.outcome);

        clear_target();
        drop(listener);
    }

    #[test]
    fn a_closed_port_is_unreachable_not_unmeasurable() {
        let _g = serial();
        // Bind then drop: the port is almost certainly free, so the kernel
        // answers with a refusal rather than silence.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").expect("bind");
            let p = l.local_addr().expect("addr").port();
            drop(l);
            p
        };
        set_target_proto("127.0.0.1".to_string(), port, "vless");
        let answer = measure();
        assert_eq!(
            answer.outcome,
            PingOutcome::Unreachable,
            "a refused connection means the host is there and the port is not"
        );
        assert_eq!(answer.quality, LinkQuality::Unknown);
        clear_target();
    }

    #[test]
    fn a_name_that_does_not_resolve_is_unreachable() {
        let _g = serial();
        set_target_proto(
            "no-such-host.invalid-tld-for-tests".to_string(),
            443,
            "vless",
        );
        assert_eq!(measure().outcome, PingOutcome::Unreachable);
        clear_target();
    }

    // ---- the "измерение…" bug -------------------------------------------

    #[test]
    fn a_silent_port_latches_to_not_applicable_instead_of_spinning() {
        let _g = serial();
        // Simulate what an unknown-protocol hy2 endpoint does to us, without
        // waiting two real timeouts: drive the latch directly.
        set_target_proto("198.51.100.4".to_string(), 443, "");
        {
            let mut guard = lock();
            let t = guard.as_mut().expect("target set above");
            assert_eq!(t.kind, TargetKind::Unknown);
            assert!(!t.latched_not_applicable());
            t.silent = SILENT_ATTEMPTS_TO_GIVE_UP - 1;
            assert!(
                !t.latched_not_applicable(),
                "one lost packet must not latch"
            );
            t.silent = SILENT_ATTEMPTS_TO_GIVE_UP;
            assert!(t.latched_not_applicable());
        }
        let answer = measure();
        assert_eq!(answer.outcome, PingOutcome::NotApplicable);
        assert!(answer.cached, "a latched target must not be dialled again");
        clear_target();
    }

    #[test]
    fn a_new_target_forgets_what_the_old_one_taught_us() {
        let _g = serial();
        set_target_proto("203.0.113.9".to_string(), 443, "hy2");
        assert_eq!(measure().outcome, PingOutcome::NotApplicable);
        // Reconnecting to a VLESS node must be measurable again.
        set_target_proto("127.0.0.1".to_string(), 1, "vless");
        assert_eq!(target_kind(), Some(TargetKind::Tcp));
        {
            let guard = lock();
            let t = guard.as_ref().expect("target");
            assert_eq!(t.silent, 0);
            assert!(t.cached.is_none());
        }
        clear_target();
    }

    #[test]
    fn the_legacy_setter_still_works_and_stays_honest() {
        let _g = serial();
        set_target("198.51.100.4".to_string(), 443);
        assert_eq!(
            target_kind(),
            Some(TargetKind::Unknown),
            "without a protocol we must not assume TCP"
        );
        clear_target();
    }

    // ---- what the window receives ---------------------------------------

    #[test]
    fn quality_words_follow_the_number() {
        assert_eq!(
            PingOutcome::Measured { rtt_ms: 38 }.quality(),
            LinkQuality::Good
        );
        assert_eq!(
            PingOutcome::Measured { rtt_ms: 220 }.quality(),
            LinkQuality::Ok
        );
        assert_eq!(
            PingOutcome::Measured { rtt_ms: 900 }.quality(),
            LinkQuality::Poor
        );
        assert_eq!(PingOutcome::NotApplicable.quality(), LinkQuality::NotApplicable);
        assert_eq!(PingOutcome::Unreachable.quality(), LinkQuality::Unknown);
    }

    #[test]
    fn the_payload_matches_the_typescript_mirror() {
        let json = serde_json::to_string(&PingPayload::new(
            PingOutcome::Measured { rtt_ms: 38 },
            false,
        ))
        .expect("serialises");
        assert!(json.contains(r#""outcome":"measured""#), "{json}");
        assert!(json.contains(r#""rttMs":38"#), "{json}");
        assert!(json.contains(r#""quality":"good""#), "{json}");
        assert!(json.contains(r#""cached":false"#), "{json}");

        let json = serde_json::to_string(&PingPayload::new(PingOutcome::NotApplicable, true))
            .expect("serialises");
        assert!(json.contains(r#""outcome":"notApplicable""#), "{json}");
        assert!(!json.contains("rttMs"), "{json}");
    }

    #[test]
    fn no_node_address_can_ride_out_in_the_payload() {
        let json = serde_json::to_string(&PingPayload::new(
            PingOutcome::Measured { rtt_ms: 38 },
            false,
        ))
        .expect("serialises");
        assert!(!json.contains("203.0.113"), "{json}");
        assert!(!json.contains("host"), "{json}");
    }

    // ---- the compatibility shim -----------------------------------------

    #[test]
    fn the_shim_rejects_with_a_code_not_a_sentence() {
        let _g = serial();
        set_target_proto("203.0.113.9".to_string(), 443, "hy2");
        let err = tcp_ping().expect_err("hy2 cannot be measured");
        // Exactly what parseAppError in src/types.ts expects to receive.
        assert_eq!(err.to_string(), r#"{"code":"PING_NOT_APPLICABLE"}"#);
        clear_target();

        let err = tcp_ping().expect_err("no target");
        assert_eq!(err.to_string(), r#"{"code":"NO_SUBSCRIPTION"}"#);
    }

    #[test]
    fn the_shim_still_returns_a_plain_number_on_success() {
        let _g = serial();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        set_target_proto("127.0.0.1".to_string(), port, "vless");
        let ms = tcp_ping().expect("loopback is measurable");
        assert!(ms >= 1);
        clear_target();
        drop(listener);
    }

    // ---- the load this file used to create -------------------------------

    #[test]
    fn a_caller_in_a_loop_cannot_recreate_the_old_connection_storm() {
        let _g = serial();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        set_target_proto("127.0.0.1".to_string(), port, "vless");

        // The old UI called this every 5 s and each call opened 3 sockets.
        let mut fresh = 0;
        for _ in 0..12 {
            if !measure().cached {
                fresh += 1;
            }
        }
        assert_eq!(
            fresh, 1,
            "twelve calls inside the TTL must produce one connection"
        );
        clear_target();
        drop(listener);
    }
}
