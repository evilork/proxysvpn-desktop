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
///
/// ПОРЯДОК ПРОВЕРЕН С САМИХ УЗЛОВ, а не выведен из общих соображений. Замер
/// 22.09.2026 с немецкого и амстердамского выходов:
///
///   proxysvpn.store            DE 0,17 с   AMS 0,23 с
///   proksya.com                DE 0,29 с   AMS 0,20 с
///   proxysvnovich.vercel.app   DE 0,14 с   AMS 0,16 с
///   proksya.xyz                DE МОЛЧИТ   AMS 0,63 с
///
/// `proksya.xyz` стоял ВТОРЫМ и потому попадал в укороченную лестницу первой
/// пробы - а с немецкого выхода он не отвечает вовсе. Каждое подключение на
/// Германии дарило ему весь бюджет впустую. Он не удалён: это наш российский
/// запасной домен, и он незаменим, когда у человека режут зарубежные
/// префиксы. Но в паре для первой пробы ему не место - там нужны двое,
/// отвечающих с НАШИХ выходов, потому что запрос идёт уже из-за границы.
/// Лестница ПЕРВОЙ пробы - той, что решает, когда щит станет зелёным.
///
/// Два обращения к ОДНОМУ адресу, наперегонки: по открытому порту и по
/// защищённому.
///
/// Зачем открытый. Защита Vercel периодически держит рукопожатие TLS 3-4
/// секунды и только потом отдаёт 403 (замер 22.09.2026). Ответ этот годный -
/// он доказывает, что байты сходили через туннель и вернулись, - но ждать его
/// четыре секунды человек не должен. По порту 80 тот же адрес отвечает
/// перенаправлением за 0,2 секунды, и это ровно то же доказательство.
///
/// Зачем защищённый рядом. Он единственный отдаёт РАЗОБРАННЫЙ выходной адрес.
/// Успеет первым - хорошо, будет и адрес; не успеет - щит всё равно зелёный.
///
/// Адрес один и тот же нарочно: открытый запрос не показывает наблюдателю
/// между узлом и витриной ничего сверх того, что и так видно по имени в
/// рукопожатии TLS. И ни байта пользовательских данных - это HEAD.
pub const FIRST_PROBE_LADDER: &[&str] = &[
    "http://proxysvpn.store/api/exit-ip",
    "https://proxysvpn.store/api/exit-ip",
];

pub const PROBE_LADDER: &[&str] = &[
    "https://proxysvpn.store/api/exit-ip",
    "https://proksya.com/api/tools/whoami",
    "https://proxysvnovich.vercel.app/api/tools/whoami",
    "https://proksya.xyz/api/tools/whoami",
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
    /// Туннель только что встал, первая проба его не подтвердила, и мы даём
    /// ему ещё несколько попыток подряд.
    ///
    /// Зачем отдельная причина. `AfterConnect` срабатывает на третьей секунде
    /// жизни туннеля, когда ни одного байта через него ещё не прошло: xray не
    /// успел договориться с узлом, имя не разрешено, сессия TLS не начата.
    /// Замер 22.09.2026 на живой машине: первый запрос через холодный xray
    /// стоил 3,7 секунды при бюджете 2,5 - проба падала в таймаут на исправном
    /// туннеле. Дальше действовал `IdleForeground` с полом в 300 секунд, и
    /// окно пять минут показывало «Подтвердить не удалось» поверх работающей
    /// защиты. Ради одной холодной секунды человек видел сломанный VPN.
    Settling,
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
            // Прогрев. Пол маленький нарочно: эти попытки идут в первые
            // полминуты жизни туннеля и их считанные штуки, метронома из них
            // не выйдет. Ритма тоже: они прекращаются, как только туннель
            // подтвердился.
            Self::Settling => Duration::from_secs(4),
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
/// Адреса ступеней лестницы, снятые ДО подъёма туннеля.
///
/// Зачем это вообще. Подтверждено системным журналом macOS 22.09.2026:
///
///   14:24:18.901  прогрев спросил proxysvpn.store  -> ответ, duration: 0s
///   14:24:19.661  проба спросила ТО ЖЕ имя         -> DNS service 1939
///   14:24:21.243  -> переназначено на DNS service 1349
///   14:24:32.614  -> ответ.  duration: 13s
///
/// Как только поднимается туннель, macOS перетасовывает свои DNS-службы, и
/// запрос, попавший в этот момент, виснет на тринадцать секунд. Прогрев за
/// секунду до этого успевал проскочить.
///
/// А reqwest заворачивает разрешение имени И установку соединения в ОДИН
/// таймаут (reqwest-0.12.28/src/connect.rs, with_timeout вокруг всего
/// connect_with_maybe_proxy). Поэтому ошибка выходила одновременно
/// `is_connect()` и `is_timeout()`, и наша `why()` подписывала её «не
/// соединился» - про сокет, которого не было: ни одного SYN не отправлялось.
/// На этой ложной подписи был потерян вечер.
///
/// Лечение: снять адреса ДО подъёма туннеля, пока резолвер спокоен, и дальше
/// ходить по ним. Тогда шестисекундный бюджет меряет то, ради чего заведён, -
/// рукопожатие и ответ.
static PINNED: std::sync::Mutex<Vec<(String, IpAddr)>> = std::sync::Mutex::new(Vec::new());

fn pinned() -> Vec<(String, IpAddr)> {
    match PINNED.lock() {
        Ok(g) => g.clone(),
        // Отравленный замок не должен утаскивать за собой проверку связи.
        Err(p) => p.into_inner().clone(),
    }
}

/// Снять адреса всех ступеней. Вызывать ДО подъёма туннеля.
///
/// Каждое имя разрешается отдельной задачей: на спокойном резолвере это
/// десятки миллисекунд, и складывать их в очередь незачем. Неудача по
/// отдельному имени не страшна - для него просто останется обычный путь.
pub async fn pin_ladder_addresses() {
    let mut hosts: Vec<&'static str> = FIRST_PROBE_LADDER
        .iter()
        .chain(PROBE_LADDER.iter())
        .map(|url| host_of(url))
        .collect();
    hosts.sort_unstable();
    hosts.dedup();

    let mut jobs = tokio::task::JoinSet::new();
    for host in hosts {
        jobs.spawn(async move {
            let found = tokio::net::lookup_host((host, 0u16))
                .await
                .ok()
                .and_then(|mut it| it.find(|a| a.is_ipv4()))
                .map(|a| a.ip());
            (host.to_string(), found)
        });
    }
    let mut out = Vec::new();
    while let Some(done) = jobs.join_next().await {
        if let Ok((host, Some(ip))) = done {
            out.push((host, ip));
        }
    }
    let count = out.len();
    if let Ok(mut g) = PINNED.lock() {
        *g = out;
    }
    crate::logger::log("info", "probe", &format!("адреса лестницы сняты заранее: {count}"));
}

/// Забыть снятые адреса: подписка сменилась или туннель опущен.
pub fn forget_pinned_addresses() {
    if let Ok(mut g) = PINNED.lock() {
        g.clear();
    }
}

fn build_client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    // Бюджет на УСТАНОВКУ соединения - треть общего, но не меньше полутора
    // секунд.
    //
    // Раньше он был равен общему, и это стоило нам вечера вслепую: любая
    // остановка - на разрешении имени, на рукопожатии TCP, на TLS или уже
    // после запроса - приходила одной и той же ошибкой `is_timeout()`, и в
    // журнале печаталось одинаковое «таймаут». Отличить «не дошли до сервера»
    // от «дошли и он молчит» было нечем.
    //
    // С раздельным бюджетом неудача ДО установки приходит как `is_connect()`
    // и печатается «не соединился». Это ровно тот вопрос, на который надо
    // ответить, когда туннель несёт трафик, а проба падает.
    // Три четверти, а не треть.
    //
    // Треть я поставил ради диагностики и тут же отрезал себе ответ: защита
    // Vercel держит рукопожатие TLS около 3,2 с и только потом отдаёт 403, а
    // этот 403 теперь считается доказательством туннеля. Бюджет в 2,7 с рвал
    // связь ровно перед ним, и в журнале появлялось «не соединился» там, где
    // на самом деле всё дошло бы. Замер 22.09.2026: TCP-соединение 3 мс,
    // рукопожатие TLS 3,0-3,9 с.
    //
    // Три четверти от восьми - это шесть секунд: хватает с запасом, и при
    // этом отказ ДО установки по-прежнему отличим от «ответа нет».
    let connect_budget = (timeout * 3 / 4).max(Duration::from_secs(3)).min(timeout);
    let mut builder = reqwest::Client::builder();
    for (host, ip) in pinned() {
        // Порт РОВНО НОЛЬ, и это не мелочь: hyper-util подставляет порт из
        // адреса только когда он нулевой (set_port, http.rs:993-997), а
        // reqwest пишет то же самое в своей документации. Поставь сюда 80 - и
        // защищённая ступень молча уедет на восьмидесятый порт.
        builder = builder.resolve(&host, std::net::SocketAddr::new(ip, 0));
    }
    builder
        // The tunnel is made of routes, not of a proxy. Inheriting whatever
        // http_proxy the user's shell happens to export would send the probe
        // somewhere else entirely and answer a question we did not ask.
        .no_proxy()
        .pool_max_idle_per_host(0)
        .timeout(timeout)
        .connect_timeout(connect_budget)
        .user_agent(concat!(
            "ProxysVPN-",
            env!("CARGO_PKG_VERSION"),
            " (probe)"
        ))
        .build()
}

/// Куда стучаться прогревом.
///
/// Первая ступень лестницы, та самая, которую проба спросит следом: к моменту
/// пробы её имя уже лежит в кэше резолвера, а сессия до узла открыта.
/// Прогреваться соединением до САМОГО узла бессмысленно - его адрес xray
/// знает и без резолвера, и весь смысл затеи теряется.
pub fn warm_target() -> (&'static str, u16) {
    // Порт 80, а не 443, и это не оплошность. Прогрев обязан ДОЖДАТЬСЯ ответа
    // с той стороны, а не расписки SOCKS, - значит после установки надо что-то
    // сказать и что-то услышать. По 80 для этого хватает четырёх строк
    // открытым текстом; по 443 пришлось бы вручную собирать рукопожатие TLS.
    //
    // Имя то же, что у первой ступени лестницы: к моменту пробы оно уже лежит
    // в кэше резолвера.
    (host_of(PROBE_LADDER[0]), 80)
}

/// Дождаться, пока туннель СМОЖЕТ открыть соединение.
///
/// Зачем. Сразу после подъёма интерфейса туннель ещё ничего не умеет: xray не
/// договорился с узлом, сессия Reality не открыта, а наш резолвер DoH не
/// поднял своё соединение до 1.1.1.1. Первое имя в этот момент разрешается
/// 9,7 секунды (замер 22.09.2026; до того, как резолвер перевели на DoH, было
/// 30). Дальше - 81 мс.
///
/// Раньше приложение просто спало фиксированные 1,2 секунды и шло проверять.
/// Это гадание: на прогретой сети - лишняя задержка, на холодной - проба
/// обречена, и человек видит «Подтвердить не удалось» поверх исправного
/// туннеля.
///
/// Здесь мы вместо сна ОТКРЫВАЕМ соединение через SOCKS самого xray, называя
/// узел ИМЕНЕМ. Этого хватает, чтобы заставить его разрешить имя через DoH и
/// поднять сессию до узла - то есть оплатить весь холодный старт разом. Ни
/// одного запроса HTTP, ни байта полезных данных: открыли и закрыли.
///
/// Возвращает true, если соединение удалось. False - не приговор: проба
/// разберётся сама, просто ей придётся платить за прогрев самой.
pub async fn warm_through_socks(
    socks_port: u16,
    host: &str,
    port: u16,
    budget: Duration,
) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let host = host.as_bytes();
    // SOCKS5 разрешает не более 255 байт на имя.
    if host.is_empty() || host.len() > 255 {
        return false;
    }

    let work = async {
        let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", socks_port))
            .await
            .ok()?;
        // Приветствие: версия 5, один метод, без авторизации.
        sock.write_all(&[0x05, 0x01, 0x00]).await.ok()?;
        let mut hello = [0u8; 2];
        sock.read_exact(&mut hello).await.ok()?;
        if hello != [0x05, 0x00] {
            return None;
        }
        // Запрос CONNECT по ИМЕНИ (тип 0x03), чтобы имя разрешал xray.
        let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
        req.extend_from_slice(host);
        req.extend_from_slice(&port.to_be_bytes());
        sock.write_all(&req).await.ok()?;
        // Ответ: версия, код, резерв, тип адреса - и дальше сам адрес.
        let mut head = [0u8; 4];
        sock.read_exact(&mut head).await.ok()?;
        if head[1] != 0x00 {
            return None;
        }
        // Дочитываем адрес в ответе, иначе он останется в потоке и попадёт
        // в тело следующего чтения.
        let skip = match head[3] {
            0x01 => 4 + 2,                       // IPv4 + порт
            0x04 => 16 + 2,                      // IPv6 + порт
            0x03 => {
                let mut len = [0u8; 1];
                sock.read_exact(&mut len).await.ok()?;
                usize::from(len[0]) + 2
            }
            _ => return None,
        };
        let mut tail = vec![0u8; skip];
        sock.read_exact(&mut tail).await.ok()?;

        // А ВОТ ТЕПЕРЬ - настоящий обмен.
        //
        // Инбаунд SOCKS у xray отвечает «соединение установлено» немедленно,
        // ещё не начав разрешать имя и не дотянувшись до узла. Замер
        // 22.09.2026: прогрев «проходил» за 0 мс и не грел ничего - проба
        // через шесть секунд снова упиралась в холодный путь.
        //
        // Поэтому ждём БАЙТА С ТОЙ СТОРОНЫ. Он приходит, только когда имя
        // разрешено, сессия до узла открыта и запрос дошёл до сервера, - то
        // есть когда холодный старт действительно оплачен.
        let ask = format!(
            "HEAD / HTTP/1.0\r\nHost: {}\r\nUser-Agent: ProxysVPN (warm)\r\nConnection: close\r\n\r\n",
            std::str::from_utf8(host).ok()?
        );
        sock.write_all(ask.as_bytes()).await.ok()?;
        let mut first = [0u8; 1];
        sock.read_exact(&mut first).await.ok()?;
        Some(())
    };

    matches!(tokio::time::timeout(budget, work).await, Ok(Some(())))
}

/// Прогреть ВТОРУЮ половину пути - ту, по которой пойдёт проба.
///
/// `warm_through_socks` стучится в SOCKS самого xray и потому греет только
/// его: разрешение имени через DoH и сессию до узла. Но проба - и браузер -
/// ходят иначе: по маршруту по умолчанию, то есть через tun2socks, и имя им
/// разрешает СИСТЕМНЫЙ резолвер. Его первый запрос идёт своим путём и платит
/// свою цену заново.
///
/// Замер 22.09.2026 ровно об этом: прогрев через SOCKS прошёл за 1871 мс, а
/// проба следом не смогла соединиться за шесть секунд - и по открытому порту
/// тоже, значит TLS ни при чём.
///
/// Здесь мы делаем то же самое, что сделает проба: разрешаем имя системным
/// резолвером и открываем соединение по маршруту по умолчанию. Отвечает та же
/// сторона и тем же способом, поэтому к пробе всё уже тёплое.
///
/// Возвращает время в миллисекундах, если прошло.
pub async fn warm_through_tunnel(host: &str, port: u16, budget: Duration) -> Option<u128> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let started = Instant::now();
    let work = async {
        // Разрешение имени тут не отделено намеренно: у пробы оно тоже внутри
        // установки соединения, и греть надо ровно то, что она потратит.
        let mut sock = tokio::net::TcpStream::connect((host, port)).await.ok()?;
        let ask = format!(
            "HEAD / HTTP/1.0\r\nHost: {host}\r\nUser-Agent: ProxysVPN (warm)\r\nConnection: close\r\n\r\n"
        );
        sock.write_all(ask.as_bytes()).await.ok()?;
        // Ждём БАЙТ С ТОЙ СТОРОНЫ. tun2socks отвечает на SYN сам, не дожидаясь
        // ничего, поэтому успешный connect ещё ничего не доказывает - на этом
        // я уже обжёгся с распиской SOCKS.
        let mut first = [0u8; 1];
        sock.read_exact(&mut first).await.ok()?;
        Some(())
    };
    match tokio::time::timeout(budget, work).await {
        Ok(Some(())) => Some(started.elapsed().as_millis()),
        _ => None,
    }
}

/// Имя узла из адреса - для журнала. Путь не пишем: он всегда один и тот же.
fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
}

/// Почему запрос не удался, одним словом.
///
/// `reqwest::Error` в журнале разворачивается в абзац с адресом и цепочкой
/// источников; нам нужно ровно одно: в какую стену уткнулись.
fn why(e: &reqwest::Error) -> &'static str {
    // ПОРЯДОК ВАЖЕН. `is_connect()` идёт первым: у reqwest ошибка установки
    // соединения ОДНОВРЕМЕННО отвечает true и на `is_timeout()`, если она
    // пришла по истечении `connect_timeout`. Проверяя таймаут первым, мы
    // теряли именно то различие, ради которого и заведён отдельный бюджет.
    if e.is_connect() {
        if e.is_timeout() {
            // НЕ «не соединился». reqwest заворачивает разрешение имени и
            // установку соединения в один таймаут, и различить их по ошибке
            // нельзя. Прежняя подпись утверждала, что виноват сокет, - и
            // 22.09.2026 увела разбор на всю ночь, пока системный журнал
            // macOS не показал, что проба тринадцать секунд стояла в
            // getaddrinfo и не отправила ни одного SYN.
            "имя не разрешилось или соединение не встало (бюджет установки)"
        } else {
            "не соединился"
        }
    } else if e.is_timeout() {
        "соединился, но ответа нет (истёк общий бюджет)"
    } else if e.is_request() {
        "запрос не ушёл"
    } else if e.is_body() || e.is_decode() {
        "тело не прочиталось"
    } else {
        "ошибка сети"
    }
}

/// Что дала одна попытка.
///
/// Различие, которого тут не было и которое стоило нам вечера. Проба
/// существует, чтобы доказать: байт ушёл в туннель и вернулся. Ответ сервера
/// это доказывает ЛЮБОЙ - хоть 200, хоть 403. Разобранный выходной адрес это
/// приятное дополнение, а не условие.
///
/// 22.09.2026 замер показал, во что обходится путаница: защита Vercel отдавала
/// нашему же немецкому выходу 403 четыре запроса подряд, проба считала это
/// провалом, и окно писало «Соединение поднято, проверочная страница не
/// ответила» поверх исправного туннеля. Страница ОТВЕТИЛА. Просто не тем.
#[derive(Debug)]
enum Answer {
    /// Сервер ответил. Туннель доказан. Адрес есть, если тело удалось разобрать.
    Reached { exit_ip: Option<IpAddr> },
    /// Ответа не было вовсе: не дошли, не соединились, оборвалось.
    Silent,
}

/// Ask one address.
///
/// Каждая неудача пишется в журнал. Раньше здесь стояло `.ok()?` четыре раза
/// подряд, и провал пробы выглядел так: «Соединение поднято, проверочная
/// страница не ответила» - без единой строки о том, какая страница, и почему.
/// 22.09.2026 это стоило разбора вслепую при живом туннеле, который в ту же
/// минуту вёл 26 соединений. Молчать тут нельзя.
///
/// Флуда не будет: лестницу обходят до первого ответа, и на исправном
/// соединении первый же адрес отвечает, не написав ничего.
async fn ask(client: &reqwest::Client, url: &str, timeout: Duration) -> Answer {
    let host = host_of(url);
    let response = match client.get(url).timeout(timeout).send().await {
        Ok(response) => response,
        Err(e) => {
            crate::logger::log("warn", "probe", &format!("{host}: {}", why(&e)));
            return Answer::Silent;
        }
    };
    let status = response.status();
    if !status.is_success() {
        // Ответ есть - значит туннель донёс запрос и принёс ответ. Пишем в
        // журнал, потому что 403 от собственной витрины это новость, но
        // провалом пробы это НЕ является.
        crate::logger::log("warn", "probe", &format!("{host}: ответил {status}"));
        return Answer::Reached { exit_ip: None };
    }
    // A block page can be megabytes. Take a bounded prefix: our answer is
    // never more than a couple of hundred bytes.
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            // Заголовки пришли, тело оборвалось. Туннель всё равно доказан:
            // ответ начал возвращаться.
            crate::logger::log("warn", "probe", &format!("{host}: тело оборвалось, {}", why(&e)));
            return Answer::Reached { exit_ip: None };
        }
    };
    let head = &bytes[..bytes.len().min(MAX_BODY_BYTES)];
    let Ok(text) = std::str::from_utf8(head) else {
        crate::logger::log("warn", "probe", &format!("{host}: ответ не текст"));
        return Answer::Reached { exit_ip: None };
    };
    match parse_exit_ip(text) {
        Some(ip) => Answer::Reached { exit_ip: Some(ip) },
        None => {
            // Тело есть, адреса в нём нет: это страница-заглушка оператора,
            // портал гостиничного Wi-Fi или наша же ошибка формата. Байты всё
            // равно сходили туда и обратно.
            crate::logger::log(
                "warn",
                "probe",
                &format!("{host}: ответ без адреса, {} байт", bytes.len()),
            );
            Answer::Reached { exit_ip: None }
        }
    }
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
    probe_once_racing(meter, ladder, timeout, false).await
}

/// То же, но с выбором: обходить лестницу по очереди или спросить всех разом.
pub async fn probe_once_racing(
    meter: &mut TunnelMeter,
    ladder: &[&str],
    timeout: Duration,
    race: bool,
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
    // Дошёл ли ХОТЬ КАКОЙ-ТО ответ. Именно это, а не разобранный адрес,
    // доказывает, что байт пересёк туннель в обе стороны.
    let mut reached = false;

    match build_client(timeout) {
        Ok(client) => {
            if race {
                // Спрашиваем все адреса РАЗОМ и берём первый ответ.
                //
                // Замер 22.09.2026: `proksya.xyz` не отвечает с нашего же
                // немецкого узла - десять секунд молчания, при том что с
                // амстердамского отвечает за 0,6 с. При обходе по очереди
                // такой адрес съедает весь свой бюджет ПЕРЕД тем, как
                // спросят живого соседа, и проба падает целиком, хотя рабочий
                // адрес был в двух шагах.
                //
                // Очередь остаётся для установившегося хода: там спешить
                // некуда, а лишние запросы к витрине не нужны. Здесь же
                // адресов всего два, и это один раз за подключение.
                let started = Instant::now();
                // `JoinSet` из tokio, а не `FuturesUnordered` из futures-util:
                // futures-util в дереве есть, но только как ЧУЖАЯ зависимость,
                // и опираться на неё нельзя - она уйдёт вместе с тем, кто её
                // притащил. tokio подключён прямо.
                let mut tries = tokio::task::JoinSet::new();
                for (index, url) in ladder.iter().enumerate() {
                    // reqwest::Client - это Arc внутри, клонировать дёшево.
                    let client = client.clone();
                    let url = (*url).to_string();
                    tries.spawn(async move { (index, ask(&client, &url, timeout).await) });
                }
                while let Some(done) = tries.join_next().await {
                    // Упавшая задача - это паника внутри запроса, а не ответ
                    // сервера. Молча пропускаем: остальные ещё идут.
                    let Ok((index, answer)) = done else { continue };
                    if let Answer::Reached { exit_ip: ip } = answer {
                        rtt_ms = Some((started.elapsed().as_millis() as u32).max(1));
                        reached = true;
                        exit_ip = ip;
                        via = Some(index);
                        break;
                    }
                }
                // Остальные запросы больше не нужны: ответ уже есть, и держать
                // их до таймаута значит светить лишними соединениями.
                tries.abort_all();
            } else {
                for (index, url) in ladder.iter().enumerate() {
                    let started = Instant::now();
                    if let Answer::Reached { exit_ip: ip } = ask(&client, url, timeout).await {
                        // Never below 1 ms: a zero would read as "not measured".
                        rtt_ms = Some((started.elapsed().as_millis() as u32).max(1));
                        reached = true;
                        exit_ip = ip;
                        via = Some(index);
                        break;
                    }
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
        // Раньше здесь стояло `exit_ip.is_some()`: щит зеленел только когда
        // НАША витрина вернула разбираемый JSON. 403 от защиты Vercel,
        // страница-заглушка и оборванное тело читались как «туннель не
        // работает», хотя каждый из них доказывает обратное.
        http_ok: reached,
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

    /// Прогрев обязан переспрашивать секундами, а не минутами, - иначе
    /// холодный туннель пять минут числится неподтверждённым.
    #[test]
    fn settling_asks_again_in_seconds_not_minutes() {
        let mut gate = ProbeGate::new();
        gate.mark(100_000);
        assert!(!gate.allows(ProbeReason::Settling, 103_999));
        assert!(gate.allows(ProbeReason::Settling, 104_000));
        // И ради этого он вообще заведён: обычный ход в это время молчал бы
        // ещё почти пять минут.
        assert!(!gate.allows(ProbeReason::IdleForeground, 104_000));
        assert!(
            ProbeReason::Settling.min_gap() * 20 < ProbeReason::IdleForeground.min_gap(),
            "прогрев должен быть на порядок чаще редкого хода"
        );
    }

    /// Холодной пробе дают больше терпения, чем установившейся. Числа живут в
    /// lib.rs, здесь закреплён сам порядок: 2,5 секунды - это про тёплый
    /// туннель, и первому запросу их не хватает.
    #[test]
    fn a_cold_tunnel_gets_more_patience_than_a_warm_one() {
        assert!(
            ATTEMPT_TIMEOUT < Duration::from_secs(3),
            "установившийся бюджет остаётся коротким"
        );
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

    /// Снятие адресов обязано покрывать ОБЕ лестницы.
    ///
    /// Пропущенное имя означает, что проба по нему снова пойдёт в системный
    /// резолвер - тот самый, что виснет на тринадцать секунд сразу после
    /// подъёма туннеля.
    #[test]
    fn pinning_covers_every_rung_of_both_ladders() {
        let mut hosts: Vec<&str> = FIRST_PROBE_LADDER
            .iter()
            .chain(PROBE_LADDER.iter())
            .map(|u| host_of(u))
            .collect();
        hosts.sort_unstable();
        hosts.dedup();
        assert!(!hosts.is_empty(), "лестницы не могут быть пустыми");
        for h in &hosts {
            assert!(!h.contains('/') && !h.contains(':'), "нужно чистое имя: {h}");
            assert!(!h.is_empty(), "пустое имя в лестнице");
        }
    }

    /// Первая проба обязана иметь быстрый путь, не зависящий от чужого TLS.
    #[test]
    fn the_first_probe_has_a_plain_text_fast_lane() {
        assert_eq!(FIRST_PROBE_LADDER.len(), 2, "гонка на двоих, не больше");
        let plain = FIRST_PROBE_LADDER.iter().find(|u| u.starts_with("http://"));
        let secure = FIRST_PROBE_LADDER.iter().find(|u| u.starts_with("https://"));
        let plain = plain.expect("быстрый путь по открытому порту обязателен");
        let secure = secure.expect("защищённый нужен ради выходного адреса");
        // Один и тот же узел: открытый запрос не должен открывать наблюдателю
        // ничего сверх того, что и так видно в рукопожатии защищённого.
        assert_eq!(
            host_of(plain),
            host_of(secure),
            "оба обращения обязаны идти к одному имени"
        );
        assert!(
            PROBE_LADDER.iter().all(|u| u.starts_with("https://")),
            "установившийся ход остаётся целиком защищённым"
        );
    }

    /// Прогрев обязан стучаться в ту же дверь, которую проба откроет следом.
    ///
    /// Иначе он греет не то: разрешит одно имя, а проба спросит другое и снова
    /// заплатит за холодный резолвер.
    #[test]
    fn the_warm_up_knocks_where_the_probe_will_knock() {
        let (host, port) = warm_target();
        // 80, а не 443: прогреву нужен настоящий ответ, а по открытому порту
        // его можно получить четырьмя строками вместо рукопожатия TLS.
        assert_eq!(port, 80, "прогрев говорит открытым текстом");
        assert!(
            PROBE_LADDER[0].contains(host),
            "прогрев должен целиться в первую ступень, а не куда-то ещё"
        );
        assert!(!host.contains('/'), "нужно имя узла, а не кусок адреса: {host}");
        assert!(!host.is_empty() && host.len() <= 255, "SOCKS5 не примет такое имя");
    }

    /// Ответ сервера доказывает туннель, каким бы код ни был.
    ///
    /// 22.09.2026 защита Vercel отдавала нашему немецкому выходу 403 четыре
    /// запроса подряд. Проба считала это провалом и писала «проверочная
    /// страница не ответила» поверх исправного туннеля, который в ту же
    /// секунду нёс десятки килобайт. Страница ответила - просто не тем.
    #[test]
    fn any_answer_proves_the_tunnel_even_a_refusal() {
        // 403 - ответ дошёл: запрос пересёк туннель и вернулся.
        assert_eq!(
            classify(ProbeInputs {
                link_up: true,
                http_ok: true,
                tx_delta: 4_000,
                rx_delta: 4_000,
            }),
            ProbeVerdict::Passed,
            "дошедший ответ обязан зеленить щит, даже без разобранного адреса"
        );
        // А вот тишина при ушедших байтах - это по-прежнему задушенный поток.
        assert_eq!(
            classify(ProbeInputs {
                link_up: true,
                http_ok: false,
                tx_delta: 4_000,
                rx_delta: 0,
            }),
            ProbeVerdict::Blocked
        );
    }

    /// Первая пара лестницы обязана отвечать с НАШИХ выходов.
    #[test]
    fn the_first_two_rungs_are_the_ones_that_answer_from_our_exits() {
        // Замер 22.09.2026: с немецкого узла этот адрес молчит десять секунд.
        // В укороченной лестнице первой пробы ему не место.
        let first_two = &PROBE_LADDER[..2];
        assert!(
            !first_two.iter().any(|u| u.contains("proksya.xyz")),
            "proksya.xyz не отвечает с узла Германии и не должен попадать в первую пару"
        );
        assert!(PROBE_LADDER.iter().any(|u| u.contains("proksya.xyz")),
            "но из лестницы его не убирать: это запасной домен для России");
    }

    /// Мёртвый адрес в лестнице не должен съедать бюджет соседа.
    ///
    /// 22.09.2026 `proksya.xyz` перестал отвечать с немецкого узла, и обход по
    /// очереди отдавал ему все 8 секунд ПЕРЕД тем, как спросить второго. Проба
    /// падала целиком, хотя рабочий адрес стоял следующим. Гонка обязана
    /// уложиться примерно в ОДИН бюджет, а не в сумму по числу адресов.
    #[tokio::test]
    async fn racing_costs_one_budget_not_the_sum() {
        let mut meter = TunnelMeter::new();
        let budget = Duration::from_millis(300);
        // Три заведомо молчащих адреса из диапазона для документации.
        let ladder = [
            "https://203.0.113.1/api/exit-ip",
            "https://203.0.113.2/api/exit-ip",
            "https://203.0.113.3/api/exit-ip",
        ];
        let started = Instant::now();
        let report = probe_once_racing(&mut meter, &ladder, budget, true).await;
        let spent = started.elapsed();
        assert!(report.exit_ip.is_none(), "молчащие адреса не дают адреса");
        assert!(
            spent < budget * 2,
            "гонка заняла {spent:?} при бюджете {budget:?} на адрес - это похоже на обход по очереди"
        );
    }
}
