// src-tauri/src/lib.rs
//
// The core: one place that knows what is true, and one place that says so.
//
// ── What this file is for ──────────────────────────────────────────────────
// The old core answered `vpn_status` once, at window mount, and never spoke
// again; `vpn_connect` returned Ok the moment routes were installed, without a
// single byte having crossed. Both are the same mistake — the window was told
// about INTENTIONS. Everything here exists to replace that with facts:
//
//   • every phase change is emitted (`vpn:state`), from every path, including
//     the ones nobody presses a button for;
//   • `vpn_snapshot` answers the same truth on demand, because events are
//     missed while a window is hidden or reloading;
//   • green is granted by a probe that came back, never by a process that
//     started;
//   • a supervisor keeps watching after the connect call returns, repairs what
//     it can in silence, and speaks only when repair runs out.
//
// ── Platform split ─────────────────────────────────────────────────────────
//   macOS — spawns xray / hysteria / tun2socks and edits the routing table
//           (root; see tun.rs). Paths, sidecar lookup, elevation and the
//           privileged-file rules come from the pvpn-platform crate, which
//           also carries the Windows and Linux backends.
//   Windows, Linux — the same engines and the same core; the tunnel itself
//           goes through the platform layer (tun_platform.rs): elevated
//           in-process on Windows, a pkexec root helper on Linux.
//   iOS   — the tunnel lives in a Network Extension driven through
//           NETunnelProviderManager (ios_vpn.rs).
// The command set and the event set are IDENTICAL on both, because the same
// window renders both. Where a platform genuinely cannot answer, it answers
// "we do not know" rather than growing a command the other one lacks.

// A lock guard in an `if let` / `match` scrutinee lives through the whole
// block, and the repair ladder once deadlocked on exactly that (see `climb`).
#![warn(clippy::significant_drop_in_scrutinee)]

mod appdirs;
mod consent;
mod entropy;
mod errors;
mod events;
mod logger;
mod ping;
mod probe;
mod netmem;
mod manifest;
mod motion;
mod pair_code;
mod site_ladder;
mod subscription;
mod tunnel_prefs;
mod notify_prefs;

// The engines and the tunnel, on every desktop. macOS keeps its own tunnel
// (tun.rs: routes, utun, the system resolver); Windows and Linux drive theirs
// through the platform layer (tun_platform.rs), imported under the same name so
// the core cannot tell which one it is talking to. The same goes for the
// engine pid file: a record on macOS and Linux, a job object on Windows.
#[cfg(desktop)]
mod hysteria_manager;
#[cfg(all(desktop, unix))]
mod pidfile;
#[cfg(target_os = "windows")]
mod pidfile_windows;
#[cfg(target_os = "windows")]
use pidfile_windows as pidfile;
#[cfg(desktop)]
mod sidecars;
#[cfg(target_os = "macos")]
mod engine_stage;
#[cfg(target_os = "macos")]
mod sysdns;
#[cfg(target_os = "macos")]
mod tun;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod tun_platform;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use tun_platform as tun;
#[cfg(desktop)]
mod xray_manager;

#[cfg(target_os = "ios")]
mod ios_vpn;
#[cfg(any(target_os = "ios", test))]
mod xray_apple;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use tauri::{Emitter, Manager, RunEvent};
use tokio::sync::Mutex;
use url::Url;

use errors::{AppError, ErrorCode};
use events::{
    EventPayload, LinkQuality, MetricPayload, StatePayload, StepPayload, SubMeta, VpnPhase,
    VpnSnapshot, VpnStep, EV_EVENT, EV_META, EV_METRIC, EV_STATE, EV_STEP,
};
use probe::{ProbeGate, ProbeReason, ProbeVerdict, TunnelMeter};
use subscription::{fetch_subscription, ServerConfig};
use tunnel_prefs::TransportPref;

#[cfg(desktop)]
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(target_os = "macos")]
use tauri::menu::Submenu;
#[cfg(desktop)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
#[cfg(desktop)]
use tauri::WindowEvent;

// ───────────────────────────────────────────────────────────────────────────
// Timings. Every number here is a product decision, so they live together.
// ───────────────────────────────────────────────────────────────────────────

/// How often the supervisor looks at the world. Passive counters are free, so
/// this is not a cost; it is how quickly a dead tunnel stops being green.
const SUPERVISOR_TICK: Duration = Duration::from_secs(2);

/// How often the routing table is inspected. Slower than the tick because
/// each pass costs two or three processes.
#[cfg(desktop)]
const ROUTE_CHECK_EVERY: Duration = Duration::from_secs(5);

/// Pause before the first probe of a fresh tunnel (design M1). Long enough for
/// the engine's own handshake to finish, short enough that nobody waits.
const FIRST_PROBE_DELAY: Duration = Duration::from_millis(1200);

/// Сколько ждать, пока туннель научится открывать соединения.
///
/// Это не сон, а ожидание события: мы открываем одно соединение через SOCKS
/// самого xray и ждём ответа. На прогретой сети оно приходит за десятки
/// миллисекунд, на холодной - за секунды, и всё это время человек видит
/// честное «Включаем...», а не преждевременный провал.
///
/// 12 секунд - потолок на случай, когда узел не отвечает вовсе. Дальше
/// начинает работать обычный разбор: проба, лечение, следующий узел.
const WARM_BUDGET: Duration = Duration::from_secs(12);

/// When the second warm-up racer starts: late enough not to double every
/// handshake on a healthy path, early enough to cut a lost-SYN tail.
const WARM_SECOND_RACER_AFTER: Duration = Duration::from_millis(700);

/// The name the race partner answers under in the warm-up race.
const RACE_WINNER_PARTNER: &str = "другой протокол";
const PROBE_TIMEOUT: Duration = Duration::from_millis(2500);

/// Терпение для пробы по ХОЛОДНОМУ туннелю.
///
/// 2500 мс выбирали под установившееся соединение: столько нужно плохой
/// мобильной сети, чтобы закончить рукопожатие TLS. К первому запросу после
/// подъёма туннеля это число неприменимо - там сверху ложатся разрешение
/// имени, рукопожатие xray с узлом и открытие сессии Reality, и ни одного
/// байта до этого через туннель не прошло. Замер 22.09.2026 на живой машине:
/// 3,7 секунды. Три прогона подряд падали в таймаут на исправном туннеле,
/// который в ту же минуту вёл 35 соединений.
///
/// 8 секунд - с запасом больше замеренного и всё ещё внутри бюджета
/// подключения; по два адреса это 16 секунд худшего случая.
const COLD_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Сколько времени переспрашивать туннель, который встал, но не подтвердился.
///
/// Считаем ЧАСАМИ, а не попытками, и это исправление собственной ошибки.
/// Сначала тут стояло «четыре попытки», и вышло вот что: одна попытка - это
/// два адреса по 8 секунд, то есть до 16 секунд, плюс пол в 4 секунды. Четыре
/// таких - больше минуты, и всё это время человек смотрел на «Включаем...».
/// Ложный провал обменялся на долгое враньё.
///
/// 45 секунд - это две-три попытки на медленной лестнице и десяток на
/// быстрой. Главное, что потолок известен заранее и не зависит от того, как
/// долго молчит сеть.
const SETTLING_WINDOW: Duration = Duration::from_secs(45);

/// How old a subscription may be and still be used for a connect without
/// asking the network first.
///
/// The owner's target is 3-5 seconds from the button to the shield (27.09.2026);
/// fetching the list cost 1.2 s of the 6.7. Node addresses change seldom, but
/// when a stored one is stale the ladder's step C4 cannot be relied on to
/// notice: it re-reads the subscription with the routes still pointing into
/// the tunnel it is trying to repair. So reuse ends the moment a connect or a
/// repair ends in Failed (`Session::sub_reuse_blocked`): the next Retry tears
/// the tunnel down and asks the network for a fresh list, instead of reusing
/// the stale one for the rest of the 15 minutes.
const SUB_REUSE_MS: u64 = 15 * 60 * 1000;

/// Outgoing bytes with nothing coming back for this long is the passive half
/// of "suspicion" (M1). It never decides anything on its own — it only buys a
/// probe, which does.
const RX_STALL: Duration = Duration::from_secs(10);

/// Restart pauses for an engine that died under us (design: 2, 5, 10 s).
const ENGINE_RESTART_PAUSES: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// A manual choice of country outlives an automatic one for a day.
const PIN_LIFETIME: Duration = Duration::from_secs(24 * 3600);

/// How long a location that failed to carry traffic is passed over. It stays
/// in the list — with "сейчас не проходит в вашей сети" — because a node that
/// one network drops is often the fastest on the next one.
const DEMOTION: Duration = Duration::from_secs(12 * 3600);

/// After a full repair ladder is exhausted, the next one waits. Doubling-ish
/// steps, so a network that is simply gone is not hammered.
const HEAL_BACKOFF: [Duration; 4] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(300),
    Duration::from_secs(900),
];

/// Human-readable events kept for [11]. Five hundred is the design's number.
const TIMELINE_LIMIT: usize = 500;

/// The bot and the cabinet, for the single button on a failure screen.
const BOT_URL: &str = "https://t.me/proxysvpn_bot";
const CABINET_PATH: &str = "/dashboard";

/// Sites that can serve the cabinet and the pairing endpoints, in the order
/// measurement put them (mirrors the ladder in subscription.rs; these are site
/// names, never node addresses). Also the only names a link delivered by a
/// pair code may live on (pair_code.rs); a test keeps it the same set as
/// `subscription::RESERVE_HOSTS`.
const SITE_LADDER: [&str; 4] = [
    "proxysvpn.com",
    "proxysvnovich.vercel.app",
    "proksya.com",
    "proksya.xyz",
];

// ───────────────────────────────────────────────────────────────────────────
// Types the window needs that are NOT part of the core contract
//
// events.rs / types.ts own the tunnel's state and may only grow in step with
// each other. Everything below serves a single screen and is mirrored by hand
// in src/bridge.ts, which carries the list of commands this file must provide.
// Note what `LocationEntry` does NOT have: no host, no port, no protocol. The
// rule "node addresses are never shown" is enforced by the shape of the data.
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum LocationQuality {
    Good,
    Ok,
    Poor,
    /// Twice refused to carry traffic on this network, recently.
    Blocked,
    /// Never measured. Honest: "проверим при подключении".
    Unknown,
}

impl LocationQuality {
    fn from_link(quality: LinkQuality) -> Self {
        match quality {
            LinkQuality::Good => Self::Good,
            LinkQuality::Ok => Self::Ok,
            LinkQuality::Poor => Self::Poor,
            // A protocol with no latency figure is not a slow protocol.
            LinkQuality::Unknown | LinkQuality::NotApplicable => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocationEntry {
    id: String,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    flag: Option<String>,
    quality: LocationQuality,
    /// Рукопожатие TCP до узла, миллисекунды. `None` - числа нет: у hysteria
    /// его не бывает (порт UDP), либо узел не ответил, либо ещё не мерили.
    #[serde(skip_serializing_if = "Option::is_none")]
    rtt_ms: Option<u32>,
    /// A badge the SERVER wrote ("12,4 из 50 ГБ"), never assembled here.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    /// «VLESS · Vision», «VLESS · XHTTP», «Hysteria2»: a technology name,
    /// never an address or a port (ServerConfig::protocol_label).
    protocol: &'static str,
    recent: bool,
    selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubState {
    has_link: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_updated_at: Option<u64>,
    used_fallback: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    platform: &'static str,
    cabinet_url: String,
    bot_url: String,
    device_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_linked_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairSession {
    token: String,
    expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum PairStatus {
    Waiting,
    Linked,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum CheckState {
    // `Idle` and `Pending` are the window's own states before and during a
    // run; the core never sends them, and they are spelled out here so the
    // two halves of this union cannot drift apart unnoticed.
    #[allow(dead_code)]
    Idle,
    #[allow(dead_code)]
    Pending,
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckRow {
    id: &'static str,
    state: CheckState,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    via_fallback: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum CheckService {
    /// The service half needs an endpoint that authorises a subscription
    /// token, and `/api/vpn/check` authorises only a cabinet session. Drawing
    /// a column that cannot work would be the same lie in a new place.
    Unavailable {
        available: bool,
        reason: &'static str,
    },
    // `rename_all` on an enum renames the VARIANTS, not the fields inside
    // them, so a struct-variant needs its own attribute. Without it this sent
    // `can_fix` while the window read `canFix`, so the "Починить" button would
    // have stayed hidden on the day the backend first said it could fix
    // something — a break that compiles and tests green on both sides,
    // because the variant is not constructed yet.
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)] // Reached the day the backend accepts `x-sub-token`.
    Available {
        available: bool,
        rows: Vec<CheckRow>,
        can_fix: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckReport {
    device: Vec<CheckRow>,
    service: CheckService,
    /// The one row that decides the verdict; `None` when everything passed.
    verdict: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum TimelineCode {
    Connected,
    Disconnected,
    Healed,
    TrafficStopped,
    NetworkChanged,
    SubRefreshed,
    SubRefreshedFallback,
    LocationSwitched,
    LocationUnreadable,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelineEntry {
    at_ms: u64,
    code: TimelineCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogLineOut {
    ts_ms: u64,
    level: String,
    source: String,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrafficSplitOut {
    via_vpn: u64,
    direct: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoutingState {
    in_russia: bool,
    /// What we worked out ourselves; `None` when we have no opinion. Always
    /// present in the JSON, because the window distinguishes "no guess" from
    /// "field missing".
    guess: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OnboardingState {
    steps: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SupportReport {
    text: String,
    bytes: usize,
}

// ───────────────────────────────────────────────────────────────────────────
// Session state
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Pinned {
    id: String,
    at: Instant,
}

/// A race computed for exactly one `start_on` call (Watafast).
///
/// `generation` and `for_index` pin it to that call: only a `start_on` whose
/// own `(generation, index)` match is allowed to consume it. Without that, a
/// stale repair-ladder `start_on` from an older generation — or one raising a
/// different index, such as the repair ladder's own next candidate — could
/// steal a partner meant for a start it has nothing to do with, and race a
/// node the person never asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingRace {
    generation: u64,
    for_index: usize,
    partner_index: usize,
}

/// Random per-session credentials for the race SOCKS inbound (Watafast).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RaceCredentials {
    user: String,
    pass: String,
}

struct Session {
    phase: VpnPhase,
    step: Option<VpnStep>,
    location: Option<String>,
    proto: Option<String>,
    error: Option<AppError>,
    healing_since: Option<Instant>,
    metric: MetricPayload,
    meta: SubMeta,

    /// The list as the subscription gave it. Addresses live here and nowhere
    /// else that the window can reach.
    servers: Vec<ServerConfig>,
    current: Option<usize>,
    pinned: Option<Pinned>,
    recents: VecDeque<String>,
    demoted: HashMap<String, Instant>,
    quality: HashMap<String, LocationQuality>,
    /// Замеренные рукопожатия по локациям, для списка стран.
    rtt: HashMap<String, u32>,
    /// Узел, который ПОСЛЕДНИМ действительно подтвердился пробой.
    ///
    /// Не то же самое, что «недавний»: в недавние узел попадает в момент
    /// попытки, удачной или нет. Весь вечер 22.09.2026 приложение бралось за
    /// Германию именно поэтому - она возглавляла список недавних, хотя ни разу
    /// не подтвердилась.
    last_good: Option<String>,
    /// The network we are on (netmem.rs), read at connect before the tunnel
    /// and refreshed by `mend_routes` whenever a route repair reports the
    /// network itself changed — otherwise a probe's `Passed` verdict after a
    /// silent network change would remember the winning node under the OLD
    /// network id, and `demote` would later forget the wrong one's winner.
    network: Option<String>,
    /// Which location last worked on which network.
    net_memory: netmem::NetworkMemory,
    /// A node on another transport to race against the chosen one at the next
    /// engine start (Watafast), scoped to the exact `(generation, index)` of
    /// the `start_on` call it was computed for. A stale repair-ladder
    /// `start_on` from an older generation, or one for a different index,
    /// must not be able to steal it — see `take_race_partner`.
    pending_race: Option<PendingRace>,
    /// Random per-session credentials for the race SOCKS inbound, generated
    /// alongside `pending_race` and cleared with it. The inbound forwards
    /// straight to another country for as long as the race lasts, so it
    /// requires a password rather than trusting "127.0.0.1 only" — any other
    /// local process reaches that loopback port too.
    race_credentials: Option<RaceCredentials>,

    sub_fetched_at: Option<u64>,
    /// Set when a connect or a repair ends in Failed, cleared by the next
    /// successful fetch: a list that just failed is not one to reuse.
    sub_reuse_blocked: bool,
    sub_used_fallback: bool,
    /// The site name that answered last. Used for the cabinet link, so the one
    /// button on a failure screen leads somewhere this person can open.
    sub_source_host: Option<String>,

    meter: TunnelMeter,
    gate: ProbeGate,
    /// До какого момента переспрашивать холодный туннель чаще обычного.
    /// Ставится на подключении, истекает сам.
    settling_until: Option<Instant>,
    /// Щит зелёный по прогреву, а проба ещё ни разу не ответила. Пока так,
    /// надзиратель переспрашивает с шагом прогрева, а не раз в пять минут:
    /// иначе зелёный без пробы значил бы «не проверено до следующей пятиминутки».
    green_unprobed: bool,
    /// Bumped by every connect and every disconnect. A task whose generation
    /// is stale finishes quietly instead of writing over a newer session.
    generation: u64,
    heal_rounds: usize,
    heal_blocked_until: Option<Instant>,
    rx_stalled_since: Option<Instant>,

    routing_in_russia: bool,
    timeline: VecDeque<TimelineEntry>,

    /// «Уведомления» (MoreScreen): true между a "protection dropped" notice
    /// this session actually owes a matching "restored" one and that notice
    /// having been sent. Set the moment a protected phase (`On`/`Unconfirmed`)
    /// first leaves for `Healing`/`Failed`; cleared either by the restore
    /// notice firing or by a manual `disconnect()` — a deliberate stop is not
    /// an outage waiting to be "restored" later.
    protection_drop_notified: bool,
}

/// A phase where traffic is actually believed to cross the tunnel — the two
/// phases a "protection dropped" notice can fall FROM and a "restored" one
/// can return TO. Deliberately excludes `Unconfirmed`'s cousin-in-spirit
/// `Starting`: still connecting is not yet a drop from anything.
fn is_protected_phase(phase: VpnPhase) -> bool {
    matches!(phase, VpnPhase::On | VpnPhase::Unconfirmed)
}

/// A phase that means the tunnel stopped carrying traffic WITHOUT the person
/// asking for that — the engine died and repair is running (`Healing`), or
/// repair gave up (`Failed`). `Off` is never in this set: see `set_phase`'s
/// own comment for why it is always the person's action, not the network's.
fn is_dropped_phase(phase: VpnPhase) -> bool {
    matches!(phase, VpnPhase::Healing | VpnPhase::Failed)
}

/// Whether the supervisor should ask again at the warm-up's pace (4 s)
/// rather than its idle one (5 min): the shield is up but no probe has stood
/// behind it yet — "Unconfirmed", or green granted by the warm-up alone — and
/// the settling window opened at connect has not run out.
fn settling_applies(
    phase: VpnPhase,
    green_unprobed: bool,
    settling_until: Option<Instant>,
    now: Instant,
) -> bool {
    (phase == VpnPhase::Unconfirmed || (phase == VpnPhase::On && green_unprobed))
        && settling_until.is_some_and(|until| now < until)
}

impl Session {
    /// Age of a list a connect may use without fetching, or `None` when it
    /// has to ask the network: no list, too old, or the last attempt with it
    /// ended in Failed.
    fn reusable_sub_age(&self, now: u64) -> Option<u64> {
        if self.servers.is_empty() || self.sub_reuse_blocked {
            return None;
        }
        self.sub_fetched_at
            .map(|at| now.saturating_sub(at))
            .filter(|age| *age < SUB_REUSE_MS)
    }

    fn new() -> Self {
        Self {
            phase: VpnPhase::Off,
            step: None,
            location: None,
            proto: None,
            error: None,
            healing_since: None,
            metric: MetricPayload::default(),
            meta: SubMeta::default(),
            servers: Vec::new(),
            current: None,
            pinned: None,
            recents: VecDeque::new(),
            demoted: HashMap::new(),
            quality: HashMap::new(),
            rtt: HashMap::new(),
            last_good: None,
            network: None,
            // Tests start from nothing: the real file on the developer's disk
            // must not decide a test.
            net_memory: if cfg!(test) { netmem::NetworkMemory::default() } else { netmem::load() },
            pending_race: None,
            race_credentials: None,
            sub_fetched_at: None,
            sub_reuse_blocked: false,
            sub_used_fallback: false,
            sub_source_host: None,
            meter: TunnelMeter::new(),
            gate: ProbeGate::new(),
            settling_until: None,
            green_unprobed: false,
            generation: 0,
            heal_rounds: 0,
            heal_blocked_until: None,
            rx_stalled_since: None,
            // Most of our people are in Russia; the guess is refined from the
            // machine's own timezone as soon as anyone asks.
            routing_in_russia: true,
            timeline: VecDeque::new(),
            protection_drop_notified: false,
        }
    }

    fn state_payload(&self) -> StatePayload {
        StatePayload {
            phase: self.phase,
            location: self.location.clone(),
            proto: self.proto.clone(),
            error: self.error.clone(),
            healing_for_ms: self
                .healing_since
                .map(|since| since.elapsed().as_millis() as u64),
        }
    }

    fn snapshot(&self) -> VpnSnapshot {
        VpnSnapshot {
            state: self.state_payload(),
            metric: self.metric,
            meta: self.meta.clone(),
            step: self.step,
        }
    }

    fn push_event(&mut self, code: TimelineCode, location: Option<String>) {
        if self.timeline.len() >= TIMELINE_LIMIT {
            self.timeline.pop_front();
        }
        self.timeline.push_back(TimelineEntry {
            at_ms: now_ms(),
            code,
            location,
        });
    }

    /// Fold a probe result into the running metric.
    ///
    /// `last_proof_at` is carried across: a failed probe does not erase the
    /// fact that a byte crossed two minutes ago, and the window ages that
    /// number out loud so a stale green is visibly stale.
    fn absorb_metric(&mut self, fresh: MetricPayload) {
        let kept_proof = fresh.last_proof_at.or(self.metric.last_proof_at);
        self.metric = MetricPayload {
            last_proof_at: kept_proof,
            ..fresh
        };
    }

    fn current_server(&self) -> Option<&ServerConfig> {
        self.current.and_then(|i| self.servers.get(i))
    }
}

struct Core {
    app: tauri::AppHandle,
    session: Mutex<Session>,
    /// Serialises connect / disconnect / location change. Held across engine
    /// work, which is why the session lock never is.
    operation: Mutex<()>,
    /// «Уведомления»: не показывать, пока окно в фокусе - человек и так
    /// смотрит на экран, где то же самое уже видно щитом. Обновляется
    /// `WindowEvent::Focused` (macOS; на iOS окна Tauri нет вовсе, и уведомить
    /// там некому этим путём — у Network Extension свои системные события).
    window_focused: std::sync::atomic::AtomicBool,
    /// Язык окна, для текста уведомления - `App.tsx` держит язык на своей
    /// стороне (`LANG_KEY` в localStorage) и сообщает сюда через
    /// `set_ui_lang` при старте и при каждой смене. `false` = ru, значение по
    /// умолчанию до первого вызова - то же, что и умолчание окна.
    ui_lang_en: std::sync::atomic::AtomicBool,
    /// Pair code v1.1: the request id of the code on the sign-in screen, the
    /// same on every press for that code until a sign-in forgets it
    /// (pair_code.rs). Its own lock, held for no network call.
    pair_code_ids: pair_code::RedeemIds,

    #[cfg(desktop)]
    xray: xray_manager::SharedXrayState,
    #[cfg(desktop)]
    hysteria: hysteria_manager::SharedHysteriaState,
    #[cfg(desktop)]
    tun: tun::SharedTunState,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    now_ms() / 1000
}

// ───────────────────────────────────────────────────────────────────────────
// Saying it out loud
// ───────────────────────────────────────────────────────────────────────────

impl Core {
    fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self {
            app,
            session: Mutex::new(Session::new()),
            operation: Mutex::new(()),
            window_focused: std::sync::atomic::AtomicBool::new(true),
            ui_lang_en: std::sync::atomic::AtomicBool::new(false),
            pair_code_ids: pair_code::RedeemIds::default(),
            #[cfg(desktop)]
            xray: xray_manager::new_state(),
            #[cfg(desktop)]
            hysteria: hysteria_manager::new_state(),
            #[cfg(desktop)]
            tun: tun::new_state(),
        })
    }

    async fn emit_state(&self) {
        let payload = self.session.lock().await.state_payload();
        let _ = self.app.emit(EV_STATE, payload);
    }

    /// The ONLY way a phase changes. Every transition therefore reaches the
    /// window, including the ones no button caused — which is the whole bug
    /// this core was written to remove.
    async fn set_phase(&self, phase: VpnPhase, error: Option<AppError>) {
        let (payload, drop_notice, restore_notice) = {
            let mut s = self.session.lock().await;
            let old_phase = s.phase;
            s.phase = phase;
            s.error = error;
            if phase == VpnPhase::Failed {
                // See SUB_REUSE_MS: the next attempt asks for a fresh list.
                s.sub_reuse_blocked = true;
            }
            if phase == VpnPhase::Healing {
                s.healing_since.get_or_insert_with(Instant::now);
            } else {
                s.healing_since = None;
            }
            if phase != VpnPhase::Starting {
                s.step = None;
            }
            if phase == VpnPhase::Off {
                s.location = None;
                s.proto = None;
                // A deliberate stop closes any outage this session was still
                // tracking: there is nothing left to call "restored" on the
                // next connect, and doing so would describe the person's own
                // action back to them as if it had happened by itself.
                s.protection_drop_notified = false;
            }
            // «Уведомления» (MoreScreen, notify_prefs.rs). Two independent
            // events, decided from the phase transition alone so this never
            // needs a caller to remember to ask for it:
            //   - "protection dropped": a phase that WAS protected leaves for
            //     one that is not carrying traffic (`Healing` — the engine
            //     died and repair is running — or `Failed` — repair gave up,
            //     including the rarer case of failing before ever reaching
            //     `Healing`). `Off` is deliberately excluded from "not
            //     protected" here — seeing it is always the person's own
            //     doing, directly or through a settings reconnect
            //     (`reconnect_if_live`), never the network's.
            //   - "protection restored": back to a protected phase, but ONLY
            //     when a drop notice is actually owed — never on an ordinary
            //     first connect, which never set the flag to begin with.
            let old_protected = is_protected_phase(old_phase);
            let new_protected = is_protected_phase(phase);
            let drop_notice = old_protected && is_dropped_phase(phase);
            let restore_notice = !old_protected && new_protected && s.protection_drop_notified;
            if drop_notice {
                s.protection_drop_notified = true;
            } else if restore_notice {
                s.protection_drop_notified = false;
            }
            (s.state_payload(), drop_notice, restore_notice)
        };
        let _ = self.app.emit(EV_STATE, payload);
        #[cfg(desktop)]
        self.refresh_tray(phase).await;
        #[cfg(desktop)]
        {
            if drop_notice {
                self.notify_protection(false).await;
            } else if restore_notice {
                self.notify_protection(true).await;
            }
        }
    }

    /// Progress inside `Starting`, and only there.
    ///
    /// The same code raises an engine during a repair and during a location
    /// change, where the phase is `Healing` or `On`. A step emitted then would
    /// put "Проверяю подписку" under a green shield — the contract says a step
    /// belongs to `Starting`, so the guard lives here rather than in five
    /// call sites that would each have to remember.
    async fn set_step(&self, step: VpnStep) {
        {
            let mut s = self.session.lock().await;
            if s.phase != VpnPhase::Starting {
                return;
            }
            s.step = Some(step);
        }
        let _ = self.app.emit(EV_STEP, StepPayload { step });
    }

    async fn emit_metric(&self) {
        let metric = self.session.lock().await.metric;
        let _ = self.app.emit(EV_METRIC, metric);
    }

    /// A one-off notice. `handled` means the core is already working on it and
    /// the person is not being asked for anything — the window stays quiet.
    fn emit_event(&self, error: AppError, handled: bool) {
        let _ = self.app.emit(EV_EVENT, EventPayload { error, handled });
    }

    async fn emit_meta(&self) {
        let meta = self.session.lock().await.meta.clone();
        let _ = self.app.emit(EV_META, meta);
    }

    async fn note(&self, code: TimelineCode, location: Option<String>) {
        self.session.lock().await.push_event(code, location);
    }

    /// Start a new generation and return it. Anything older stops writing.
    ///
    /// Called at the top of both `connect` and `disconnect` — the only two
    /// places that do — so this is also where a pending race is retired: a
    /// partner scoped to the old generation has no `start_on` left that could
    /// legitimately claim it, and a fresh connect computes its own if one
    /// applies.
    async fn bump_generation(&self) -> u64 {
        let mut s = self.session.lock().await;
        s.generation = s.generation.wrapping_add(1);
        s.pending_race = None;
        s.race_credentials = None;
        s.generation
    }

    async fn is_current(&self, generation: u64) -> bool {
        self.session.lock().await.generation == generation
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Where the subscription link lives
//
// The design wants the Keychain (M8, stage 4). Until that module exists the
// link lives beside the device id, in the application support directory, with
// the same two-location rule and 0600: the launcher starts us with two
// different `HOME`s depending on the path it takes, so a per-user file alone
// would be two different installations on one machine.
// ───────────────────────────────────────────────────────────────────────────

fn link_paths() -> Vec<std::path::PathBuf> {
    appdirs::state_dirs()
        .into_iter()
        .map(|dir| dir.join("sub-link"))
        .collect()
}

fn load_link() -> Option<String> {
    for path in link_paths() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn link_stored_at() -> Option<u64> {
    for path in link_paths() {
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                if let Ok(age) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                    return Some(age.as_millis() as u64);
                }
            }
        }
    }
    None
}

fn store_link(link: &str) -> Result<(), AppError> {
    let mut last: Option<std::io::Error> = None;
    for path in link_paths() {
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                continue;
            }
        }
        match std::fs::write(&path, format!("{link}\n")) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    // The token in this link IS the credential; unlike the
                    // device id it must not be world readable.
                    let _ =
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                return Ok(());
            }
            Err(e) => last = Some(e),
        }
    }
    logger::log(
        "error",
        "app",
        &format!(
            "could not store the subscription link: {}",
            last.map(|e| e.to_string())
                .unwrap_or_else(|| "no writable location".to_string())
        ),
    );
    Err(AppError::new(ErrorCode::Unknown))
}

fn clear_link() {
    for path in link_paths() {
        let _ = std::fs::remove_file(path);
    }
}

/// A link we are willing to keep. Deliberately strict: a person who pastes the
/// wrong thing gets "это не ссылка ProxysVPN" now rather than a failed connect
/// in ten seconds.
fn validate_link(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(ErrorCode::NoSubscription));
    }
    let url = Url::parse(trimmed).map_err(|_| AppError::new(ErrorCode::SubMalformed))?;
    // https only: the token in the path is the account's credential, and a
    // plain-http link sent it in clear text to every site of the ladder.
    if url.scheme() != "https" {
        return Err(AppError::new(ErrorCode::SubMalformed));
    }
    if url.host_str().is_none() || url.path().trim_matches('/').is_empty() {
        return Err(AppError::new(ErrorCode::SubMalformed));
    }
    Ok(trimmed.to_string())
}

/// The link as we ask for it, given where the person says they are.
///
/// `split=0` is the service's own flag for "everything through the tunnel";
/// it is a query parameter of the subscription, not a client setting, which is
/// why the answer also carries `routing-enable: false` and the routing layer
/// switches the previous Russian rules off instead of quietly keeping them.
fn effective_sub_url(link: &str, in_russia: bool) -> Result<String, AppError> {
    let mut url = Url::parse(link).map_err(|_| AppError::new(ErrorCode::SubMalformed))?;
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "split")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut qs = url.query_pairs_mut();
        qs.clear();
        for (k, v) in kept {
            qs.append_pair(&k, &v);
        }
        if !in_russia {
            qs.append_pair("split", "0");
        }
    }
    if url.query() == Some("") {
        url.set_query(None);
    }
    Ok(url.to_string())
}

// ───────────────────────────────────────────────────────────────────────────
// Locations: labels without addresses
// ───────────────────────────────────────────────────────────────────────────

/// A stable id for a location that is not, and can never become, an address.
///
/// Derived from the label the subscription wrote, so it survives a node
/// address change — the thing that happens most often in this service — and
/// dies only when the service renames the country, which is when a pinned
/// choice SHOULD be reconsidered.
fn location_id(label: &str, ordinal: usize) -> String {
    // FNV-1a: four lines, no dependency, and stable across runs, which a
    // DefaultHasher explicitly is not.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in label.trim().to_lowercase().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if ordinal == 0 {
        format!("{hash:x}")
    } else {
        // Two nodes behind one label: the second gets its own id rather than
        // silently becoming the first.
        format!("{hash:x}-{ordinal}")
    }
}

/// Split a subscription remark into the flag it starts with and the rest.
///
/// The service writes labels like "🇳🇱 Нидерланды" and sometimes appends a
/// badge ("США · 12,4 из 50 ГБ"). Both belong to the server; the app renders
/// them and never assembles them.
fn split_label(remark: &str) -> (Option<String>, String, Option<String>) {
    let text = remark.trim();
    let mut chars = text.chars().peekable();
    let mut flag = String::new();
    while let Some(c) = chars.peek() {
        // Regional indicator symbols, i.e. a country flag.
        if ('\u{1F1E6}'..='\u{1F1FF}').contains(c) {
            flag.push(*c);
            chars.next();
        } else {
            break;
        }
    }
    let rest: String = chars.collect();
    let rest = rest.trim().to_string();
    let (label, note) = match rest.split_once('·') {
        Some((left, right)) => (left.trim().to_string(), Some(right.trim().to_string())),
        None => (rest, None),
    };
    (
        if flag.is_empty() { None } else { Some(flag) },
        label,
        note.filter(|n| !n.is_empty()),
    )
}

/// The label a person sees when the service sent none.
///
/// Never the host: the old list put the node address on screen whenever a
/// remark was empty (App.tsx:293-322).
fn fallback_label(index: usize) -> String {
    format!("#{}", index + 1)
}

impl Session {
    /// The list as the window should see it.
    fn locations(&self) -> Vec<LocationEntry> {
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut out = Vec::with_capacity(self.servers.len());
        let now = Instant::now();

        for (index, server) in self.servers.iter().enumerate() {
            let remark = server.remark().trim();
            let (flag, label, note) = if remark.is_empty() {
                (None, fallback_label(index), None)
            } else {
                split_label(remark)
            };
            let note = subscription::display_note(note);
            let label = if label.is_empty() {
                fallback_label(index)
            } else {
                label
            };
            let ordinal = seen.entry(label.to_lowercase()).or_insert(0);
            let id = location_id(&label, *ordinal);
            *ordinal += 1;

            let quality = if self
                .demoted
                .get(&id)
                .is_some_and(|until| *until > now)
            {
                LocationQuality::Blocked
            } else {
                self.quality
                    .get(&id)
                    .copied()
                    .unwrap_or(LocationQuality::Unknown)
            };

            out.push(LocationEntry {
                recent: self.recents.contains(&id),
                selected: self.pinned.as_ref().is_some_and(|p| p.id == id),
                rtt_ms: self.rtt.get(&id).copied(),
                id,
                label,
                flag,
                quality,
                note,
                protocol: server.protocol_label(),
            });
        }
        out
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.locations().iter().position(|entry| entry.id == id)
    }

    fn id_of(&self, index: usize) -> Option<String> {
        self.locations().get(index).map(|entry| entry.id.clone())
    }

    /// Whether `network`'s remembered winner is one we would actually start
    /// on right now: present, fresh (`net_memory.winner` already drops stale
    /// entries), still in the current list, and not demoted.
    ///
    /// This is the ONE place that decides "is this network's winner usable",
    /// used both by `choose_server`'s own network-winner step below and by
    /// the "known network" gate at connect (lib.rs, races only unknown
    /// networks). Before this they could disagree: the gate asked only
    /// whether the memory had an entry, `choose_server` also asked whether it
    /// was demoted, and a demoted winner made the gate say "known" — no race
    /// — while `choose_server` quietly started on someone else.
    fn network_winner_usable(&self, network: &str) -> Option<usize> {
        let id = self.net_memory.winner(network, now_ms())?;
        let index = self.index_of(id)?;
        let now = Instant::now();
        let healthy = self
            .id_of(index)
            .map(|id| self.demoted.get(&id).is_none_or(|until| *until <= now))
            .unwrap_or(true);
        healthy.then_some(index)
    }

    /// Which node to try, and why.
    ///
    /// Order: a manual pin that is still young, then the last one that worked,
    /// then the first that is not currently demoted, then anything at all. The
    /// last step matters: a list where every node is demoted still has to
    /// produce a candidate, or the app refuses to try on the one network where
    /// the person is stuck.
    ///
    /// `transport` (TunnelScreen «Способ подключения») is a SOFT preference
    /// layered on top of that order, not a new tier of its own: a manual pin
    /// still wins outright (choosing one specific line already accepts its
    /// transport), and every automatic tier below it prefers a match but
    /// falls through to its own next tier rather than refuse a candidate it
    /// would otherwise have picked. Between "fastest measured" and "anything
    /// at all" sits one more step for exactly this preference: an unmeasured
    /// list (the common case — nothing has an RTT yet) must not fall all the
    /// way to ignoring `transport` just because nothing is measured. Only the
    /// very last step ignores `transport` entirely, for the same reason it
    /// already ignores health — a preference is not a reason to end up with
    /// no node at all on the one network where nothing of the wanted kind is
    /// reachable.
    fn choose_server(&self, transport: TransportPref) -> Option<usize> {
        if self.servers.is_empty() {
            return None;
        }
        let now = Instant::now();
        if let Some(pin) = &self.pinned {
            if pin.at + PIN_LIFETIME > now {
                if let Some(index) = self.index_of(&pin.id) {
                    return Some(index);
                }
            }
        }
        let healthy = |index: usize| -> bool {
            self.id_of(index)
                .map(|id| self.demoted.get(&id).is_none_or(|until| *until <= now))
                .unwrap_or(true)
        };
        let wants = |index: usize| -> bool {
            self.servers
                .get(index)
                .is_some_and(|server| subscription::matches_transport_pref(server, transport))
        };
        // Тот, что подтвердился последним В ЭТОЙ СЕТИ (Watafast, 27.09.2026).
        // Дома проходит одно, в офисе и на мобильном - другое; общий
        // «последний удачный» начинал бы каждую сеть с чужого победителя.
        if let Some(network) = &self.network {
            if let Some(index) = self.network_winner_usable(network) {
                if wants(index) {
                    return Some(index);
                }
            }
        }
        // Тот, что ПОДТВЕРДИЛСЯ последним. Факт сильнее любой оценки.
        if let Some(id) = &self.last_good {
            if let Some(index) = self.index_of(id) {
                if healthy(index) && wants(index) {
                    return Some(index);
                }
            }
        }
        // Самый быстрый из ЗАМЕРЕННЫХ.
        //
        // До этого здесь стоял просто первый живой из списка - то есть выбор
        // определялся порядком, в котором подписка перечислила страны. У
        // владельца первой шла Германия, и приложение упорно бралось за неё,
        // хотя Амстердам в тот вечер отвечал заметно быстрее.
        //
        // Берём рукопожатие TCP до узла - то же число, что видно в списке
        // стран. Узлы без замера (их ещё не мерили, или это Hysteria2, где
        // рукопожатия TCP не бывает) не проигрывают автоматически: если
        // замеров нет ни у кого, работает прежний порядок. Но и не выигрывают
        // вслепую - предпочесть неизвестное известному быстрому не за что.
        let fastest = (0..self.servers.len())
            .filter(|index| healthy(*index) && wants(*index))
            .filter_map(|index| {
                let id = self.id_of(index)?;
                let ms = self.rtt.get(&id).copied()?;
                Some((ms, index))
            })
            .min();
        if let Some((_, index)) = fastest {
            return Some(index);
        }
        // Ни одного ЗАМЕРЕННОГО под здоровье и транспорт сразу - это не то же
        // самое, что «ничего подходящего нет вовсе»: чаще всего это просто
        // список, который ещё никто не мерил. Первый живой, что подходит под
        // выбранный транспорт, всё ещё стоит найти раньше, чем сдаваться на
        // предпочтении - тот же принцип, каким живёт авто-выбор без
        // предпочтения (первый живой при отсутствии замеров), внутри
        // отфильтрованного по транспорту подмножества.
        if let Some(index) = (0..self.servers.len()).find(|index| healthy(*index) && wants(*index)) {
            return Some(index);
        }
        // И только когда под выбранный транспорт нет вообще ни одного живого
        // узла - предпочтение больше не спрашивают. Живой узел важнее точного
        // совпадения транспорта, а на сети, где нет ни одного узла вовсе,
        // первый по списку лучше отказа подключаться.
        (0..self.servers.len())
            .find(|index| healthy(*index))
            .or(Some(0))
    }

    /// The next candidate after a failure, same protocol where possible.
    fn next_server(&self, after: usize, same_proto: bool) -> Option<usize> {
        let proto = self.servers.get(after).map(ServerConfig::proto);
        let total = self.servers.len();
        (1..total)
            .map(|step| (after + step) % total)
            .find(|index| {
                let candidate = &self.servers[*index];
                let proto_ok = match (same_proto, proto) {
                    (true, Some(p)) => candidate.proto() == p,
                    (false, Some(p)) => candidate.proto() != p,
                    _ => true,
                };
                proto_ok
                    && self
                        .id_of(*index)
                        .map(|id| {
                            self.demoted
                                .get(&id)
                                .is_none_or(|until| *until <= Instant::now())
                        })
                        .unwrap_or(true)
            })
    }

    fn remember_use(&mut self, index: usize) {
        let Some(id) = self.id_of(index) else { return };
        self.recents.retain(|existing| *existing != id);
        self.recents.push_front(id);
        while self.recents.len() > 3 {
            self.recents.pop_back();
        }
    }

    fn demote(&mut self, index: usize) {
        if let Some(id) = self.id_of(index) {
            // It stopped getting through here: this network forgets it as its
            // winner, or the next connect would start on it again.
            if let Some(network) = self.network.clone() {
                if self.net_memory.winner(&network, now_ms()) == Some(id.as_str()) {
                    self.net_memory.forget(&network);
                    let snapshot = self.net_memory.clone();
                    std::thread::spawn(move || netmem::store(&snapshot));
                }
            }
            self.demoted.insert(id, Instant::now() + DEMOTION);
        }
    }

    /// A node on ANOTHER transport to race against `chosen` (Watafast).
    ///
    /// Only VLESS against VLESS: both ride one xray. Same exit first (the
    /// same country, «Британия» and «Британия · XHTTP»): then only the
    /// transport differs, and the race asks exactly "which way gets through
    /// here". Then the fastest measured, then list order. `None` when every
    /// other node speaks the chosen transport or is demoted.
    fn race_partner(&self, chosen: usize) -> Option<usize> {
        let a = self.servers.get(chosen)?;
        if !matches!(a, ServerConfig::Vless(_)) {
            return None;
        }
        let a_proto = a.protocol_label();
        let a_place = split_label(a.remark()).1.to_lowercase();
        let now = Instant::now();
        (0..self.servers.len())
            .filter(|&i| i != chosen)
            .filter(|&i| matches!(self.servers[i], ServerConfig::Vless(_)))
            .filter(|&i| self.servers[i].protocol_label() != a_proto)
            .filter(|&i| {
                self.id_of(i)
                    .map(|id| self.demoted.get(&id).is_none_or(|until| *until <= now))
                    .unwrap_or(true)
            })
            .min_by_key(|&i| {
                let place = split_label(self.servers[i].remark()).1.to_lowercase();
                let rtt = self
                    .id_of(i)
                    .and_then(|id| self.rtt.get(&id).copied())
                    .unwrap_or(u32::MAX);
                (place != a_place, rtt, i)
            })
    }

    /// Consume `pending_race`, but only for the exact `start_on` call it was
    /// computed for.
    ///
    /// A plain `.take()` handed the race to WHICHEVER `start_on` ran next,
    /// generation and index be damned: a stale repair-ladder `start_on` from
    /// an older generation could steal it, and so could a `start_on` raising
    /// a different index than the one the race was computed against. Neither
    /// of those should ever build a race config for a partner meant for a
    /// different start.
    fn take_race_partner(&mut self, generation: u64, index: usize) -> Option<usize> {
        match self.pending_race {
            Some(p) if p.generation == generation && p.for_index == index => {
                self.pending_race = None;
                Some(p.partner_index)
            }
            _ => None,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Engines — the one place the platforms differ
// ───────────────────────────────────────────────────────────────────────────

#[cfg(desktop)]
impl Core {
    /// Bring up the chain for `server`, leaving tun2socks alone if it already
    /// runs. That last part is the soft location change: the device, its
    /// address and both halves of the default route are never touched.
    async fn engine_start(
        &self,
        server: &ServerConfig,
        partner: Option<&ServerConfig>,
        race_creds: Option<&RaceCredentials>,
    ) -> Result<(), AppError> {
        let physical = tun::physical_default().await?;
        let config = match (partner, race_creds) {
            (Some(ServerConfig::Vless(other)), Some(creds))
                if matches!(server, ServerConfig::Vless(_)) =>
            {
                xray_manager::build_race_config(
                    server,
                    other,
                    &physical.interface,
                    (&creds.user, &creds.pass),
                )?
            }
            // A partner without credentials never races: better a plain
            // config than a race inbound with no password on it.
            _ => xray_manager::build_runtime_config(server, &physical.interface)?,
        };

        // Hysteria first: xray's outbound points at its SOCKS port, and an
        // xray that starts against a port nobody listens on spends its first
        // seconds failing every connection.
        hysteria_manager::stop(&self.hysteria).await?;
        if let ServerConfig::Hy2(cfg) = server {
            hysteria_manager::start(&self.hysteria, &self.app, cfg).await?;
        }

        let (bin, assets) = xray_manager::xray_paths(&self.app)?;
        xray_manager::restart(&self.xray, &bin, &assets, config).await
    }

    /// Raise the tunnel, or move the live one to a different exit.
    async fn tunnel_up(&self, server: &ServerConfig, keep: bool) -> Result<(), AppError> {
        if keep {
            // Only the host route moves, and the new one is added before the
            // old one goes: there is no instant where the engine's traffic to
            // a node has no way out but our own tunnel.
            tun::retarget(&self.tun, server.host()).await
        } else {
            tun::start(&self.tun, &self.app, server.host()).await?;
            tun::persist_route_hint(&self.tun).await;
            Ok(())
        }
    }

    async fn engine_down(&self) {
        let _ = tun::stop(&self.tun).await;
        let _ = xray_manager::stop(&self.xray).await;
        let _ = hysteria_manager::stop(&self.hysteria).await;
        ping::clear_target();
    }

    /// Which part of the chain is missing, if any.
    async fn dead_engine(&self) -> Option<&'static str> {
        if !tun::is_running(&self.tun).await {
            return Some("tun2socks");
        }
        if !xray_manager::is_running(&self.xray).await {
            return Some("xray");
        }
        let needs_hy2 = matches!(
            self.session.lock().await.current_server(),
            Some(ServerConfig::Hy2(_))
        );
        if needs_hy2 && !hysteria_manager::is_running(&self.hysteria).await {
            return Some("hysteria");
        }
        None
    }

    /// True when every process the current node needs is alive.
    ///
    /// This is what `vpn_status` used to get wrong: it asked about xray only,
    /// and on a Hysteria2 node xray was not even started, so the answer was
    /// permanently `false` on the Netherlands.
    async fn engines_alive(&self) -> bool {
        self.dead_engine().await.is_none()
    }
}

/// The iOS Simulator cannot run Network Extensions at all: the connect below
/// could only fail there, with a phrase promising that a retry helps. Say
/// what it really is (28.09.2026, the owner saw exactly that on an iPad
/// simulator).
#[cfg(all(target_os = "ios", target_abi = "sim"))]
async fn ios_engine_start(_server: &ServerConfig) -> Result<(), AppError> {
    Err(AppError::new(ErrorCode::SimulatorNoVpn))
}

#[cfg(all(target_os = "ios", not(target_abi = "sim")))]
async fn ios_engine_start(server: &ServerConfig) -> Result<(), AppError> {
    ios_vpn::connect(server).await.map_err(|e| {
        logger::log("error", "ios-vpn", &format!("connect failed: {e}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })
}

#[cfg(target_os = "ios")]
impl Core {
    /// There is no second half on iOS: the extension owns the device, the
    /// routes and the engine, and starts all three from one config.
    async fn engine_start(
        &self,
        server: &ServerConfig,
        _partner: Option<&ServerConfig>,
        _race_creds: Option<&RaceCredentials>,
    ) -> Result<(), AppError> {
        ios_engine_start(server).await
    }

    async fn tunnel_up(&self, _server: &ServerConfig, _keep: bool) -> Result<(), AppError> {
        Ok(())
    }

    async fn engine_down(&self) {
        ios_vpn::disconnect().await;
        ping::clear_target();
    }

    async fn dead_engine(&self) -> Option<&'static str> {
        if ios_vpn::is_connected_wait().await {
            None
        } else {
            Some("network-extension")
        }
    }

    async fn engines_alive(&self) -> bool {
        self.dead_engine().await.is_none()
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Connect
// ───────────────────────────────────────────────────────────────────────────

impl Core {
    async fn connect(self: &Arc<Self>) -> Result<(), AppError> {
        // Хронометраж подключения по шагам: 27.09.2026 человек видел 20-30
        // секунд, а журнал сумел показать только последние семь.
        let pressed = Instant::now();
        logger::log("info", "vpn", "подключение: нажата кнопка");
        let _op = self.operation.lock().await;
        let waited = pressed.elapsed().as_millis();
        if waited > 300 {
            logger::log("warn", "vpn", &format!("подключение ждало другую операцию {waited} мс"));
        }
        let generation = self.bump_generation().await;

        self.engine_down().await;
        {
            let mut s = self.session.lock().await;
            s.green_unprobed = false;
            s.meter.reset();
            s.gate.reset();
            s.metric = MetricPayload::default();
            s.rx_stalled_since = None;
            s.heal_rounds = 0;
            s.heal_blocked_until = None;
        }
        logger::reset_traffic_split();
        self.set_phase(VpnPhase::Starting, None).await;

        match self.connect_inner(generation).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if self.is_current(generation).await {
                    self.note(TimelineCode::Failed, None).await;
                    self.engine_down().await;
                    {
                        // The same backoff the repair ladder uses. Without it,
                        // a subscription that refuses us — no balance, link
                        // taken — would be re-fetched on every wake forever.
                        let mut s = self.session.lock().await;
                        let wait = heal_backoff(s.heal_rounds);
                        s.heal_rounds += 1;
                        s.heal_blocked_until = Some(Instant::now() + wait);
                    }
                    self.set_phase(VpnPhase::Failed, Some(err.clone())).await;
                    // Watch on even in failure: a machine that comes back from
                    // sleep on a working network should not need the button
                    // pressed again. The supervisor does nothing else while
                    // the phase is Failed.
                    self.spawn_supervisor(generation);
                }
                Err(err)
            }
        }
    }

    async fn connect_inner(self: &Arc<Self>, generation: u64) -> Result<(), AppError> {
        let started = Instant::now();
        self.set_step(VpnStep::FetchingSub).await;
        let reusable_age = self.session.lock().await.reusable_sub_age(now_ms());
        if let Some(age) = reusable_age {
            logger::log(
                "info",
                "vpn",
                &format!("подписка: сохранённая, {} с назад, сеть не ждём", age / 1000),
            );
        } else {
            let fetched = self.refresh_subscription().await;
            logger::log(
                if fetched.is_ok() { "info" } else { "warn" },
                "vpn",
                &format!(
                    "подписка: {} за {} мс",
                    if fetched.is_ok() { "получена" } else { "НЕ получена" },
                    started.elapsed().as_millis()
                ),
            );
            fetched?;
        }
        if !self.is_current(generation).await {
            return Ok(());
        }

        self.set_step(VpnStep::PickingServer).await;
        // Сеть узнаём ДО подъёма туннеля, по роутеру (netmem.rs).
        #[cfg(desktop)]
        let network = netmem::current_network().await;
        #[cfg(not(desktop))]
        let network: Option<String> = None;
        let index = {
            let mut s = self.session.lock().await;
            s.network = network;
            // Same predicate `choose_server` uses for its own network-winner
            // step (`network_winner_usable`): a "known" network must be one
            // whose winner `choose_server` would actually pick, or the two
            // disagree and the app skips a race it still needed.
            let known = s
                .network
                .as_deref()
                .and_then(|n| s.network_winner_usable(n))
                .is_some();
            logger::log(
                "info",
                "vpn",
                match (&s.network, known) {
                    (None, _) => "сеть: не распознана",
                    (Some(_), true) => "сеть: знакомая, начинаем с её победителя",
                    (Some(_), false) => "сеть: новая для приложения",
                },
            );
            let index = s
                .choose_server(tunnel_prefs::load().transport)
                .ok_or_else(|| AppError::new(ErrorCode::SubEmpty))?;
            // Race only where we know nothing: no pin by hand, no winner on
            // this network. A known network starts on its winner alone.
            let pinned = s
                .pinned
                .as_ref()
                .is_some_and(|p| p.at + PIN_LIFETIME > Instant::now());
            // `xray_manager` (and the race it can build) only exists on
            // macOS — on iOS `partner` is always `None` below, so this whole
            // decision is gated the same way rather than referencing that
            // module where it does not exist.
            #[cfg_attr(not(desktop), allow(unused_variables))]
            let partner = if cfg!(desktop) && !pinned && !known {
                s.race_partner(index)
            } else {
                None
            };
            s.pending_race = None;
            s.race_credentials = None;
            #[cfg(desktop)]
            if let Some(partner_index) = partner {
                // The race inbound needs its own password (defence in
                // depth): it forwards straight to another country for as
                // long as the race lasts. Failing to mint one — the system RNG
                // unreadable — cancels the race rather than running it
                // unauthenticated.
                match xray_manager::race_credentials() {
                    Ok((user, pass)) => {
                        s.pending_race = Some(PendingRace {
                            generation,
                            for_index: index,
                            partner_index,
                        });
                        s.race_credentials = Some(RaceCredentials { user, pass });
                    }
                    Err(err) => {
                        logger::log(
                            "warn",
                            "vpn",
                            &format!("гонка протоколов: пароль не выдан, гонки не будет: {err}"),
                        );
                    }
                }
            }
            if let Some(p) = s.pending_race {
                logger::log(
                    "info",
                    "vpn",
                    &format!(
                        "гонка протоколов: {} против {}",
                        s.servers[index].protocol_label(),
                        s.servers[p.partner_index].protocol_label()
                    ),
                );
            }
            index
        };
        // `race_credentials` only matters on macOS, where a race can actually
        // be pending; on iOS `pending_race` is never set (see above), so it
        // stays `None` and would otherwise be an unused-variable warning.
        #[cfg_attr(not(desktop), allow(unused_variables))]
        let (race_partner, race_credentials) = {
            let s = self.session.lock().await;
            (
                s.pending_race.map(|p| p.partner_index),
                s.race_credentials.clone(),
            )
        };

        // Снять адреса ступеней ПОКА РЕЗОЛВЕР СПОКОЕН.
        //
        // Как только поднимется туннель, macOS перетасует DNS-службы, и
        // запрос, попавший в этот момент, виснет на тринадцать секунд
        // (подтверждено системным журналом 22.09.2026). Здесь же, до подъёма,
        // это десятки миллисекунд.
        probe::pin_ladder_addresses().await;

        let raising = Instant::now();
        self.start_on(index, generation, false).await?;
        logger::log(
            "info",
            "vpn",
            &format!("движок и туннель подняты за {} мс", raising.elapsed().as_millis()),
        );
        if !self.is_current(generation).await {
            return Ok(());
        }

        self.set_step(VpnStep::Probing).await;
        // Открываем окно прогрева ДО первой пробы: если она не подтвердит
        // туннель, надзиратель переспросит через секунды, а не через пять
        // минут.
        self.session.lock().await.settling_until = Some(Instant::now() + SETTLING_WINDOW);

        // Ждём СОБЫТИЯ, а не времени: пока через туннель не пройдёт первое
        // соединение, проверять нечего. Оно же оплачивает весь холодный старт
        // - разрешение имени через DoH и сессию до узла, - поэтому проба сразу
        // за ним идёт уже по тёплому пути.
        let (warm_host, warm_port) = probe::warm_target();

        // Прогрев: первый настоящий обмен байтами с той стороной через узел.
        //
        // До 27.09.2026 он шёл в два захода ДРУГ ЗА ДРУГОМ и ПО ИМЕНИ: сперва
        // через SOCKS движка, потом через туннель. Имя xray разрешал через DoH
        // на узле прямо на критическом пути, и прогрев занимал от 2,2 до 7 с
        // из 8 до щита. Теперь - по адресу, снятому до подъёма туннеля, и
        // гонкой (ниже); резолвер прогревается отдельно, в фоне, когда щит уже
        // зелёный.
        let warm_ip = probe::pinned_ip(warm_host);
        // Гонка, а не очередь: щит ждёт ПЕРВОГО ответа, а не всех. Первое
        // соединение до узла обычно ~1,3 с, но изредка 6 с и больше (потеря
        // пакета на рукопожатии; замер 27.09.2026 на XHTTP через Амстердам),
        // и вторая попытка на своём соединении срезает такой хвост. Ответ
        // через туннель доказывает весь путь; ответ через SOCKS движка -
        // путь до узла, а туннель подтвердит надзиратель через секунды.
        let warm_started = Instant::now();
        let mut racers: tokio::task::JoinSet<(&'static str, bool)> = tokio::task::JoinSet::new();
        racers.spawn(async move {
            ("туннель", probe::warm_through_tunnel(warm_host, warm_ip, warm_port, WARM_BUDGET).await.is_some())
        });
        racers.spawn(async move {
            tokio::time::sleep(WARM_SECOND_RACER_AFTER).await;
            ("туннель, вторая попытка", probe::warm_through_tunnel(warm_host, warm_ip, warm_port, WARM_BUDGET).await.is_some())
        });
        #[cfg(desktop)]
        racers.spawn(async move {
            ("движок", probe::warm_through_socks(tun::SOCKS_PORT, warm_host, warm_ip, warm_port, WARM_BUDGET, None).await)
        });
        // The other transport, through its own port (Watafast): when it
        // answers first, the session moves to it below. Authenticated: this
        // inbound forwards straight to another country while the race lasts,
        // so the warm-up has to prove it is us before xray forwards anything.
        #[cfg(desktop)]
        let racing = race_partner.is_some() && race_credentials.is_some();
        #[cfg(desktop)]
        if let Some(creds) = &race_credentials {
            if race_partner.is_some() {
                let auth = Some((creds.user.clone(), creds.pass.clone()));
                racers.spawn(async move {
                    (
                        RACE_WINNER_PARTNER,
                        probe::warm_through_socks(
                            xray_manager::RACE_SOCKS_PORT,
                            warm_host,
                            warm_ip,
                            warm_port,
                            WARM_BUDGET,
                            auth,
                        )
                        .await,
                    )
                });
            }
        }
        #[cfg(not(desktop))]
        let racing = false;
        let mut winner: Option<&'static str> = None;
        while let Some(done) = racers.join_next().await {
            if let Ok((name, true)) = done {
                winner = Some(name);
                break;
            }
        }
        // Проигравшие больше не нужны: их соединения закрываются вместе с задачами.
        racers.abort_all();
        let warm_ms = warm_started.elapsed().as_millis();
        let warmed = winner.is_some();
        if let (Some(RACE_WINNER_PARTNER), Some(partner)) = (winner, race_partner) {
            // The other transport got through first: move the session to it,
            // tunnel kept (only the engine restarts, ~0.7 s). The chosen one
            // is not demoted: losing a race is not failing. `start_on` builds
            // a plain config for `partner` alone — `pending_race` was already
            // consumed by the FIRST `start_on` above, so no partner is passed
            // and the race inbound is gone with this restart.
            logger::log("info", "vpn", "гонку выиграл другой протокол, переходим на него");
            self.start_on(partner, generation, true).await?;
            if !self.is_current(generation).await {
                return Ok(());
            }
        } else if racing {
            // The race ran and the CHOSEN node won it (or nothing answered
            // inside the warm-up budget): either way, the config xray is
            // still running has the race inbound in it — an unauthenticated
            // forward would be bad enough, and even a password-protected one
            // has no reason to keep listening for the rest of the session.
            // Rebuild the same node's engine: `pending_race` is already
            // consumed, so this config-builds without a partner and the
            // inbound disappears with the restart, same as the branch above.
            logger::log("info", "vpn", "гонку выиграл выбранный узел, закрываем гоночный вход");
            self.start_on(index, generation, true).await?;
            if !self.is_current(generation).await {
                return Ok(());
            }
        }

        // Резолвер через узел - в фоне: первое имя, которое спросит человек,
        // не должно платить за холодный DoH, но и щит этого ждать не должен.
        #[cfg(desktop)]
        if warm_ip.is_some() {
            tauri::async_runtime::spawn(async move {
                let started = Instant::now();
                let ok = probe::warm_through_socks(
                    tun::SOCKS_PORT,
                    warm_host,
                    None,
                    warm_port,
                    WARM_BUDGET,
                    None,
                )
                .await;
                logger::log(
                    if ok { "info" } else { "warn" },
                    "tun",
                    &format!(
                        "резолвер через узел прогрет в фоне: {} за {} мс",
                        if ok { "ок" } else { "НЕ ПРОШЁЛ" },
                        started.elapsed().as_millis()
                    ),
                );
            });
        }

        logger::log(
            if warmed { "info" } else { "warn" },
            "tun",
            &match winner {
                Some(name) => format!("прогрев: первым ответил {name} за {warm_ms} мс"),
                None => format!("прогрев: НЕ ПРОШЁЛ ни один путь за {warm_ms} мс"),
            },
        );
        if warmed {
            // Прогрев через туннель - это уже настоящий обмен байтами с той
            // стороной через узел: TCP, запрос, ответ. Ждать ещё и пробу -
            // это две секунды сверху (замер 27.09.2026: 6,7 с до щита при
            // цели владельца 3-5). Щит зеленеет сразу, с пометкой
            // `green_unprobed`: пока она стоит и открыто окно SETTLING_WINDOW,
            // надзиратель переспрашивает с шагом прогрева (4 с), и если проба
            // не пройдёт, дальше работает обычная лестница.
            if !self.is_current(generation).await {
                return Ok(());
            }
            logger::log(
                "info",
                "vpn",
                &format!(
                    "защищено по прогреву; от начала подключения {} мс",
                    started.elapsed().as_millis()
                ),
            );
            let label = {
                let mut s = self.session.lock().await;
                s.green_unprobed = true;
                s.location.clone()
            };
            self.note(TimelineCode::Connected, label).await;
            self.set_phase(VpnPhase::On, None).await;
            self.spawn_supervisor(generation);
            return Ok(());
        }
        // Прогрев не вышел - не беда: даём прежнюю фиксированную паузу,
        // чтобы не броситься проверять совсем уж мгновенно.
        tokio::time::sleep(FIRST_PROBE_DELAY).await;
        // `AfterConnect` has no floor, so this always asks: it is the probe
        // that decides whether the shield turns green at all.
        let verdict = self
            .probe(ProbeReason::AfterConnect)
            .await
            .unwrap_or(ProbeVerdict::Unconfirmed);
        logger::log(
            if matches!(verdict, ProbeVerdict::Passed) { "info" } else { "warn" },
            "vpn",
            &format!(
                "проверка после подключения: {verdict:?}; от начала подключения {} мс",
                started.elapsed().as_millis()
            ),
        );
        if !self.is_current(generation).await {
            return Ok(());
        }

        match verdict {
            ProbeVerdict::Passed | ProbeVerdict::Unconfirmed => {
                let label = self.session.lock().await.location.clone();
                self.note(TimelineCode::Connected, label).await;
                // Состояние показываем сразу, каким бы оно ни было.
                //
                // Пробовали иначе - держать окно на «Включаем...», пока идёт
                // прогрев. Вышло хуже: спиннер крутился больше минуты, и это
                // враньё оказалось неприятнее честного «поднято, но не
                // подтверждено». Тем более что кнопка «Выключить» под ним
                // работает, а туннель к этому моменту уже несёт трафик.
                // Прогрев продолжает переспрашивать в фоне и сам переведёт
                // щит в зелёное, когда проверка пройдёт.
                self.set_phase(verdict.phase(), None).await;
                self.spawn_supervisor(generation);
                Ok(())
            }
            ProbeVerdict::NetworkOffline => Err(AppError::new(ErrorCode::NetworkOffline)),
            // Up but not carrying: this is what the ladder exists for, and the
            // person is told nothing at all for the first seconds. The ladder
            // is SPAWNED rather than awaited so this call returns and releases
            // the operation lock — otherwise "Отмена" would wait out a repair
            // that can run for the better part of a minute.
            _ => {
                let cause = verdict
                    .as_error()
                    .unwrap_or_else(|| AppError::new(ErrorCode::Unknown));
                self.spawn_supervisor(generation);
                self.spawn_heal(generation, cause);
                Ok(())
            }
        }
    }

    /// Fetch the subscription and take everything it says into the session.
    async fn refresh_subscription(self: &Arc<Self>) -> Result<(), AppError> {
        let (link, in_russia) = {
            let s = self.session.lock().await;
            (load_link(), s.routing_in_russia)
        };
        let link = link.ok_or_else(|| AppError::new(ErrorCode::NoSubscription))?;
        let url = effective_sub_url(&link, in_russia)?;

        let lang_en = self.ui_lang_en.load(std::sync::atomic::Ordering::Relaxed);
        let sub = fetch_subscription(&url, lang_en).await?;
        if sub.servers.is_empty() {
            return Err(AppError::new(ErrorCode::SubEmpty));
        }

        // Teach the redactor every node name BEFORE anything can log one. A
        // numeric address is masked structurally; a host name is only masked
        // once this has happened.
        for server in &sub.servers {
            logger::remember_node_host(server.host());
        }

        // "Fallback" means we left the address in the person's own link, not
        // that we left `proxysvpn.com`: a link issued on a reserve site is
        // somebody's normal, and calling it a fallback would be noise.
        let home = Url::parse(&link)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| SITE_LADDER[0].to_string());
        let used_fallback = !sub.source_host.eq_ignore_ascii_case(&home);
        let unreadable = sub.unreadable_lines;
        {
            let mut s = self.session.lock().await;
            s.servers = sub.servers;
            s.meta = sub.meta;
            s.sub_fetched_at = Some(now_ms());
            s.sub_reuse_blocked = false;
            s.sub_used_fallback = used_fallback;
            s.sub_source_host = Some(sub.source_host);
            s.push_event(
                if used_fallback {
                    TimelineCode::SubRefreshedFallback
                } else {
                    TimelineCode::SubRefreshed
                },
                None,
            );
            if unreadable > 0 {
                // A line we could not read never disappears in silence: the
                // list would simply be shorter and nobody would know why.
                s.push_event(TimelineCode::LocationUnreadable, None);
            }
        }
        self.emit_meta().await;

        // Список стран сменился - значит прежние замеры относятся уже не к
        // тем узлам. Меряем заново, ФОНОМ: обновление подписки не должно
        // ждать двенадцати рукопожатий, а автовыбор к моменту нажатия
        // «Включить» уже будет знать, кто быстрее.
        let core = self.clone();
        tauri::async_runtime::spawn(async move {
            core.measure_all_rtt().await;
        });
        Ok(())
    }

    /// Put the chain on one node and record that we did.
    async fn start_on(
        self: &Arc<Self>,
        index: usize,
        generation: u64,
        keep_tunnel: bool,
    ) -> Result<(), AppError> {
        let server = {
            let s = self.session.lock().await;
            s.servers
                .get(index)
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::SubEmpty))?
        };

        let (_, parsed_label, note) = split_label(server.remark());
        let note = subscription::display_note(note);
        let label = if parsed_label.is_empty() {
            fallback_label(index)
        } else {
            parsed_label
        };
        // Which location and protocol, never the address. Without this line
        // the log of 27.09.2026 could not say which of two «Британия» rows the
        // person was on when "Claude stopped thinking".
        logger::log(
            "info",
            "vpn",
            &format!(
                "узел: {label}{} · {}{}",
                note.as_deref().map(|n| format!(" · {n}")).unwrap_or_default(),
                server.protocol_label(),
                if keep_tunnel { " (туннель не трогаем)" } else { "" }
            ),
        );

        self.set_step(VpnStep::StartingEngine).await;
        // `take_race_partner` only yields something when THIS call's own
        // `(generation, index)` is the exact one the race was computed for
        // (connect_inner, right before its own first `start_on`). Every other
        // caller — the repair ladder, a network-change rebuild, the "close
        // the race" restart below — gets `None` and builds a plain config,
        // which is what actually removes the race inbound once the race is
        // decided.
        let (partner, race_creds) = {
            let mut s = self.session.lock().await;
            let partner_index = s.take_race_partner(generation, index);
            let partner = partner_index.and_then(|i| s.servers.get(i).cloned());
            let creds = if partner.is_some() { s.race_credentials.clone() } else { None };
            (partner, creds)
        };
        self.engine_start(&server, partner.as_ref(), race_creds.as_ref()).await?;
        if !self.is_current(generation).await {
            return Ok(());
        }
        if !keep_tunnel {
            // The step that asks for the administrator password, and the only
            // one worth naming separately to a person who is waiting.
            self.set_step(VpnStep::RaisingTun).await;
        }
        self.tunnel_up(&server, keep_tunnel).await?;
        if !self.is_current(generation).await {
            return Ok(());
        }

        // The measurement module is told what it is looking at so a UDP node
        // answers "не применимо" at once instead of timing out twice.
        ping::set_target_proto(server.host().to_string(), server.port(), server.proto());

        let mut s = self.session.lock().await;
        s.current = Some(index);
        s.location = Some(label);
        s.proto = Some(server.proto().to_string());
        s.remember_use(index);
        s.meter.reset();
        Ok(())
    }

    async fn disconnect(self: &Arc<Self>) {
        logger::log("info", "vpn", "отключение по кнопке");
        // Снятые адреса живут ровно столько, сколько соединение: держать их
        // дольше - значит однажды пойти по устаревшему.
        probe::forget_pinned_addresses();
        // Bumped BEFORE the lock is taken: a connect or a repair in flight
        // notices at its next checkpoint and stops, so "Отмена" is felt in a
        // second rather than after the ladder finishes.
        self.bump_generation().await;
        let _op = self.operation.lock().await;
        self.engine_down().await;
        {
            let mut s = self.session.lock().await;
            s.current = None;
            s.green_unprobed = false;
            s.metric = MetricPayload::default();
            s.meter.reset();
            s.gate.reset();
            s.push_event(TimelineCode::Disconnected, None);
        }
        self.emit_metric().await;
        self.set_phase(VpnPhase::Off, None).await;
    }
}

// ───────────────────────────────────────────────────────────────────────────
// The probe: the only thing that grants green
// ───────────────────────────────────────────────────────────────────────────

impl Core {
    /// `None` means we did not ask — the gate said it was too soon. That is
    /// deliberately not a verdict: inventing one would let the repair ladder
    /// fire on an answer nobody gave.
    async fn probe(&self, reason: ProbeReason) -> Option<ProbeVerdict> {
        let allowed = {
            let s = self.session.lock().await;
            s.gate.allows(reason, now_ms())
        };
        if !allowed {
            return None;
        }

        let mut meter = { self.session.lock().await.meter };
        let (rx_before_probe, tx_before_probe) = meter.totals();
        // Two addresses, not four: the first probe of a session decides how
        // long the person waits for green, and the rest of the ladder only
        // matters once the first has actually failed.
        let ladder = match reason {
            // Первая проба идёт по своей лестнице: там тот же адрес по
            // открытому и по защищённому порту, наперегонки. Подробности - у
            // FIRST_PROBE_LADDER.
            ProbeReason::AfterConnect | ProbeReason::Settling => probe::FIRST_PROBE_LADDER,
            _ => probe::PROBE_LADDER,
        };
        // Холодному туннелю - холодное терпение.
        let patience = match reason {
            ProbeReason::AfterConnect | ProbeReason::Settling => COLD_PROBE_TIMEOUT,
            _ => PROBE_TIMEOUT,
        };
        // Первая проба спрашивает адреса РАЗОМ: один мёртвый адрес не должен
        // съедать весь бюджет до того, как спросят живого соседа. Установившийся
        // ход остаётся очередью - там спешить некуда.
        let race = matches!(
            reason,
            ProbeReason::AfterConnect | ProbeReason::Settling
        );
        let report = probe::probe_once_racing(&mut meter, ladder, patience, race).await;

        let verdict = self.interpret(report.verdict).await;
        if verdict != ProbeVerdict::Passed {
            // Приговор и обе половины улики в одной строке. Именно дельты
            // отличают «ушло и не вернулось» от «ничего не ушло», и без них
            // по журналу не отличить задушенный туннель от мёртвого адреса.
            logger::log(
                "warn",
                "probe",
                &format!(
                    "{verdict:?}: отдано {} Б, принято {} Б за пробу, адресов в лестнице {}",
                    report.tx_bytes.saturating_sub(tx_before_probe),
                    report.rx_bytes.saturating_sub(rx_before_probe),
                    ladder.len()
                ),
            );
        }
        // Число, которое видит человек, - это рукопожатие TCP до узла, а не
        // длительность проверочного запроса.
        //
        // Раньше показывали `report.rtt_ms` - время всего GET через туннель:
        // разрешение имени, три рукопожатия, ответ страницы. На холодном
        // туннеле это 3814 мс, и окно честно писало «3814 мс» под словом
        // «Защищено». Человек читает это как задержку до сервера и решает,
        // что сервис никуда не годится, хотя настоящая задержка до узла в
        // десятки раз меньше.
        //
        // Модуль `ping` для этого и написан, и он же знает, что у hysteria
        // рукопожатия TCP не бывает: там ответ - «не применимо», и число не
        // показывается вовсе. Это честнее выдуманной цифры.
        let shown_rtt = if verdict == ProbeVerdict::Passed {
            ping::tcp_ping_async().await.ok()
        } else {
            None
        };
        {
            let mut s = self.session.lock().await;
            s.meter = meter;
            s.gate.mark(now_ms());
            let mut metric = report.metric();
            metric.rtt_ms = shown_rtt;
            metric.quality = Some(match shown_rtt {
                Some(ms) => LinkQuality::from_rtt_ms(ms),
                None => LinkQuality::Unknown,
            });
            s.absorb_metric(metric);
            if verdict == ProbeVerdict::Passed {
                s.rx_stalled_since = None;
                // Вот теперь узел действительно рабочий, и его стоит помнить -
                // и вообще, и для этой сети.
                if let Some(index) = s.current {
                    s.last_good = s.id_of(index);
                    if let (Some(network), Some(id)) = (s.network.clone(), s.id_of(index)) {
                        if s.net_memory.winner(&network, now_ms()) != Some(id.as_str()) {
                            s.net_memory.remember(&network, &id, now_ms());
                            let snapshot = s.net_memory.clone();
                            std::thread::spawn(move || netmem::store(&snapshot));
                        }
                    }
                }
                if let Some(index) = s.current {
                    if let (Some(id), Some(rtt)) = (s.id_of(index), shown_rtt) {
                        // The one number we trust: measured through the tunnel
                        // we are actually using, so it is comparable with
                        // itself and with nothing else.
                        s.quality
                            .insert(id, LocationQuality::from_link(LinkQuality::from_rtt_ms(rtt)));
                    }
                }
            }
        }
        self.emit_metric().await;
        Some(verdict)
    }

    /// Померить рукопожатие TCP до каждого узла и запомнить.
    ///
    /// Нужно двоим: списку стран (там числа видит человек) и автовыбору (там
    /// они решают, за какую страну браться). Ходит параллельно - двенадцать
    /// узлов по очереди сложились бы в двенадцать секунд.
    ///
    /// Ни одного запроса к нашим сайтам и ни одного байта в туннель: только
    /// рукопожатие до узла и сразу разрыв. Поэтому это можно делать и в фоне,
    /// не боясь ни нагрузки на витрину, ни узнаваемого ритма в сети.
    async fn measure_all_rtt(&self) {
        // Снимок того, что меряем: держать замок сессии через сеть нельзя.
        let targets: Vec<(String, String, u16, String)> = {
            let s = self.session.lock().await;
            s.locations()
                .into_iter()
                .zip(s.servers.iter())
                .map(|(entry, server)| {
                    (
                        entry.id,
                        server.host().to_string(),
                        server.port(),
                        server.proto().to_string(),
                    )
                })
                .collect()
        };
        if targets.is_empty() {
            return;
        }

        let mut jobs = tokio::task::JoinSet::new();
        for (id, host, port, proto) in targets {
            // Рукопожатие блокирующее, поэтому уходит в отдельный поток.
            jobs.spawn_blocking(move || (id, ping::rtt_of(&host, port, &proto)));
        }
        let mut measured: HashMap<String, u32> = HashMap::new();
        while let Some(done) = jobs.join_next().await {
            if let Ok((id, Some(ms))) = done {
                measured.insert(id, ms);
            }
        }

        // Заменяем целиком: узел, переставший отвечать, обязан ПОТЕРЯТЬ прежнее
        // число, а не показывать вчерашнее как сегодняшнее.
        self.session.lock().await.rtt = measured;
    }

    /// Platform correction for a verdict built from interface counters.
    #[cfg(desktop)]
    async fn interpret(&self, verdict: ProbeVerdict) -> ProbeVerdict {
        verdict
    }

    /// On iOS the tunnel device belongs to the Network Extension and does not
    /// carry our name, so the passive counters are simply absent. Without them
    /// `classify` reads "nothing left the interface" and answers `NoRoute`,
    /// which on a working iPhone would be a lie. When the extension says it is
    /// connected and we have no counters, the honest word is "unconfirmed".
    #[cfg(target_os = "ios")]
    async fn interpret(&self, verdict: ProbeVerdict) -> ProbeVerdict {
        if verdict == ProbeVerdict::NoRoute
            && probe::tunnel_counters().is_none()
            && ios_vpn::is_connected_wait().await
        {
            return ProbeVerdict::Unconfirmed;
        }
        verdict
    }
}

// ───────────────────────────────────────────────────────────────────────────
// The supervisor: the watchdog that was given a voice
// ───────────────────────────────────────────────────────────────────────────

impl Core {
    fn spawn_supervisor(self: &Arc<Self>, generation: u64) {
        let core = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            core.supervise(generation).await;
        });
    }

    async fn supervise(self: Arc<Self>, generation: u64) {
        let mut last_tick = Instant::now();
        #[cfg(desktop)]
        let mut last_routes = Instant::now();
        let mut had_link = probe::has_usable_link();
        loop {
            tokio::time::sleep(SUPERVISOR_TICK).await;
            if !self.is_current(generation).await {
                return;
            }
            // A tick that took far longer than we asked for means the machine
            // was asleep. Waking is one of the few moments worth an immediate
            // probe: the network is usually a different one.
            let slept = last_tick.elapsed() > SUPERVISOR_TICK * 8;
            last_tick = Instant::now();
            let link = probe::has_usable_link();
            let link_returned = link && !had_link;
            had_link = link;

            let phase = self.session.lock().await.phase;
            if phase == VpnPhase::Off {
                return;
            }
            if phase == VpnPhase::Failed {
                // Waiting mode (design M3): the ladder is spent, the screen
                // says so, and we do not hammer a network that is not there.
                // A wake or a network that came back is worth one more try,
                // and both are answered from the interface list — no traffic,
                // no requests, nothing anybody can throttle.
                if (slept || link_returned) && self.may_retry().await {
                    let (cause, has_node) = {
                        let s = self.session.lock().await;
                        (s.error.clone(), s.current.is_some())
                    };
                    if has_node {
                        self.heal(
                            generation,
                            cause.unwrap_or_else(|| AppError::new(ErrorCode::Unknown)),
                        )
                        .await;
                    } else {
                        // We never got as far as a node, so there is nothing
                        // to repair — start over. `connect` opens a new
                        // generation, which retires this loop.
                        let _ = self.connect().await;
                        return;
                    }
                }
                continue;
            }
            if phase == VpnPhase::Healing {
                // A ladder started elsewhere is running; it pulses its own
                // state, and there is nothing here to add.
                continue;
            }

            self.passive_tick().await;

            if let Some(dead) = self.dead_engine().await {
                logger::log("error", "app", &format!("{dead} is not running"));
                // Handled: the person is not being asked to do anything, and
                // a toast about a repair we are already performing is noise.
                self.emit_event(AppError::new(ErrorCode::EngineDied), true);
                self.revive(generation).await;
                continue;
            }

            // Route maintenance spawns two or three processes, so it keeps
            // the five-second cadence the old watchdog ran at rather than the
            // two-second one the free passive counters can afford.
            #[cfg(desktop)]
            if last_routes.elapsed() >= ROUTE_CHECK_EVERY {
                last_routes = Instant::now();
                if self.mend_routes(generation).await {
                    continue;
                }
            }

            if let Some(reason) = self.probe_reason(slept).await {
                let Some(verdict) = self.probe(reason).await else {
                    continue;
                };
                if !self.is_current(generation).await {
                    return;
                }
                // Any answer ends "green by the warm-up alone": from here the
                // shield stands on a probe, whatever it said.
                self.session.lock().await.green_unprobed = false;
                match verdict {
                    ProbeVerdict::Passed => self.settle(VpnPhase::On).await,
                    ProbeVerdict::Unconfirmed => self.settle(VpnPhase::Unconfirmed).await,
                    ProbeVerdict::NetworkOffline => {
                        // Nothing of ours is broken. Saying so plainly is the
                        // difference between a support ticket and a shrug.
                        self.set_phase(
                            VpnPhase::Failed,
                            Some(AppError::new(ErrorCode::NetworkOffline)),
                        )
                        .await;
                        return;
                    }
                    other => {
                        self.note(TimelineCode::TrafficStopped, None).await;
                        let cause = other
                            .as_error()
                            .unwrap_or_else(|| AppError::new(ErrorCode::Unknown));
                        self.heal(generation, cause).await;
                        // The ladder either recovered or ended in Failed; the
                        // next tick reads the phase and leaves if it is over.
                    }
                }
            }
        }
    }

    /// Is another repair pass allowed yet?
    async fn may_retry(&self) -> bool {
        let s = self.session.lock().await;
        s.heal_blocked_until
            .is_none_or(|until| until <= Instant::now())
    }

    /// Update the phase without disturbing anything else.
    async fn settle(&self, phase: VpnPhase) {
        let changed = {
            let s = self.session.lock().await;
            s.phase != phase
        };
        if changed {
            self.set_phase(phase, None).await;
        }
    }

    /// The free half of the pulse: interface counters, no requests at all.
    async fn passive_tick(&self) {
        let Some(sample) = probe::tunnel_counters() else {
            return;
        };
        let mut s = self.session.lock().await;
        let (rx_before, _) = s.meter.totals();
        let (rx, tx) = s.meter.observe(sample);
        s.metric.rx_bytes = rx;
        s.metric.tx_bytes = tx;

        // Bytes going out and nothing coming back is the shape of a throttled
        // link. It never decides anything by itself — it buys a probe.
        if tx > 0 && rx == rx_before {
            s.rx_stalled_since.get_or_insert_with(Instant::now);
        } else {
            s.rx_stalled_since = None;
        }
    }

    /// Why we would probe right now, if at all.
    async fn probe_reason(&self, woke: bool) -> Option<ProbeReason> {
        let stalled = {
            let s = self.session.lock().await;
            s.rx_stalled_since
                .is_some_and(|since| since.elapsed() >= RX_STALL)
        };
        // Туннель встал, но первая проба его не подтвердила. Пока попытки
        // прогрева не исчерпаны, переспрашиваем через секунды: иначе один
        // холодный промах прибивает окно к «Подтвердить не удалось» на пять
        // минут поверх работающей защиты.
        let settling = {
            let s = self.session.lock().await;
            settling_applies(s.phase, s.green_unprobed, s.settling_until, Instant::now())
        };
        let reason = if woke {
            ProbeReason::Woke
        } else if settling {
            ProbeReason::Settling
        } else if stalled {
            ProbeReason::Suspicion
        } else if self.window_visible() {
            ProbeReason::IdleForeground
        } else {
            ProbeReason::IdleBackground
        };
        let allowed = {
            let s = self.session.lock().await;
            s.gate.allows(reason, now_ms())
        };
        allowed.then_some(reason)
    }

    fn window_visible(&self) -> bool {
        self.app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false)
    }

    /// Put back routes the system took away; rebuild the engine when the
    /// machine changed how it reaches the internet.
    ///
    /// Returns true when it did something that makes this tick's probe moot.
    #[cfg(desktop)]
    async fn mend_routes(self: &Arc<Self>, generation: u64) -> bool {
        let repair = tun::ensure_routes(&self.tun).await;
        if repair.is_clean() {
            return false;
        }
        if repair.network_changed {
            self.note(TimelineCode::NetworkChanged, None).await;
            // `s.network` was fingerprinted for the OLD network at connect and
            // never touched since. Left alone, a probe's next `Passed`
            // verdict would remember the winning node under that stale id,
            // and `demote` would later forget the wrong network's winner.
            // Recompute it here, off the clean-repair hot path — this branch
            // only runs when the network itself actually changed — and store
            // `None` when it cannot be read, so neither remember nor forget
            // touches the memory until a real network is known again.
            let fresh_network = netmem::current_network().await;
            {
                let mut s = self.session.lock().await;
                s.network = fresh_network;
            }
            // The engine's sockets are pinned to the interface that existed a
            // moment ago. On a new one they would fail every connection, so
            // the config is rebuilt — TUN and routes are untouched.
            let index = self.session.lock().await.current;
            if let Some(index) = index {
                if let Err(err) = self.start_on(index, generation, true).await {
                    self.heal(generation, err).await;
                    return true;
                }
            }
            let _ = self.probe(ProbeReason::NetworkChanged).await;
            return true;
        }
        // Routes were repaired in place; the next tick decides whether traffic
        // followed them back.
        true
    }

    /// An engine died. Three attempts with the pauses the design names.
    async fn revive(self: &Arc<Self>, generation: u64) {
        self.set_phase(VpnPhase::Healing, Some(AppError::new(ErrorCode::EngineDied)))
            .await;

        for pause in ENGINE_RESTART_PAUSES {
            tokio::time::sleep(pause).await;
            if !self.is_current(generation).await {
                return;
            }
            if self.restart_dead(generation).await.is_ok() && self.engines_alive().await {
                if let Some(phase) = self.try_probe(generation).await {
                    let label = self.session.lock().await.location.clone();
                    self.note(TimelineCode::Healed, label).await;
                    self.settle(phase).await;
                    return;
                }
            }
        }
        // The engine will not stay up on this node. The full ladder knows more
        // tricks than "start it again".
        self.heal(generation, AppError::new(ErrorCode::EngineDied))
            .await;
    }

    #[cfg(desktop)]
    async fn restart_dead(self: &Arc<Self>, generation: u64) -> Result<(), AppError> {
        let dead = self.dead_engine().await;
        match dead {
            Some("tun2socks") => tun::restart_engine(&self.tun, &self.app).await,
            Some(_) => {
                let index = self
                    .session
                    .lock()
                    .await
                    .current
                    .ok_or_else(|| AppError::new(ErrorCode::EngineDied))?;
                // Keep the tunnel: only what is behind the SOCKS port changes.
                self.start_on(index, generation, true).await
            }
            None => Ok(()),
        }
    }

    #[cfg(target_os = "ios")]
    async fn restart_dead(self: &Arc<Self>, generation: u64) -> Result<(), AppError> {
        let index = self
            .session
            .lock()
            .await
            .current
            .ok_or_else(|| AppError::new(ErrorCode::EngineDied))?;
        self.start_on(index, generation, false).await
    }
}

// ───────────────────────────────────────────────────────────────────────────
// The repair ladder (design M3)
//
// Silent for the first seconds, a whisper while it works, a sentence only when
// it runs out. The window decides which of the three from `healingForMs`, so
// the core's job is to keep that clock honest and to keep the routes up the
// whole way: a repair that tears the tunnel down is indistinguishable from
// "the internet went away", which is the thing we are repairing.
// ───────────────────────────────────────────────────────────────────────────

impl Core {
    async fn heal(self: &Arc<Self>, generation: u64, cause: AppError) {
        {
            let s = self.session.lock().await;
            if s.heal_blocked_until.is_some_and(|until| until > Instant::now()) {
                // A ladder every few seconds is a way to keep a broken network
                // busy, not a way to fix it.
                drop(s);
                // `heal` is also reached fire-and-forget, from `spawn_heal`
                // (`connect_inner`'s "up but not carrying" branch returns
                // `Ok(())` and lets the operation lock go BEFORE this task
                // runs) — by the time it gets here, a newer `connect` or
                // `disconnect` can already own the phase for a generation of
                // its own. Without this check a stale ladder's "Failed"
                // could land on top of that newer attempt's own state.
                if self.is_current(generation).await {
                    self.set_phase(VpnPhase::Failed, Some(cause)).await;
                }
                return;
            }
        }
        if !self.is_current(generation).await {
            return;
        }
        self.set_phase(VpnPhase::Healing, Some(cause.clone())).await;
        self.spawn_healing_pulse(generation);

        let outcome = self.climb(generation, &cause).await;
        if !self.is_current(generation).await {
            return;
        }
        match outcome {
            Some(phase) => {
                let label = self.session.lock().await.location.clone();
                self.note(TimelineCode::Healed, label).await;
                {
                    let mut s = self.session.lock().await;
                    s.heal_rounds = 0;
                    s.heal_blocked_until = None;
                }
                self.set_phase(phase, None).await;
            }
            None => {
                {
                    let mut s = self.session.lock().await;
                    let wait = heal_backoff(s.heal_rounds);
                    s.heal_rounds += 1;
                    s.heal_blocked_until = Some(Instant::now() + wait);
                    s.push_event(TimelineCode::Failed, None);
                }
                self.set_phase(VpnPhase::Failed, Some(cause)).await;
            }
        }
    }

    /// One pass of the ladder. `Some(phase)` when something worked.
    async fn climb(self: &Arc<Self>, generation: u64, cause: &AppError) -> Option<VpnPhase> {
        // C0 — the shake. Two more looks before anything is touched: most
        // "outages" are a lost second on a mobile network.
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(phase) = self.try_probe(generation).await {
            return Some(phase);
        }

        let current = self.session.lock().await.current?;

        // C1 — same node, engine restarted. TUN and routes untouched while
        // they are there; raised again when tun2socks took them down with it.
        let restarted = {
            let _step = self.repair_step(generation).await?;
            let keep = self.tunnel_is_up().await;
            self.start_on(current, generation, keep).await.is_ok()
        };
        if restarted {
            if let Some(phase) = self.try_probe(generation).await {
                return Some(phase);
            }
        }
        if !self.is_current(generation).await {
            return None;
        }

        // C2 — another node, same protocol. The old one is marked as "does not
        // get through on this network" rather than as broken.
        self.session.lock().await.demote(current);
        // Bound first, so the guard is gone before `switch_to`: a guard in an
        // `if let` scrutinee lives to the end of the block, and `start_on`
        // takes the same lock. Written inline, this step hung the whole core
        // (found 01.10.2026 on the Android emulator: every command waiting on
        // the session, the shield frozen on "healing").
        let next = self.session.lock().await.next_server(current, true);
        if let Some(next) = next {
            if self.repair_switch(next, generation).await? {
                if let Some(phase) = self.try_probe(generation).await {
                    return Some(phase);
                }
                self.session.lock().await.demote(next);
            }
        }
        if !self.is_current(generation).await {
            return None;
        }

        // C3 — the other protocol, but only on the signature that calls for
        // it: bytes leave and nothing returns is how a provider strangles a
        // UDP stream, and TCP is the answer to exactly that.
        if cause.code == ErrorCode::Blocked {
            // Bound first, for the reason C2 gives.
            let other = self.session.lock().await.next_server(current, false);
            if let Some(other) = other {
                if self.repair_switch(other, generation).await? {
                    if let Some(phase) = self.try_probe(generation).await {
                        return Some(phase);
                    }
                }
            }
        }
        if !self.is_current(generation).await {
            return None;
        }

        // C4 — the addresses may simply have been burned. Re-read the
        // subscription over the ladder of sites and try the best of what comes
        // back. This is the step that closes "the server died" tickets.
        if self.refresh_subscription().await.is_ok() {
            let next = self.session.lock().await.choose_server(tunnel_prefs::load().transport);
            if let Some(next) = next {
                if self.repair_switch(next, generation).await? {
                    if let Some(phase) = self.try_probe(generation).await {
                        return Some(phase);
                    }
                }
            }
        }

        // C5 would ask the service (`/api/vpn/check` → `/api/vpn/fix`). It is
        // not here because both endpoints authorise a cabinet session and we
        // hold a subscription token; see DESIGN.md M6. Pretending to try it
        // would only spend the person's time.
        None
    }

    /// The operation lock for one engine-touching step of a repair, or `None`
    /// when this ladder was retired while it waited for it.
    ///
    /// Not held for the whole climb — a person pressing "Cancel" on step four
    /// must not wait for the ladder to finish — only around each restart, so
    /// a location chosen by hand (`select_location`, which holds this lock
    /// across its own switch and then retires the ladder) and a repair never
    /// restart the engines at the same time.
    async fn repair_step(&self, generation: u64) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        let guard = self.operation.lock().await;
        if !self.is_current(generation).await {
            return None;
        }
        Some(guard)
    }

    /// `switch_to` as a repair step: `None` when the ladder was retired,
    /// otherwise whether the switch worked.
    async fn repair_switch(self: &Arc<Self>, index: usize, generation: u64) -> Option<bool> {
        let _step = self.repair_step(generation).await?;
        Some(self.switch_to(index, generation).await.is_ok())
    }

    async fn try_probe(self: &Arc<Self>, generation: u64) -> Option<VpnPhase> {
        if !self.is_current(generation).await {
            return None;
        }
        // The probe dials with `no_proxy` over the system routes, so it says
        // nothing about the tunnel unless the tunnel is there to take it. When
        // tun2socks has died the device and both half-defaults go with it, the
        // probe reaches the site straight from the real address, and C0 used
        // to call that "Protection restored" while every byte went outside the
        // VPN. Desktop only: on iOS the extension owns the tunnel and
        // `tunnel_is_up` is always false by design.
        #[cfg(desktop)]
        let carrying = self.tunnel_is_up().await && self.engines_alive().await;
        #[cfg(target_os = "ios")]
        let carrying = true;
        if !carrying {
            logger::log("info", "vpn", "проверка не засчитана: туннель не поднят");
        }
        healing_verdict(self.probe(ProbeReason::AfterConnect).await, carrying)
    }

    /// Keep `healingForMs` moving for as long as the ladder runs.
    ///
    /// The window turns that one number into silence (under 8 s), a whisper
    /// (under 45 s) and finally a sentence. A ladder that climbs for half a
    /// minute without emitting would leave the screen frozen on the first of
    /// the three, which is the same stale-state bug in a new place.
    fn spawn_healing_pulse(self: &Arc<Self>, generation: u64) {
        let core = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if !core.is_current(generation).await {
                    return;
                }
                if core.session.lock().await.phase != VpnPhase::Healing {
                    return;
                }
                core.emit_state().await;
            }
        });
    }

    /// Run the ladder off to one side.
    ///
    /// Repair must not hold the operation lock: a person pressing "Отмена"
    /// while we are on step four would otherwise wait for the whole climb.
    fn spawn_heal(self: &Arc<Self>, generation: u64, cause: AppError) {
        let core = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            core.heal(generation, cause).await;
        });
    }

    /// Move the live tunnel to another node without dropping it.
    async fn switch_to(self: &Arc<Self>, index: usize, generation: u64) -> Result<(), AppError> {
        let keep = self.tunnel_is_up().await;
        self.start_on(index, generation, keep).await?;
        let label = self.session.lock().await.location.clone();
        self.note(TimelineCode::LocationSwitched, label).await;
        Ok(())
    }

    #[cfg(desktop)]
    async fn tunnel_is_up(&self) -> bool {
        tun::is_running(&self.tun).await
    }

    #[cfg(target_os = "ios")]
    async fn tunnel_is_up(&self) -> bool {
        // The extension rebuilds its own tunnel from a new config, so there is
        // nothing to keep.
        false
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Commands
//
// Every one of them answers `Result<T, String>` with `AppError::to_payload()`
// in the error, which `parseAppError` in src/types.ts turns back into a code.
// A prefix like `format!("subscription: {e}")` would collapse every code into
// UNKNOWN, so the payload travels untouched.
// ───────────────────────────────────────────────────────────────────────────

type Cmd<T> = Result<T, String>;

#[tauri::command]
async fn vpn_snapshot(core: tauri::State<'_, Arc<Core>>) -> Cmd<VpnSnapshot> {
    Ok(core.session.lock().await.snapshot())
}

#[tauri::command]
async fn vpn_connect(core: tauri::State<'_, Arc<Core>>) -> Cmd<()> {
    let core = core.inner().clone();
    core.connect().await.map_err(|e| e.to_payload())
}

#[tauri::command]
async fn vpn_disconnect(core: tauri::State<'_, Arc<Core>>) -> Cmd<()> {
    let core = core.inner().clone();
    core.disconnect().await;
    Ok(())
}

/// Legacy single-bool status, kept because the tray and any old caller ask it.
///
/// It now counts the ACTIVE chain rather than xray alone: on a Hysteria2 node
/// the old answer was permanently `false`, because xray was never started.
#[tauri::command]
async fn vpn_status(core: tauri::State<'_, Arc<Core>>) -> Cmd<bool> {
    let phase = core.session.lock().await.phase;
    Ok(matches!(phase, VpnPhase::On | VpnPhase::Unconfirmed) && core.engines_alive().await)
}

#[tauri::command]
async fn vpn_ping() -> Cmd<ping::PingPayload> {
    Ok(ping::measure_async().await)
}

#[tauri::command]
async fn list_locations(core: tauri::State<'_, Arc<Core>>) -> Cmd<Vec<LocationEntry>> {
    {
        let s = core.session.lock().await;
        if !s.servers.is_empty() {
            return Ok(s.locations());
        }
    }
    // Nothing cached: the list is worth a fetch, and the honest failure of
    // that fetch is the honest failure of the screen.
    let core = core.inner().clone();
    core.refresh_subscription()
        .await
        .map_err(|e| e.to_payload())?;
    let list = core.session.lock().await.locations();
    Ok(list)
}

/// Померить рукопожатие TCP до КАЖДОЙ локации и вернуть список с числами.
///
/// Отдельной командой, а не внутри `list_locations`: список должен открыться
/// мгновенно, со словами, а числа приезжают следом. Замер идёт параллельно -
/// последовательно двенадцать узлов по секунде складывались бы в двенадцать
/// секунд ожидания.
///
/// У локаций на hysteria числа не будет никогда: там порт UDP, и рукопожатию
/// TCP стучать некуда. Это не поломка, и рисовать вместо него ноль или
/// «ошибка» нельзя - именно так когда-то и родился вечный спиннер.
/// Настройки туннеля, как они сейчас записаны.
#[tauri::command]
async fn tunnel_prefs_get() -> Cmd<tunnel_prefs::TunnelPrefs> {
    Ok(tunnel_prefs::load())
}

/// Записать настройки туннеля и, если туннель поднят, ПРИМЕНИТЬ их.
///
/// Применяем сразу, а не «при следующем подключении». Настройка, которая
/// молча ничего не делает до перезапуска, - это половина правды: человек
/// щёлкнул тумблер, увидел его включённым и вправе считать, что он работает.
///
/// Переподключение честно показывается окном: оно и так умеет рисовать
/// «Включаем...».
#[tauri::command]
async fn tunnel_prefs_set(
    core: tauri::State<'_, Arc<Core>>,
    prefs: tunnel_prefs::TunnelPrefs,
) -> Cmd<bool> {
    if !tunnel_prefs::store(&prefs) {
        return Err(AppError::new(ErrorCode::Unknown).to_payload());
    }
    Ok(reconnect_if_live(core.inner().clone()).await)
}

/// Общий хвост для любой команды настроек, которая уже сохранила себя на
/// диск и должна применить изменение НЕМЕДЛЕННО, если туннель поднят - вместо
/// того чтобы промолчать до следующего подключения. Возвращает то же самое
/// `true`/`false`, что окно уже умеет превращать в «Включаем...».
async fn reconnect_if_live(core: Arc<Core>) -> bool {
    let live = {
        let s = core.session.lock().await;
        matches!(s.phase, VpnPhase::On | VpnPhase::Unconfirmed | VpnPhase::Healing)
    };
    if live {
        logger::log("info", "app", "настройки туннеля изменены, переподключаемся");
        // Переподключение в фоне: команда не должна ждать весь подъём, иначе
        // окно замрёт на тумблере.
        tauri::async_runtime::spawn(async move {
            core.disconnect().await;
            let _ = core.connect().await;
        });
    }
    live
}

/// Итог сохранения «Своих правил» (TunnelScreen): что не приняли, по списку,
/// и переподключился ли туннель. Строки, а не просто число - иначе окну
/// нечем было бы объяснить «строка не принята» рядом с полем.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CustomRulesOutcome {
    direct_ignored: Vec<String>,
    proxy_ignored: Vec<String>,
    reconnected: bool,
}

/// Проверить и сохранить «Свои правила»: два списка доменов (по строке на
/// правило), «Всегда напрямую» и «Всегда через VPN». Каждая строка проходит
/// через `tunnel_prefs::parse_custom_rules` - недействительная (адрес,
/// ссылка, звёздочка) или домен, оказавшийся сразу в обоих списках, попадает
/// в `*_ignored`, а не молча теряется и не роняет всю команду.
#[tauri::command]
async fn tunnel_rules_set(
    core: tauri::State<'_, Arc<Core>>,
    direct: String,
    proxy: String,
) -> Cmd<CustomRulesOutcome> {
    let parsed = tunnel_prefs::parse_custom_rules(&direct, &proxy);
    let mut prefs = tunnel_prefs::load();
    prefs.direct_domains = parsed.direct;
    prefs.proxy_domains = parsed.proxy;
    if !tunnel_prefs::store(&prefs) {
        return Err(AppError::new(ErrorCode::Unknown).to_payload());
    }
    let reconnected = reconnect_if_live(core.inner().clone()).await;
    Ok(CustomRulesOutcome {
        direct_ignored: parsed.direct_ignored,
        proxy_ignored: parsed.proxy_ignored,
        reconnected,
    })
}

/// «Уведомления» (MoreScreen), как записаны сейчас. Не влияет на туннель -
/// поэтому, в отличие от `tunnel_prefs_set`, никогда не переподключает.
#[tauri::command]
async fn notify_prefs_get() -> Cmd<notify_prefs::NotifyPrefs> {
    Ok(notify_prefs::load())
}

#[tauri::command]
async fn notify_prefs_set(enabled: bool) -> Cmd<()> {
    if !notify_prefs::store(&notify_prefs::NotifyPrefs { enabled }) {
        return Err(AppError::new(ErrorCode::Unknown).to_payload());
    }
    Ok(())
}

/// `App.tsx` держит язык окна на своей стороне (`LANG_KEY`); эта команда
/// только сообщает его сюда, чтобы системное уведомление (`notify_protection`)
/// заговорило на том же языке, что и всё остальное окно. Ничего не пишет на
/// диск - при следующем запуске окно пришлёт его заново само, как и всегда
/// делает при монтировании.
#[tauri::command]
async fn set_ui_lang(core: tauri::State<'_, Arc<Core>>, lang: String) -> Cmd<()> {
    core.ui_lang_en
        .store(lang == "en", std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
async fn measure_locations(core: tauri::State<'_, Arc<Core>>) -> Cmd<Vec<LocationEntry>> {
    let core = core.inner().clone();
    core.measure_all_rtt().await;
    // Список берём в отдельной области: иначе заимствование доживает до
    // возврата и компилятор справедливо ругается.
    let list = {
        let s = core.session.lock().await;
        s.locations()
    };
    Ok(list)
}

#[tauri::command]
async fn select_location(core: tauri::State<'_, Arc<Core>>, id: Option<String>) -> Cmd<()> {
    let core = core.inner().clone();
    let target = {
        let mut s = core.session.lock().await;
        match &id {
            None => {
                // Back to automatic. The live connection is left alone: moving
                // it would punish the person for choosing less control.
                s.pinned = None;
                None
            }
            Some(id) => {
                let index = s
                    .index_of(id)
                    .ok_or_else(|| AppError::new(ErrorCode::SubEmpty).to_payload())?;
                s.pinned = Some(Pinned {
                    id: id.clone(),
                    at: Instant::now(),
                });
                // A country chosen by hand is not one that "does not get
                // through": the person is allowed to overrule us.
                s.demoted.remove(id);
                (s.current != Some(index)).then_some(index)
            }
        }
    };

    let Some(index) = target else { return Ok(()) };
    let phase = core.session.lock().await.phase;
    if matches!(phase, VpnPhase::Off | VpnPhase::Failed) {
        return Ok(());
    }

    let _op = core.operation.lock().await;
    // A repair ladder may be climbing right now — the main screen says it is
    // healing, which is exactly when people open the list. Both restart the
    // engines, on different nodes, and the ladder never took this lock: the
    // two interleaved, the ladder could override the choice a moment later,
    // or one restart failed with "already running". A new generation retires
    // that ladder and the supervisor that runs it (both stop at their next
    // check, and the ladder's engine steps wait for this lock — see
    // `repair_step`); a fresh supervisor follows this switch.
    let generation = core.bump_generation().await;
    core.spawn_supervisor(generation);
    let switched = core.switch_to(index, generation).await;
    drop(_op);
    if let Err(err) = switched {
        // The retired ladder will not finish what it started; this one does.
        core.spawn_heal(generation, err.clone());
        return Err(err.to_payload());
    }

    match core.probe(ProbeReason::AfterConnect).await {
        Some(ProbeVerdict::Passed) => core.settle(VpnPhase::On).await,
        Some(ProbeVerdict::Unconfirmed) => core.settle(VpnPhase::Unconfirmed).await,
        Some(other) => {
            let cause = other
                .as_error()
                .unwrap_or_else(|| AppError::new(ErrorCode::Unknown));
            core.spawn_heal(generation, cause);
        }
        None => {
            // No verdict, and a ladder that was running is retired: do not
            // leave the screen on "healing" with nobody working on it.
            let (phase, cause) = {
                let s = core.session.lock().await;
                (s.phase, s.error.clone())
            };
            if phase == VpnPhase::Healing {
                core.spawn_heal(generation, cause.unwrap_or_else(|| AppError::new(ErrorCode::Unknown)));
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn sub_state(core: tauri::State<'_, Arc<Core>>) -> Cmd<SubState> {
    let s = core.session.lock().await;
    Ok(SubState {
        has_link: load_link().is_some(),
        last_updated_at: s.sub_fetched_at,
        used_fallback: s.sub_used_fallback,
    })
}

#[tauri::command]
async fn sub_set(core: tauri::State<'_, Arc<Core>>, link: String) -> Cmd<()> {
    let link = validate_link(&link).map_err(|e| e.to_payload())?;
    store_link(&link).map_err(|e| e.to_payload())?;
    // A pasted link is a sign-in too: a code tried before it is done with.
    core.pair_code_ids.forget();
    let mut s = core.session.lock().await;
    // A new link is a new account: everything learned about the old one is
    // about somebody else's list.
    s.servers.clear();
    s.current = None;
    s.pinned = None;
    s.recents.clear();
    s.demoted.clear();
    s.quality.clear();
    s.rtt.clear();
    s.last_good = None;
    s.sub_fetched_at = None;
    Ok(())
}

#[tauri::command]
async fn sub_clear(core: tauri::State<'_, Arc<Core>>) -> Cmd<()> {
    let core = core.inner().clone();
    core.disconnect().await;
    clear_link();
    let mut s = core.session.lock().await;
    s.servers.clear();
    s.meta = SubMeta::default();
    s.sub_fetched_at = None;
    Ok(())
}

#[tauri::command]
async fn sub_refresh(core: tauri::State<'_, Arc<Core>>) -> Cmd<SubMeta> {
    let core = core.inner().clone();
    core.refresh_subscription()
        .await
        .map_err(|e| e.to_payload())?;
    let meta = core.session.lock().await.meta.clone();
    Ok(meta)
}

#[tauri::command]
async fn routing_get(core: tauri::State<'_, Arc<Core>>) -> Cmd<RoutingState> {
    let s = core.session.lock().await;
    Ok(RoutingState {
        in_russia: s.routing_in_russia,
        guess: guess_region(&read_timezone()),
    })
}

#[tauri::command]
async fn routing_set(core: tauri::State<'_, Arc<Core>>, in_russia: bool) -> Cmd<()> {
    let core = core.inner().clone();
    let changed = {
        let mut s = core.session.lock().await;
        let changed = s.routing_in_russia != in_russia;
        s.routing_in_russia = in_russia;
        changed
    };
    if !changed {
        return Ok(());
    }

    let phase = core.session.lock().await.phase;
    if matches!(phase, VpnPhase::Off | VpnPhase::Failed) {
        return Ok(());
    }

    // The rules live in the subscription answer, so applying the choice means
    // asking for the list again and rebuilding the engine — about a second,
    // and the tunnel is never dropped.
    let _op = core.operation.lock().await;
    let generation = core.session.lock().await.generation;
    let apply = async {
        core.refresh_subscription().await?;
        let index = core.session.lock().await.current.unwrap_or(0);
        let keep = core.tunnel_is_up().await;
        core.start_on(index, generation, keep).await
    };
    if let Err(err) = apply.await {
        // Leaving a silent disagreement between the switch and the engine is
        // worse than saying it did not take.
        core.session.lock().await.routing_in_russia = !in_russia;
        return Err(err.to_payload());
    }
    Ok(())
}

#[tauri::command]
async fn traffic_split(core: tauri::State<'_, Arc<Core>>) -> Cmd<Option<TrafficSplitOut>> {
    let phase = core.session.lock().await.phase;
    if matches!(phase, VpnPhase::Off) {
        return Ok(None);
    }
    let split = logger::traffic_split();
    if split.via_tunnel == 0 && split.direct == 0 {
        // Nothing counted yet. An honest absence, not two zeroes.
        return Ok(None);
    }
    Ok(Some(TrafficSplitOut {
        via_vpn: split.via_tunnel,
        direct: split.direct,
    }))
}

#[tauri::command]
async fn diag_check(core: tauri::State<'_, Arc<Core>>) -> Cmd<CheckReport> {
    let core = core.inner().clone();
    Ok(core.run_checks().await)
}

#[tauri::command]
async fn diag_fix(core: tauri::State<'_, Arc<Core>>) -> Cmd<CheckReport> {
    // Fixing is the service's half, and the service half is not reachable with
    // the credentials this app holds. Re-running the local checks is the only
    // honest answer, and the window only offers the button when `canFix` is
    // true — which, today, it never is.
    let core = core.inner().clone();
    Ok(core.run_checks().await)
}

#[tauri::command]
async fn report_build(core: tauri::State<'_, Arc<Core>>) -> Cmd<SupportReport> {
    let core = core.inner().clone();
    Ok(core.build_report().await)
}

#[tauri::command]
async fn timeline_list(
    core: tauri::State<'_, Arc<Core>>,
    limit: Option<usize>,
) -> Cmd<Vec<TimelineEntry>> {
    let s = core.session.lock().await;
    let take = limit.unwrap_or(TIMELINE_LIMIT).clamp(1, TIMELINE_LIMIT);
    Ok(s.timeline.iter().rev().take(take).cloned().collect())
}

#[tauri::command]
fn logs_page(offset: Option<usize>, limit: Option<usize>) -> Vec<LogLineOut> {
    let limit = limit.unwrap_or(200).clamp(1, logger::SNAPSHOT_LIMIT);
    let offset = offset.unwrap_or(0);
    let mut lines = logger::snapshot(None);
    lines.reverse(); // newest first: a log is read from the end
    lines
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|line| LogLineOut {
            ts_ms: line.ts_ms as u64,
            level: line.level,
            source: line.source,
            message: line.message,
        })
        .collect()
}

#[tauri::command]
async fn pair_start() -> Cmd<PairSession> {
    // Сопряжение не писало в журнал НИ СТРОЧКИ, и 22.09.2026 это стоило часа:
    // телефон отчитался «отправлено», окно ждало вечно, а посмотреть было не на
    // что. Теперь виден каждый шаг.
    match start_pairing().await {
        Ok(session) => {
            logger::log(
                "info",
                "pair",
                &format!("code issued, valid for {} ms", session.expires_at.saturating_sub(now_ms())),
            );
            Ok(session)
        }
        Err(e) => {
            logger::log("error", "pair", &format!("code request failed: {e}"));
            Err(e.to_payload())
        }
    }
}

#[tauri::command]
async fn pair_poll(core: tauri::State<'_, Arc<Core>>, token: String) -> Cmd<PairStatus> {
    let core = core.inner().clone();
    let outcome = match poll_pairing(&token).await {
        Ok(outcome) => outcome,
        Err(e) => {
            // Окно глотает ошибку опроса молча — «одна неудачная проба не
            // новость». Если она неудачная КАЖДЫЙ раз, человек смотрит на код
            // до бесконечности. В журнале это теперь видно.
            logger::log("error", "pair", &format!("poll failed: {e}"));
            return Err(e.to_payload());
        }
    };
    logger::log(
        "info",
        "pair",
        match outcome {
            PairOutcome::Waiting => "server says: still waiting",
            PairOutcome::Linked(_) => "server says: link arrived",
            PairOutcome::Expired => "server says: code spent or expired",
        },
    );
    if let PairOutcome::Linked(link) = outcome {
        adopt_paired_link(&core, &link)
            .await
            .map_err(|e| e.to_payload())?;
        return Ok(PairStatus::Linked);
    }
    Ok(match outcome {
        PairOutcome::Waiting => PairStatus::Waiting,
        PairOutcome::Expired => PairStatus::Expired,
        PairOutcome::Linked(_) => PairStatus::Linked,
    })
}

/// Pair code v1.1: the eight characters from the cabinet or the bot, typed on
/// the sign-in screen. On success the link is kept exactly as a scanned QR
/// keeps it, and the window connects the same way.
///
/// Every press with the same code carries the same request id
/// (`core.pair_code_ids`), so pressing again after "could not reach the
/// service" gets back a link whose first answer was lost on the way.
///
/// The log gets the outcome class and nothing else: never the code, never
/// the link, never the request id (pair_code.rs).
#[tauri::command]
async fn redeem_pair_code(core: tauri::State<'_, Arc<Core>>, code: String) -> Cmd<()> {
    let core = core.inner().clone();
    let wire = site_ladder::HttpWire::new(http_client().map_err(|e| e.to_payload())?);
    let link = match pair_code::redeem(&wire, &SITE_LADDER, &core.pair_code_ids, &code).await {
        Ok(link) => link,
        Err(e) => {
            // A wrong or spent code is the person's everyday, not our error.
            let level = match e.code {
                ErrorCode::PairCodeMalformed | ErrorCode::PairCodeNotFound | ErrorCode::PairRateLimited => {
                    "info"
                }
                _ => "error",
            };
            logger::log(level, "pair", &format!("code not redeemed: {:?}", e.code));
            return Err(e.to_payload());
        }
    };
    logger::log("info", "pair", "code redeemed: link arrived");
    adopt_paired_link(&core, &link)
        .await
        .map_err(|e| e.to_payload())
}

/// A pairing delivered a link, by QR or by code: keep it, and forget the list
/// fetched for whatever link came before, so the next connect reads this one.
///
/// The pair code's request id is forgotten only once the link is stored: a
/// link that arrived but could not be kept may be asked for again with the
/// same code, and the service repeats it for five minutes.
async fn adopt_paired_link(core: &Core, link: &str) -> Result<(), AppError> {
    let link = match validate_link(link) {
        Ok(link) => link,
        Err(e) => {
            // Токен уже погашен сервером: ссылка потеряна навсегда, и
            // человеку придётся просить новый код. Молчать тут нельзя.
            logger::log("error", "pair", &format!("link rejected: {e}"));
            return Err(e);
        }
    };
    if let Err(e) = store_link(&link) {
        logger::log("error", "pair", &format!("link not stored: {e}"));
        return Err(e);
    }
    logger::log("info", "pair", "link stored");
    core.pair_code_ids.forget();
    let mut s = core.session.lock().await;
    s.servers.clear();
    s.sub_fetched_at = None;
    Ok(())
}

#[tauri::command]
async fn onboarding_state() -> Cmd<OnboardingState> {
    Ok(OnboardingState {
        steps: pending_onboarding(),
    })
}

#[tauri::command]
async fn onboarding_run(step: String) -> Cmd<()> {
    run_onboarding(&step).await.map_err(|e| e.to_payload())
}

/// Windows: open a link through the person's own, unelevated shell
/// (pvpn-platform shell.rs). The app runs as administrator there, and the
/// opener plugin would start a browser that is not yet running with those
/// rights. The window calls this on Windows only; elsewhere the opener plugin
/// is used as before. The link is never logged: it can carry a token.
#[tauri::command]
fn open_external_unelevated(url: String) -> Cmd<()> {
    #[cfg(target_os = "windows")]
    {
        pvpn_platform::shell::open_url_unelevated(&url).map_err(|e| {
            logger::log("warn", "app", &format!("link not opened: {e:#}"));
            AppError::new(ErrorCode::Unknown).to_payload()
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        drop(url);
        Err(AppError::new(ErrorCode::Unknown).to_payload())
    }
}

#[tauri::command]
async fn app_info(core: tauri::State<'_, Arc<Core>>) -> Cmd<AppInfo> {
    let (support, site) = {
        let s = core.session.lock().await;
        (
            s.meta.support_url.clone(),
            // The site that answered last already walked the ladder, so it is
            // the one this person's provider lets through. Asking the network
            // again here would put a request in front of every window mount.
            s.sub_source_host.clone(),
        )
    };
    let site = site
        .or_else(|| {
            load_link()
                .and_then(|link| Url::parse(&link).ok())
                .and_then(|url| url.host_str().map(str::to_string))
        })
        .unwrap_or_else(|| SITE_LADDER[0].to_string());
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(target_os = "ios") {
            "ios"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "other"
        },
        // The host that answered last is the host that works for THIS person;
        // sending them to a name their provider blocks is how a working button
        // becomes a dead end.
        cabinet_url: format!("https://{site}{CABINET_PATH}"),
        bot_url: support.unwrap_or_else(|| BOT_URL.to_string()),
        device_name: device_name(),
        device_linked_at: link_stored_at(),
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Diagnostics (design M6, local layer)
// ───────────────────────────────────────────────────────────────────────────

/// Two well-known public addresses used only to ask "does this network let
/// anything out at all". Deliberately not ours: the answer has to hold when
/// every one of our names is blocked, which is exactly when it is asked.
const NEUTRAL_PROBES: [&str; 2] = ["1.1.1.1:443", "8.8.8.8:443"];

impl Core {
    async fn run_checks(self: &Arc<Self>) -> CheckReport {
        let mut rows: Vec<CheckRow> = Vec::new();
        let mut row = |id: &'static str, state: CheckState, via_fallback: bool| {
            rows.push(CheckRow {
                id,
                state,
                via_fallback,
            });
        };

        // The clock, first and offline: a machine whose date is wrong fails
        // every TLS handshake and blames us for it.
        row("clock", clock_is_plausible(), false);
        row(
            "internet",
            bool_state(probe::has_usable_link()),
            false,
        );
        row("dns", bool_state(resolve_works().await), false);

        let (sub_state, via_fallback) = self.check_subscription().await;
        row("sub", sub_state, via_fallback);

        row("handshake", self.check_handshake().await, false);

        // Asked now, not read off the phase: the person pressed "Проверить"
        // because they do not believe the screen, and answering with the same
        // screen is not a check. `UserRequested` exists for exactly this and
        // carries a three-second floor against a double tap.
        let phase = self.session.lock().await.phase;
        row(
            "traffic",
            if phase == VpnPhase::Off {
                CheckState::Skip
            } else {
                match self.probe(ProbeReason::UserRequested).await {
                    Some(ProbeVerdict::Passed) | Some(ProbeVerdict::Unconfirmed) => {
                        CheckState::Pass
                    }
                    Some(_) => CheckState::Fail,
                    // Too soon to ask again; the last answer still stands.
                    None => bool_state(matches!(
                        phase,
                        VpnPhase::On | VpnPhase::Unconfirmed
                    )),
                }
            },
            false,
        );
        row("network", bool_state(network_lets_anything_out().await), false);

        let verdict = rows
            .iter()
            .find(|r| r.state == CheckState::Fail)
            .map(|r| r.id);

        CheckReport {
            device: rows,
            service: CheckService::Unavailable {
                available: false,
                reason: "cabinetOnly",
            },
            verdict,
        }
    }

    async fn check_subscription(self: &Arc<Self>) -> (CheckState, bool) {
        let Some(link) = load_link() else {
            return (CheckState::Skip, false);
        };
        let in_russia = self.session.lock().await.routing_in_russia;
        let Ok(url) = effective_sub_url(&link, in_russia) else {
            return (CheckState::Fail, false);
        };
        let lang_en = self.ui_lang_en.load(std::sync::atomic::Ordering::Relaxed);
        match fetch_subscription(&url, lang_en).await {
            Ok(sub) => {
                let fallback = !sub.source_host.eq_ignore_ascii_case(SITE_LADDER[0]);
                (CheckState::Pass, fallback)
            }
            Err(_) => (CheckState::Fail, false),
        }
    }

    /// Can we open a TCP connection to the node at all?
    ///
    /// Skipped, not failed, on a Hysteria2 node: it is pure UDP and a TCP
    /// handshake to its port never completes. Calling that a failure is the
    /// same mistake the old ping made on the Netherlands for months.
    async fn check_handshake(self: &Arc<Self>) -> CheckState {
        let server = {
            let s = self.session.lock().await;
            s.current_server().cloned()
        };
        let Some(server) = server else {
            return CheckState::Skip;
        };
        if matches!(server, ServerConfig::Hy2(_)) {
            return CheckState::Skip;
        }
        let target = format!("{}:{}", server.host(), server.port());
        match tokio::time::timeout(
            Duration::from_millis(2500),
            tokio::net::TcpStream::connect(target),
        )
        .await
        {
            Ok(Ok(_)) => CheckState::Pass,
            _ => CheckState::Fail,
        }
    }
}

fn bool_state(ok: bool) -> CheckState {
    if ok {
        CheckState::Pass
    } else {
        CheckState::Fail
    }
}

/// A clock far enough out to break TLS.
///
/// No network involved on purpose — this row has to answer when nothing else
/// can. The lower bound is the date this code was written; a machine claiming
/// to be earlier than its own software has a wrong clock.
fn clock_is_plausible() -> CheckState {
    clock_verdict(now_secs())
}

/// Pure half, so both ends of the window can be tested without moving the
/// machine's clock.
fn clock_verdict(now: u64) -> CheckState {
    const WRITTEN_AT: u64 = 1_758_400_000; // 2026-09-21
    const TEN_YEARS: u64 = 10 * 365 * 24 * 3600;
    // Both directions matter: a certificate is refused for being not yet
    // valid as readily as for having expired, and a machine whose clock ran
    // away is a machine where nothing of ours can work.
    if (WRITTEN_AT..=WRITTEN_AT + TEN_YEARS).contains(&now) {
        CheckState::Pass
    } else {
        CheckState::Fail
    }
}

/// How long to wait before the next full repair ladder.
///
/// Separated from the state it mutates so the walk up the table — and the
/// fact that it stops at the top instead of panicking — is testable.
fn heal_backoff(round: usize) -> Duration {
    HEAL_BACKOFF[round.min(HEAL_BACKOFF.len() - 1)]
}

async fn resolve_works() -> bool {
    let host = format!("{}:443", SITE_LADDER[0]);
    tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        host.to_socket_addrs().map(|mut it| it.next().is_some()).unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Does this network let ANY outbound TLS through?
///
/// Both addresses have to fail before we say no: one unreachable resolver is a
/// routing quirk, two is a network that only allows a list. The cost of
/// getting this wrong in either direction is high — a support template already
/// told somebody "only Wi-Fi works" while he was connecting over cellular.
async fn network_lets_anything_out() -> bool {
    for target in NEUTRAL_PROBES {
        let ok = tokio::time::timeout(
            Duration::from_millis(1500),
            tokio::net::TcpStream::connect(target),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
        if ok {
            return true;
        }
    }
    false
}

// ───────────────────────────────────────────────────────────────────────────
// The support report (design [9])
// ───────────────────────────────────────────────────────────────────────────

impl Core {
    async fn build_report(self: &Arc<Self>) -> SupportReport {
        let s = self.session.lock().await;
        let mut out = String::new();

        out.push_str(&format!(
            "app {} {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ));
        out.push_str(&format!("phase {:?}\n", s.phase));
        if let Some(err) = &s.error {
            out.push_str(&format!("code {:?}\n", err.code));
        }
        if let Some(proto) = &s.proto {
            // The protocol and the label, never the address.
            out.push_str(&format!("link {proto}\n"));
        }
        if let Some(location) = &s.location {
            out.push_str(&format!("location {location}\n"));
        }
        out.push_str(&format!(
            "bytes rx={} tx={}\n",
            s.metric.rx_bytes, s.metric.tx_bytes
        ));
        if let Some(proof) = s.metric.last_proof_at {
            out.push_str(&format!(
                "last-proof {}s ago\n",
                now_ms().saturating_sub(proof) / 1000
            ));
        }
        if let Some(at) = s.sub_fetched_at {
            out.push_str(&format!(
                "subscription {}s ago{}\n",
                now_ms().saturating_sub(at) / 1000,
                if s.sub_used_fallback { " (reserve)" } else { "" }
            ));
        }
        let split = logger::traffic_split();
        out.push_str(&format!(
            "connections vpn={} direct={}\n",
            split.via_tunnel, split.direct
        ));
        out.push_str(&format!("locations {}\n", s.servers.len()));

        out.push_str("events\n");
        for entry in s.timeline.iter().rev().take(20) {
            out.push_str(&format!(
                "  {} {:?}{}\n",
                entry.at_ms,
                entry.code,
                entry
                    .location
                    .as_ref()
                    .map(|l| format!(" {l}"))
                    .unwrap_or_default()
            ));
        }

        // Errors only, and already redacted on the way into the buffer —
        // redacting at export would mean the file itself was unsafe.
        out.push_str("errors\n");
        for line in logger::snapshot(None)
            .into_iter()
            .filter(|l| l.level == "error")
            .rev()
            .take(10)
        {
            out.push_str(&format!("  [{}] {}\n", line.source, line.message));
        }

        SupportReport {
            bytes: out.len(),
            text: out,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Pairing: the QR that replaces typing a link with a token in it
// ───────────────────────────────────────────────────────────────────────────

enum PairOutcome {
    Waiting,
    Linked(String),
    Expired,
}

fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(subscription::device_id::user_agent())
        .build()
        .map_err(|e| {
            logger::log("error", "pair", &format!("client: {e}"));
            AppError::new(ErrorCode::Unknown)
        })
}

/// Walk the ladder of site names until one answers (site_ladder.rs: in
/// order, one at a time). A 404 is the endpoint's own "nothing there".
async fn ladder_get(path: &str) -> Result<(String, String), AppError> {
    let wire = site_ladder::HttpWire::new(http_client()?);
    site_ladder::walk(&wire, &SITE_LADDER, site_ladder::Method::Get, path, None, |host, reply| {
        if reply.is_success() {
            site_ladder::Verdict::Done((host.to_string(), reply.body))
        } else if reply.status == 404 {
            site_ladder::Verdict::Stop(AppError::new(ErrorCode::SubEmpty))
        } else {
            site_ladder::Verdict::Next
        }
    })
    .await
}

/// The same walk for a POST, with an optional JSON body. Anything but a 2xx
/// sends the request on to the next name. `/api/pair/code`, whose errors are
/// answers, judges its replies itself (`pair_code::redeem`) over this ladder.
async fn ladder_post(
    path: &str,
    json: Option<&serde_json::Value>,
) -> Result<(String, String), AppError> {
    let wire = site_ladder::HttpWire::new(http_client()?);
    site_ladder::walk(&wire, &SITE_LADDER, site_ladder::Method::Post, path, json, |host, reply| {
        if reply.is_success() {
            site_ladder::Verdict::Done((host.to_string(), reply.body))
        } else {
            site_ladder::Verdict::Next
        }
    })
    .await
}

/// `POST /api/pair/new` → `{"token": "<32 hex>", "expiresIn": 300}`.
async fn start_pairing() -> Result<PairSession, AppError> {
    let (_, body) = ladder_post("/api/pair/new", None).await?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| AppError::new(ErrorCode::SubInvalid))?;
    let token = parsed
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|t| is_pair_token(t))
        .ok_or_else(|| AppError::new(ErrorCode::SubInvalid))?;
    let ttl = parsed
        .get("expiresIn")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(300);
    Ok(PairSession {
        token: token.to_string(),
        expires_at: now_ms() + ttl * 1000,
    })
}

/// `GET /api/pair/<token>` → pending | ready+subUrl | 404 expired.
async fn poll_pairing(token: &str) -> Result<PairOutcome, AppError> {
    if !is_pair_token(token) {
        return Err(AppError::new(ErrorCode::SubMalformed));
    }
    match ladder_get(&format!("/api/pair/{token}")).await {
        Ok((_, body)) => read_pair_answer(&body),
        // The endpoint answers 404 once the token is spent or timed out, which
        // the ladder reports as "nothing there".
        Err(err) if err.code == ErrorCode::SubEmpty => Ok(PairOutcome::Expired),
        Err(err) => Err(err),
    }
}

/// What one answer of `GET /api/pair/<token>` means.
///
/// The link in a "ready" answer goes through the same gate as a link
/// delivered by a pair code (`pair_code::accept_pair_link`: https, one of our
/// own sites). The QR slot is written by whoever POSTs first, with no session,
/// and the token is in the QR on the screen: someone who reads it off a
/// screen share or a screenshot could put any link there — plain http, any
/// host — and the app stored it and sent all of the person's traffic through
/// the servers that link names.
fn read_pair_answer(body: &str) -> Result<PairOutcome, AppError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| AppError::new(ErrorCode::SubInvalid))?;
    match parsed.get("status").and_then(|v| v.as_str()) {
        Some("ready") => {
            let raw = parsed
                .get("subUrl")
                .and_then(|v| v.as_str())
                .ok_or_else(|| AppError::new(ErrorCode::SubInvalid))?;
            pair_code::accept_pair_link(raw, &SITE_LADDER)
                .map(PairOutcome::Linked)
                .inspect_err(|_| {
                    logger::log("warn", "pair", "refused a paired link that is not https on one of our sites");
                })
        }
        Some("pending") => Ok(PairOutcome::Waiting),
        _ => Ok(PairOutcome::Expired),
    }
}

/// Exactly what the cabinet's scanner accepts: `^[a-f0-9]{32}$`.
fn is_pair_token(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

// ───────────────────────────────────────────────────────────────────────────
// First run
// ───────────────────────────────────────────────────────────────────────────

fn pending_onboarding() -> Vec<&'static str> {
    onboarding_steps(&consent_paths())
}

/// Where the data-notice record lives: beside the subscription link, in each
/// of the same directories (`link_paths`) and read in the same order - on iOS
/// that is the app's own container, on macOS the shared directory first.
fn consent_paths() -> Vec<std::path::PathBuf> {
    link_paths()
        .into_iter()
        .filter_map(|link| link.parent().map(|dir| dir.join(consent::FILE_NAME)))
        .collect()
}

/// The first-run steps, given where the data-notice record would be.
///
/// The data notice goes first on every platform (guideline 5.4: the
/// declaration comes before any use of the service). The window shows nothing
/// else - pairing included, and with it the first request to the service -
/// until every step here is done.
///
/// macOS caveat: a record written by the first, non-root run lands under that
/// user's `HOME`. When the launcher then restarts the app as root with root's
/// own `HOME` (the osascript path, see device_id.rs), that run cannot see it
/// and shows the notice once more; its own record goes to the shared
/// directory, and the question does not come back after that.
fn onboarding_steps(consent_paths: &[std::path::PathBuf]) -> Vec<&'static str> {
    let mut steps = Vec::new();
    if consent::is_pending(consent_paths) {
        steps.push("dataNotice");
    }
    #[cfg(target_os = "macos")]
    {
        if !running_from_applications() {
            steps.push("moveToApplications");
        }
        if !tun::is_root() {
            // Not a warning before the prompt — we are past it and it did not
            // happen. The screen explains and the button re-launches properly.
            steps.push("password");
        }
    }
    steps
}

#[cfg(target_os = "macos")]
fn running_from_applications() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        // Unknown is not a reason to send someone through a move they may not
        // need; the step is an instruction, and a wrong instruction is worse
        // than a missing one.
        return true;
    };
    if exe.starts_with("/Applications") {
        return true;
    }
    // `cargo tauri dev` runs out of target/, where there is no bundle to move
    // and the screen would appear on every launch for whoever is developing.
    let path = exe.to_string_lossy();
    path.contains("/target/debug/") || path.contains("/target/release/")
}

#[cfg(target_os = "macos")]
fn bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // …/ProxysVPN.app/Contents/MacOS/<exec>
    let bundle = exe.parent()?.parent()?.parent()?;
    bundle
        .extension()
        .is_some_and(|ext| ext == "app")
        .then(|| bundle.to_path_buf())
}

async fn run_onboarding(step: &str) -> Result<(), AppError> {
    match step {
        "dataNotice" => record_data_notice(),
        #[cfg(target_os = "macos")]
        "moveToApplications" => move_to_applications().await,
        #[cfg(target_os = "macos")]
        "password" => relaunch_through_launcher().await,
        // iOS asks for the VPN permission as part of the first connect; there
        // is nothing to run ahead of it, and a button that does nothing would
        // be worse than no button.
        "iosPermission" => Ok(()),
        _ => Err(AppError::new(ErrorCode::Unknown)),
    }
}

/// "Continue" on the data notice: remember that this version was shown.
fn record_data_notice() -> Result<(), AppError> {
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    match consent::record(&consent_paths(), now_ms) {
        Ok(()) => {
            logger::log(
                "info",
                "app",
                &format!("data notice v{} accepted", consent::DATA_NOTICE_VERSION),
            );
            Ok(())
        }
        Err(err) => {
            // The io error alone: the path would carry the user's name.
            logger::log("error", "app", &format!("could not record the data notice: {err}"));
            Err(AppError::new(ErrorCode::Unknown))
        }
    }
}

#[cfg(target_os = "macos")]
async fn move_to_applications() -> Result<(), AppError> {
    let Some(bundle) = bundle_path() else {
        logger::log("error", "app", "not running from a bundle");
        return Err(AppError::new(ErrorCode::Unknown));
    };
    let name = bundle
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "ProxysVPN.app".to_string());
    let destination = std::path::PathBuf::from("/Applications").join(&name);

    let status = tokio::process::Command::new("/bin/cp")
        .args(["-R", &bundle.to_string_lossy(), "/Applications/"])
        .status()
        .await
        .map_err(|e| {
            logger::log("error", "app", &format!("copy to /Applications: {e}"));
            AppError::new(ErrorCode::PermissionDenied)
        })?;
    if !status.success() {
        logger::log("error", "app", "copy to /Applications failed");
        return Err(AppError::new(ErrorCode::PermissionDenied));
    }

    let _ = tokio::process::Command::new("/usr/bin/open")
        .args(["-n", &destination.to_string_lossy()])
        .status()
        .await;
    // The copy is done and the new one is starting; this process has nothing
    // left to do, and two copies would fight over the routing table. Engines
    // and routes go first: `exit` runs no destructors.
    sync_cleanup();
    std::process::exit(0);
}

#[cfg(target_os = "macos")]
async fn relaunch_through_launcher() -> Result<(), AppError> {
    let Some(bundle) = bundle_path() else {
        return Err(AppError::new(ErrorCode::Unknown));
    };
    let _ = tokio::process::Command::new("/usr/bin/open")
        .args(["-n", &bundle.to_string_lossy()])
        .status()
        .await;
    sync_cleanup();
    std::process::exit(0);
}

// ───────────────────────────────────────────────────────────────────────────
// Small local facts
// ───────────────────────────────────────────────────────────────────────────

fn read_timezone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|path| {
            let text = path.to_string_lossy().to_string();
            text.split_once("zoneinfo/")
                .map(|(_, zone)| zone.to_string())
        })
        .unwrap_or_default()
}

/// Where the machine thinks it is, from its own timezone.
///
/// A guess, offered as one: the window shows it under the choice rather than
/// making it. Nothing leaves the device to produce this answer.
fn guess_region(timezone: &str) -> Option<&'static str> {
    const RU_ZONES: [&str; 27] = [
        "Europe/Moscow",
        "Europe/Kaliningrad",
        "Europe/Samara",
        "Europe/Saratov",
        "Europe/Astrakhan",
        "Europe/Ulyanovsk",
        "Europe/Volgograd",
        "Europe/Kirov",
        "Europe/Simferopol",
        "Asia/Yekaterinburg",
        "Asia/Omsk",
        "Asia/Novosibirsk",
        "Asia/Barnaul",
        "Asia/Tomsk",
        "Asia/Novokuznetsk",
        "Asia/Krasnoyarsk",
        "Asia/Irkutsk",
        "Asia/Chita",
        "Asia/Yakutsk",
        "Asia/Khandyga",
        "Asia/Vladivostok",
        "Asia/Ust-Nera",
        "Asia/Magadan",
        "Asia/Sakhalin",
        "Asia/Srednekolymsk",
        "Asia/Kamchatka",
        "Asia/Anadyr",
    ];
    if timezone.trim().is_empty() {
        return None;
    }
    if RU_ZONES.contains(&timezone) {
        Some("ru")
    } else {
        Some("abroad")
    }
}

/// The machine's own name, for [10]. Never a serial number.
fn device_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/sbin/scutil")
            .args(["--get", "ComputerName"])
            .output()
        {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
        "Mac".to_string()
    }
    // The name the person gave the machine; never the user name, which the
    // same environment also carries.
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Windows PC".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Linux".to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "iPhone".to_string()
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Desktop chrome: signals, menu, tray
// ───────────────────────────────────────────────────────────────────────────

/// Stop anything of ours that outlived a previous run, and its routes.
///
/// "Ours" means written down in `engine.pids` when we spawned it, and still
/// running that very binary out of our own Contents/MacOS (pidfile.rs). This
/// used to be `pkill -9 -x` over tun2socks, xray, hysteria and sing-box —
/// every process on the Mac with those names, so launching or quitting
/// ProxysVPN also killed another VPN client's engine. Every engine is spawned
/// through `pidfile::Engine`, which is why there is no list of names here to
/// keep in step any more. Linux reads the same file from the user's data
/// folder; on Windows nothing of ours can outlive us (pidfile_windows.rs) and
/// only the routes are left to clean.
#[cfg(desktop)]
fn sync_cleanup() {
    pidfile::reap_all();
    tun::purge_stale_routes();
}

#[cfg(all(desktop, unix))]
fn install_signal_handlers() {
    tauri::async_runtime::spawn(async {
        use tokio::signal::unix::{signal, SignalKind};
        let Ok(mut term) = signal(SignalKind::terminate()) else {
            return;
        };
        let Ok(mut int) = signal(SignalKind::interrupt()) else {
            return;
        };
        let Ok(mut hup) = signal(SignalKind::hangup()) else {
            return;
        };

        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
            _ = hup.recv() => {}
        }
        sync_cleanup();
        std::process::exit(0);
    });
}

/// Leaves the routing table clean when the process is asked to die outside the
/// normal UI path. Windows has no signals: Ctrl+C is the only equivalent a GUI
/// process can observe, and a logoff or shutdown ends us without a usable
/// notification. That case is covered elsewhere — every Windows route is
/// written with `store=active`, so it does not survive a reboot, the Wintun
/// adapter dies with tun2socks and takes its routes along, the engines die with
/// the app (job object), and the host route left behind by a logoff is cleared
/// from the route hint on the next start.
#[cfg(target_os = "windows")]
fn install_signal_handlers() {
    tauri::async_runtime::spawn(async {
        if tokio::signal::ctrl_c().await.is_err() {
            return;
        }
        sync_cleanup();
        std::process::exit(0);
    });
}

/// macOS application menu.
///
/// Only macOS: `hide_others` / `show_all` are Cocoa concepts, and on Windows
/// and Linux `set_menu` would draw a menu bar inside the 480x720 fixed window,
/// which is not part of the design. Copy and paste still work there through
/// the WebView's own accelerators.
#[cfg(target_os = "macos")]
fn build_menu(handle: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let app_submenu = Submenu::with_items(
        handle,
        "ProxysVPN",
        true,
        &[
            &PredefinedMenuItem::about(handle, Some("About ProxysVPN"), None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::hide(handle, None)?,
            &PredefinedMenuItem::hide_others(handle, None)?,
            &PredefinedMenuItem::show_all(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::quit(handle, None)?,
        ],
    )?;

    let edit_submenu = Submenu::with_items(
        handle,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(handle, None)?,
            &PredefinedMenuItem::redo(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::cut(handle, None)?,
            &PredefinedMenuItem::copy(handle, None)?,
            &PredefinedMenuItem::paste(handle, None)?,
            &PredefinedMenuItem::select_all(handle, None)?,
        ],
    )?;

    let window_submenu = Submenu::with_items(
        handle,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(handle, None)?,
            &PredefinedMenuItem::close_window(handle, None)?,
        ],
    )?;

    Menu::with_items(handle, &[&app_submenu, &edit_submenu, &window_submenu])
}

// The only Russian sentences left in Rust, and they are here because a native
// menu cannot read src/i18n.ts: the strings are handed to macOS, not to the
// window. Everything the WINDOW says still arrives as a code. When the tray
// gains more languages it will be by asking the window for these five labels,
// not by growing a dictionary here.
#[cfg(desktop)]
const TRAY_STATE_OFF: &str = "Защита выключена";
#[cfg(desktop)]
const TRAY_STATE_WORKING: &str = "Подключаем…";
#[cfg(desktop)]
const TRAY_STATE_ON: &str = "Защищено";
#[cfg(desktop)]
const TRAY_STATE_UNCONFIRMED: &str = "Подтвердить не удалось";
#[cfg(desktop)]
const TRAY_STATE_FAILED: &str = "Не проходит";
#[cfg(desktop)]
const TRAY_SHOW: &str = "Открыть окно";
#[cfg(desktop)]
const TRAY_DISCONNECT: &str = "Выключить защиту";
#[cfg(desktop)]
const TRAY_QUIT: &str = "Выйти";

#[cfg(desktop)]
fn tray_state_text(phase: VpnPhase) -> &'static str {
    match phase {
        VpnPhase::Off => TRAY_STATE_OFF,
        VpnPhase::Starting | VpnPhase::Healing => TRAY_STATE_WORKING,
        VpnPhase::On => TRAY_STATE_ON,
        VpnPhase::Unconfirmed => TRAY_STATE_UNCONFIRMED,
        VpnPhase::Failed => TRAY_STATE_FAILED,
    }
}

#[cfg(desktop)]
impl Core {
    /// First line of the tray menu is always the state (design [13]).
    async fn refresh_tray(&self, phase: VpnPhase) {
        let location = self.session.lock().await.location.clone();
        let text = match (phase, location) {
            (VpnPhase::On, Some(place)) => format!("{} · {place}", tray_state_text(phase)),
            _ => tray_state_text(phase).to_string(),
        };
        if let Some(tray) = self.app.tray_by_id("main-tray") {
            let _ = tray.set_tooltip(Some(&format!("ProxysVPN · {text}")));
        }
        // `try_state`: the tray is built during setup, and a phase change can
        // reach here before it exists or after a build that failed.
        if let Some(items) = self.app.try_state::<TrayItems>() {
            if let Some(item) = items.state.lock().await.as_ref() {
                let _ = item.set_text(&text);
            }
        }
    }

    /// «Уведомления» (MoreScreen, notify_prefs.rs): a system notification for
    /// the two events `set_phase` decided are worth one — `restored = false`
    /// is "protection dropped", `true` is "protection restored". Called at
    /// most once per real phase change (see `set_phase`'s own gating), so no
    /// separate de-duplication is needed here.
    ///
    /// Two more gates, both cheap to skip on: the setting itself, and the
    /// window's own focus — a toast for something already visible as a shield
    /// on screen would just be noise repeating itself.
    async fn notify_protection(&self, restored: bool) {
        if !notify_prefs::load().enabled {
            return;
        }
        if self.window_focused.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let en = self.ui_lang_en.load(std::sync::atomic::Ordering::Relaxed);
        let (title, body) = match (restored, en) {
            (false, false) => ("Защита пропала", "Соединение прервалось, приложение пытается его восстановить"),
            (false, true) => ("Protection dropped", "The connection broke; the app is trying to bring it back"),
            (true, false) => ("Защита восстановлена", "Соединение снова работает"),
            (true, true) => ("Protection restored", "The connection is working again"),
        };
        use tauri_plugin_notification::NotificationExt;
        if let Err(err) = self.app.notification().builder().title(title).body(body).show() {
            // A missed OS notification is not worth failing anything over —
            // the shield in the window already says the same thing to anyone
            // looking, which is the whole reason `notify_protection` skips
            // itself while that window is focused.
            logger::log("warn", "app", &format!("уведомление не показано: {err}"));
        }
    }
}

/// The tray's state line, so it can be rewritten as the phase changes.
#[cfg(desktop)]
struct TrayItems {
    state: Mutex<Option<MenuItem<tauri::Wry>>>,
}

#[cfg(desktop)]
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state_item = MenuItem::with_id(app, "state", TRAY_STATE_OFF, false, None::<&str>)?;
    let show_item = MenuItem::with_id(app, "show", TRAY_SHOW, true, None::<&str>)?;
    let disconnect_item =
        MenuItem::with_id(app, "disconnect", TRAY_DISCONNECT, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", TRAY_QUIT, true, None::<&str>)?;

    let tray_menu = Menu::with_items(
        app,
        &[
            &state_item,
            &PredefinedMenuItem::separator(app)?,
            &disconnect_item,
            &show_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;

    let items = TrayItems {
        state: Mutex::new(Some(state_item)),
    };
    app.manage(items);

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("window icon".into()))?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("ProxysVPN")
        .icon(icon)
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event: MenuEvent| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "disconnect" => {
                if let Some(core) = app.try_state::<Arc<Core>>() {
                    let core = core.inner().clone();
                    // Through the same path as the button in the window, so
                    // every engine stops and the window learns about it.
                    tauri::async_runtime::spawn(async move { core.disconnect().await });
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Bring the main window back: from the tray, or from a second launch.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn reveal_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// What a probe during repair counts for. A pass is only a pass when the
/// tunnel was there to carry it: otherwise it went out of the real interface
/// and proves the internet works, not that protection does.
fn healing_verdict(probe: Option<ProbeVerdict>, tunnel_carrying: bool) -> Option<VpnPhase> {
    if !tunnel_carrying {
        return None;
    }
    match probe? {
        ProbeVerdict::Passed => Some(VpnPhase::On),
        ProbeVerdict::Unconfirmed => Some(VpnPhase::Unconfirmed),
        _ => None,
    }
}

/// Smallest inner height the window may be fitted down to, in logical
/// pixels. Matches `minHeight` in tauri.conf.json: every screen body scrolls
/// above its pinned footer, so this is enough for a heading, a little body
/// and the footer.
#[cfg(desktop)]
const MIN_WINDOW_HEIGHT: f64 = 520.0;

/// The inner height (physical pixels) that makes a window whose frame is
/// `outer` tall fit a work area `work_area` tall, or `None` when it already
/// fits. Never below `min_inner`.
#[cfg(desktop)]
fn fitted_inner_height(work_area: u32, outer: u32, inner: u32, min_inner: u32) -> Option<u32> {
    if outer <= work_area {
        return None;
    }
    let frame = outer.saturating_sub(inner);
    Some(work_area.saturating_sub(frame).max(min_inner))
}

/// Shrink the window to the screen it opened on when 720 px plus the frame
/// does not fit there.
///
/// The window is 480x720 logical px. On a 1366x768 screen, or a 1080p laptop
/// at 150 % (672 px of work area under the taskbar), its bottom 30-80 px sat
/// under the taskbar, and the layout pins every footer to the bottom of the
/// viewport: the data notice's only button, "Continue", was unreachable on
/// first run, and later the Country/Check bar was cut off. The window is now
/// resizable too (tauri.conf.json), but nobody should have to discover that
/// before they can press the first button.
#[cfg(desktop)]
fn fit_main_window_to_screen(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let (Ok(outer), Ok(inner), Ok(scale)) =
        (window.outer_size(), window.inner_size(), window.scale_factor())
    else {
        return;
    };
    let work = monitor.work_area();
    // Rounded, positive and far below u32::MAX: a few hundred logical px at a
    // scale factor of at most a handful.
    let min_inner = (MIN_WINDOW_HEIGHT * scale).round().max(0.0) as u32;
    let Some(height) = fitted_inner_height(work.size.height, outer.height, inner.height, min_inner) else {
        return;
    };
    if let Err(e) = window.set_size(tauri::PhysicalSize::new(inner.width, height)) {
        logger::log("warn", "app", &format!("could not fit the window to the screen: {e}"));
        return;
    }
    // Top of the work area, centred across it: `center` uses the whole
    // monitor on some platforms and would put the bottom back under the bar.
    let x = work.position.x + (i64::from(work.size.width) - i64::from(outer.width)).max(0) as i32 / 2;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, work.position.y));
    logger::log(
        "info",
        "app",
        &format!("window fitted to a {} px tall work area", work.size.height),
    );
}

/// The red X with a tray icon present: hide, the VPN keeps running.
///
/// Linux asks first whether the icon can be seen at all (pvpn-platform
/// tray.rs). Stock GNOME has no StatusNotifier host: the tray is "built" and
/// invisible there, and a hidden window could never come back. Minimised, it
/// stays in the dock and in Alt+Tab.
#[cfg(desktop)]
fn close_to_tray(window: &tauri::Window) {
    #[cfg(target_os = "linux")]
    if !pvpn_platform::tray::host_present() {
        let _ = window.minimize();
        return;
    }
    let _ = window.hide();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Linux: when we were started as the privileged helper, become it and never
    // return. This must come before anything else — the helper speaks a JSON
    // protocol on stdout, so a single log line printed first would corrupt the
    // pipe, and it must not build a webview it has no business owning as root.
    // `is_helper_invocation` is `false` on macOS and Windows, which have no
    // helper to become.
    #[cfg(desktop)]
    if pvpn_platform::helper::is_helper_invocation() {
        #[cfg(target_os = "linux")]
        pvpn_platform::helper::run_helper();
    }

    logger::init();
    // Route the platform layer's messages into the same ring buffer and file
    // the support UI reads; without this they would only reach stdout.
    #[cfg(desktop)]
    pvpn_platform::log::set_sink(logger::log);

    // macOS: LaunchServices never starts a second copy, so the leftovers of a
    // crashed run are cleaned here, before anything else. Windows and Linux
    // clean in `setup` instead, after the single-instance plugin has sent a
    // second launch away: run here, it would tear down the routes and engines
    // of the copy that is already running.
    #[cfg(target_os = "macos")]
    sync_cleanup();
    // After the cleanup (which stops what a crashed run of this copy left
    // running from the engine folder) and before anything can connect: copy
    // the engines out of the user-owned bundle into a root-owned folder.
    #[cfg(target_os = "macos")]
    engine_stage::prepare();

    // ОДИН setup на всё приложение. Их было два, и это молча ломало macOS
    // целиком: `Builder::setup` не добавляет обработчик, а ЗАМЕНЯЕТ его, и
    // второй вызов (меню и трей) затирал первый — тот, где регистрируется
    // ядро. `app.manage` не выполнялся никогда.
    //
    // Наружу это выглядело так: команды без состояния работали, а всё, что
    // берёт `State<Arc<Core>>`, падало ещё до входа в тело — Tauri отвечал
    // ошибкой «нет такого состояния», не нашей, поэтому окно видело только
    // UNKNOWN, а в журнале ядра не появлялось ни строчки. Сопряжение
    // показывало код и ждало вечно; подключение не запустилось бы тоже.
    // Найдено 22.09.2026 живым прогоном — тесты такое не ловят, потому что
    // собирают приложение без Builder.
    let builder = tauri::Builder::default();
    // Windows and Linux: a second launch (the desktop icon clicked again while
    // the app sits in the tray) shows the running window and exits. It must
    // be the first plugin: plugins start in order, and the second copy has to
    // leave before anything else of it runs.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        reveal_main_window(app);
    }));
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    // «Уведомления» (notify_prefs.rs): desktop-only, same split as the
    // dependency itself in Cargo.toml — iOS has no window to notify past, and
    // its Network Extension speaks through its own OS-level lifecycle, not
    // this plugin.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_notification::init());
    let builder = builder.setup(|app| {
            // After the single-instance plugin: only the first copy gets here.
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            sync_cleanup();

            let core = Core::new(app.handle().clone());
            app.manage(core);
            app.manage(motion::MotionState::default());

            #[cfg(target_os = "macos")]
            {
                let menu = build_menu(app.handle())?;
                app.set_menu(menu)?;
            }
            #[cfg(target_os = "macos")]
            build_tray(app.handle())?;
            // Windows and Linux: a missing tray is not a reason to refuse to
            // start. Without it the window closes for real instead of hiding
            // (see CloseRequested below). On Linux libappindicator panics
            // rather than fail when neither appindicator library is installed,
            // so the panic is caught here and read as "no tray".
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                build_tray(app.handle())
            })) {
                Ok(Ok(())) => {}
                Ok(Err(err)) => logger::log("error", "app", &format!("no tray icon: {err}")),
                Err(_) => logger::log(
                    "error",
                    "app",
                    "no tray icon: the tray library panicked (libayatana-appindicator3 missing?)",
                ),
            }
            #[cfg(desktop)]
            install_signal_handlers();
            #[cfg(desktop)]
            fit_main_window_to_screen(app.handle());

            Ok(())
        });

    #[cfg(desktop)]
    let builder = builder
        .on_window_event(|window, event| match event {
            // Red-X / Cmd+W hides to the tray; the VPN keeps running.
            // Tray → Выйти, or Cmd+Q, stops everything. Without a tray (a
            // Linux desktop with no AppIndicator host) a hidden window could
            // never be brought back, so it closes instead.
            WindowEvent::CloseRequested { api, .. }
                if window.app_handle().tray_by_id("main-tray").is_some() =>
            {
                api.prevent_close();
                close_to_tray(window);
            }
            // «Уведомления»: не слать тост, пока окно на экране - в нём и так
            // видно то же самое щитом. `Focused(false)` приходит и когда окно
            // СКРЫТО (Cmd+W) - а это как раз момент, когда уведомление нужнее
            // всего, так что это ровно нужная сторона проверки.
            WindowEvent::Focused(focused) => {
                if let Some(core) = window.app_handle().try_state::<Arc<Core>>() {
                    core.window_focused.store(*focused, std::sync::atomic::Ordering::Relaxed);
                }
            }
            _ => {}
        });

    let app = builder
        .invoke_handler(tauri::generate_handler![
            vpn_snapshot,
            vpn_connect,
            vpn_disconnect,
            vpn_status,
            vpn_ping,
            list_locations,
            measure_locations,
            tunnel_prefs_get,
            tunnel_prefs_set,
            tunnel_rules_set,
            notify_prefs_get,
            notify_prefs_set,
            set_ui_lang,
            select_location,
            sub_state,
            sub_set,
            sub_clear,
            sub_refresh,
            routing_get,
            routing_set,
            traffic_split,
            diag_check,
            diag_fix,
            report_build,
            timeline_list,
            logs_page,
            pair_start,
            pair_poll,
            redeem_pair_code,
            onboarding_state,
            onboarding_run,
            app_info,
            open_external_unelevated,
            motion::motion_start,
            motion::motion_stop
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_handle, event| {
        if let RunEvent::Exit = event {
            #[cfg(desktop)]
            sync_cleanup();
        }
    });
}

#[cfg(test)]
mod tests {

    /// A list a connect or repair just failed with must not be reused by the
    /// Retry: step C4 re-reads the subscription through the broken tunnel and
    /// cannot be relied on, so the 15-minute reuse kept a stale list alive.
    #[test]
    fn a_list_that_ended_in_failed_is_fetched_again_on_retry() {
        let mut s = Session::new();
        s.servers = vec![vless("Германия")];
        s.sub_fetched_at = Some(1_000);
        assert_eq!(s.reusable_sub_age(61_000), Some(60_000));
        assert_eq!(s.reusable_sub_age(1_000 + SUB_REUSE_MS), None, "too old");

        s.sub_reuse_blocked = true;
        assert_eq!(s.reusable_sub_age(61_000), None);

        let empty = Session::new();
        assert_eq!(empty.reusable_sub_age(0), None);
    }

    /// The QR slot is writable by anyone who reads the token off the screen;
    /// a link it returns must be https on one of our own sites, exactly like
    /// one a pair code returns.
    #[test]
    fn a_paired_link_from_the_qr_slot_must_be_ours_and_https() {
        let ours = r#"{"status":"ready","subUrl":"https://proxysvpn.com/api/sub/0123456789abcdef0123456789abcdef"}"#;
        assert!(matches!(read_pair_answer(ours), Ok(PairOutcome::Linked(url)) if url.starts_with("https://proxysvpn.com/")));

        for planted in [
            r#"{"status":"ready","subUrl":"http://evil.example/s/x"}"#,
            r#"{"status":"ready","subUrl":"https://evil.example/s/x"}"#,
            r#"{"status":"ready","subUrl":"http://proxysvpn.com/api/sub/0123456789abcdef0123456789abcdef"}"#,
            r#"{"status":"ready","subUrl":"https://203.0.113.9/api/sub/x"}"#,
            r#"{"status":"ready"}"#,
        ] {
            let refused = read_pair_answer(planted);
            assert!(
                matches!(&refused, Err(e) if e.code == ErrorCode::SubInvalid),
                "{planted}"
            );
        }
        assert!(matches!(read_pair_answer(r#"{"status":"pending"}"#), Ok(PairOutcome::Waiting)));
        assert!(matches!(read_pair_answer(r#"{"status":"gone"}"#), Ok(PairOutcome::Expired)));
        assert!(read_pair_answer("not json").is_err());
    }

    /// A dead tun2socks takes the device and both half-defaults with it; the
    /// probe then reaches the site straight from the real address. That must
    /// not turn the shield green ("Protection restored") during repair.
    #[test]
    fn a_probe_that_passed_without_the_tunnel_does_not_restore_protection() {
        assert_eq!(healing_verdict(Some(ProbeVerdict::Passed), false), None);
        assert_eq!(healing_verdict(Some(ProbeVerdict::Unconfirmed), false), None);
        assert_eq!(healing_verdict(Some(ProbeVerdict::Passed), true), Some(VpnPhase::On));
        assert_eq!(healing_verdict(Some(ProbeVerdict::Unconfirmed), true), Some(VpnPhase::Unconfirmed));
        assert_eq!(healing_verdict(None, true), None);
    }

    /// 1366x768 at 100 % leaves a 720 px work area under the taskbar, a 1080p
    /// laptop at 150 % 672 px; the 720 px window plus its caption did not fit
    /// either, and the first-run "Continue" button sat under the taskbar.
    #[test]
    fn the_window_is_fitted_to_a_short_work_area_and_left_alone_otherwise() {
        // Fits: untouched.
        assert_eq!(fitted_inner_height(1040, 751, 720, 520), None);
        assert_eq!(fitted_inner_height(751, 751, 720, 520), None);
        // 672 px work area, 31 px caption: the content shrinks to 641.
        assert_eq!(fitted_inner_height(672, 751, 720, 520), Some(641));
        // At 150 %: physical pixels throughout.
        assert_eq!(fitted_inner_height(1008, 1127, 1080, 780), Some(961));
        // Never below the minimum, even on an absurd screen.
        assert_eq!(fitted_inner_height(400, 751, 720, 520), Some(520));
    }

    /// The window must be resizable and allowed below 720 px: with the old
    /// fixed 480x720 nobody on a short screen could reach the pinned footer.
    #[test]
    fn the_window_config_lets_a_short_screen_reach_the_footer() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses");
        let window = &conf["app"]["windows"][0];
        assert_eq!(window["resizable"], true, "{window}");
        let min_height = window["minHeight"].as_f64().expect("minHeight");
        assert!(min_height < 720.0, "{window}");
        assert!((min_height - MIN_WINDOW_HEIGHT).abs() < f64::EPSILON, "fit and config agree");
        assert!(window["minWidth"].as_f64().is_some_and(|w| w <= 480.0), "{window}");
    }

    #[test]
    fn green_by_warm_up_is_checked_at_the_warm_up_pace_until_a_probe_answers() {
        let now = Instant::now();
        let open = Some(now + Duration::from_secs(30));
        let closed = Some(now - Duration::from_secs(1));
        // Green by the warm-up alone: asked again within seconds.
        assert!(settling_applies(VpnPhase::On, true, open, now));
        // Green that a probe stood behind: idle pace.
        assert!(!settling_applies(VpnPhase::On, false, open, now));
        // Unconfirmed: as before, within the window.
        assert!(settling_applies(VpnPhase::Unconfirmed, false, open, now));
        // The window is shut: nobody gets the fast pace.
        assert!(!settling_applies(VpnPhase::On, true, closed, now));
        assert!(!settling_applies(VpnPhase::Unconfirmed, false, None, now));
        // Off never settles.
        assert!(!settling_applies(VpnPhase::Off, true, open, now));
    }
    use super::*;
    use subscription::{Hy2Config, VlessConfig};

    fn vless(remark: &str) -> ServerConfig {
        ServerConfig::Vless(VlessConfig {
            uuid: "u".into(),
            host: "node.example".into(),
            port: 443,
            encryption: "none".into(),
            public_key: "pk".into(),
            short_id: "ab".into(),
            sni: "s".into(),
            fingerprint: "chrome".into(),
            flow: String::new(),
            spider_x: String::new(),
            remark: remark.into(),
            transport: crate::subscription::VlessTransport::Tcp,
        })
    }

    /// The race partner speaks another transport, same country first.
    #[test]
    fn the_race_partner_is_another_transport_and_the_same_exit_first() {
        let mut s = Session::new();
        let mut xhttp = match vless("🇬🇧 Британия · XHTTP") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        xhttp.transport = crate::subscription::VlessTransport::Xhttp {
            path: "/p".into(),
            mode: crate::subscription::XhttpMode::StreamOne,
            host: None,
        };
        let mut vision_uk = match vless("🇬🇧 Британия") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        vision_uk.flow = "xtls-rprx-vision".into();
        let mut vision_de = match vless("🇩🇪 Германия") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        vision_de.flow = "xtls-rprx-vision".into();
        s.servers = vec![
            ServerConfig::Vless(vision_de),
            ServerConfig::Vless(vision_uk),
            ServerConfig::Vless(xhttp),
        ];
        // Chosen: Britain on Vision; partner: Britain on XHTTP (same exit).
        assert_eq!(s.race_partner(1), Some(2));
        // Chosen: Germany on Vision; the only other transport is XHTTP.
        assert_eq!(s.race_partner(0), Some(2));
        // Chosen: XHTTP; partner: Vision in the same country, not Germany.
        assert_eq!(s.race_partner(2), Some(1));
        // A demoted partner is not raced.
        let id = s.id_of(2).unwrap();
        s.demoted.insert(id, Instant::now() + Duration::from_secs(60));
        assert_eq!(s.race_partner(0), None, "no other transport left to race");
    }

    /// A known network starts on its own winner; an unknown one as before.
    #[test]
    fn each_network_starts_on_the_location_that_last_worked_there() {
        let mut s = Session::new();
        s.servers = vec![vless("🇩🇪 Германия"), vless("🇬🇧 Британия · XHTTP"), vless("🇫🇷 Франция")];
        let ids: Vec<String> = (0..3).map(|i| s.id_of(i).expect("id")).collect();
        s.last_good = Some(ids[0].clone());
        s.net_memory.remember("office", &ids[1], now_ms());

        s.network = Some("office".into());
        assert_eq!(s.choose_server(TransportPref::Auto), Some(1), "the office's own winner beats the global last good");

        s.network = Some("cafe".into());
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0), "an unknown network falls back to the global last good");

        s.network = None;
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0), "no network id: exactly the old behaviour");

        // The office winner stops getting through: demoted and forgotten there.
        s.network = Some("office".into());
        s.demote(1);
        assert_eq!(s.net_memory.winner("office", now_ms()), None, "a demoted winner is forgotten");
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0));
    }

    /// The "known network" gate at connect and `choose_server`'s own
    /// network-winner step must never disagree about the same winner.
    ///
    /// Before `network_winner_usable` unified them, the gate asked only
    /// whether `net_memory` had an entry — ignoring demotion — while
    /// `choose_server` also checked `demoted`. A winner demoted through some
    /// path other than `demote()` itself (for instance surviving a spell with
    /// `network: None`, see the test below) left the gate saying "known,
    /// no race" while `choose_server` quietly started on someone else.
    #[test]
    fn known_network_gate_agrees_with_choose_server_about_demotion() {
        let mut s = Session::new();
        s.servers = vec![vless("🇩🇪 Германия"), vless("🇬🇧 Британия")];
        let ids: Vec<String> = (0..2).map(|i| s.id_of(i).expect("id")).collect();
        s.network = Some("office".into());
        s.net_memory.remember("office", &ids[0], now_ms());

        // Fresh, not demoted: both the predicate and choose_server agree.
        assert_eq!(s.network_winner_usable("office"), Some(0));
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0));

        // The remembered winner is demoted directly, bypassing `demote()`.
        s.demoted.insert(ids[0].clone(), Instant::now() + Duration::from_secs(60));
        assert_eq!(
            s.network_winner_usable("office"),
            None,
            "a demoted winner must not read as usable"
        );
        assert_ne!(
            s.choose_server(TransportPref::Auto),
            Some(0),
            "choose_server must actually skip it too, in agreement with the predicate"
        );

        // An id the current subscription no longer has must also be "not
        // usable" — present in memory, but nothing to point `choose_server`
        // at any more.
        s.net_memory.remember("office", "gone-from-the-list", now_ms());
        assert_eq!(s.network_winner_usable("office"), None);
    }

    /// A network we could not fingerprint must not touch ANOTHER network's
    /// memory. This is the invariant `mend_routes` relies on after a route
    /// repair: when the fresh fingerprint comes back `None`, `s.network` is
    /// set to `None`, and neither `remember` nor `forget` may run against
    /// whatever network id happens to still be sitting in `net_memory` from
    /// before.
    #[test]
    fn demote_does_not_touch_the_memory_when_the_network_is_unknown() {
        let mut s = Session::new();
        s.servers = vec![vless("🇩🇪 Германия")];
        let id = s.id_of(0).unwrap();
        s.net_memory.remember("stale-network", &id, now_ms());
        // As if a route repair just ran and the new network could not be
        // fingerprinted (no gateway hardware address, no reply at all).
        s.network = None;

        s.demote(0);

        assert_eq!(
            s.net_memory.winner("stale-network", now_ms()),
            Some(id.as_str()),
            "an unresolved network must not forget another network's entry"
        );
        assert!(s.demoted.contains_key(&id), "the node itself is still demoted, network or not");
    }

    /// `pending_race` must only ever be consumed by the exact `start_on` call
    /// it was computed for.
    ///
    /// A plain `.take()` handed a race to whichever `start_on` ran next —
    /// generation and index be damned. A stale repair-ladder `start_on` from
    /// an older generation, or one raising a different index, must come away
    /// empty-handed and leave the pending race for its rightful caller.
    #[test]
    fn pending_race_is_scoped_to_its_own_generation_and_index() {
        let mut s = Session::new();
        s.pending_race = Some(PendingRace { generation: 5, for_index: 2, partner_index: 7 });

        assert_eq!(s.take_race_partner(4, 2), None, "an older generation must not steal it");
        assert_eq!(s.take_race_partner(5, 0), None, "a start_on for a different index must not either");
        // Both misses above must have left it untouched for its real caller.
        assert_eq!(s.take_race_partner(5, 2), Some(7), "the matching start_on gets it");
        assert_eq!(s.take_race_partner(5, 2), None, "and only once — nobody gets it twice");
    }

    /// Автовыбор обязан брать САМЫЙ БЫСТРЫЙ из замеренных, а не первый в списке.
    ///
    /// До 22.09.2026 он брал первый живой, то есть выбор определялся порядком,
    /// в котором подписка перечислила страны. У владельца первой шла Германия,
    /// и приложение бралось за неё, хотя Амстердам отвечал вдвое быстрее.
    #[test]
    fn auto_pick_takes_the_fastest_measured_node() {
        let mut s = Session::new();
        s.servers = vec![vless("🇩🇪 Германия"), vless("🇳🇱 Амстердам"), vless("🇬🇧 Британия")];

        // Без замеров - прежнее поведение: первый в списке.
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0), "без замеров берём первый живой");

        // С замерами - самый быстрый, где бы он ни стоял.
        let ids: Vec<String> = (0..3).map(|i| s.id_of(i).expect("идентификатор есть")).collect();
        s.rtt.insert(ids[0].clone(), 180);
        s.rtt.insert(ids[1].clone(), 42);
        s.rtt.insert(ids[2].clone(), 95);
        assert_eq!(s.choose_server(TransportPref::Auto), Some(1), "42 мс обязаны победить 180");

        // Использование САМО ПО СЕБЕ ничего не значит: мы весь вечер бились
        // об Германию, и она возглавляла список недавних, ни разу не
        // подтвердившись. Выбор такое игнорирует.
        s.remember_use(0);
        assert_eq!(
            s.choose_server(TransportPref::Auto),
            Some(1),
            "попытка - не успех; самый быстрый по-прежнему побеждает"
        );

        // А вот ПОДТВЕРЖДЁННЫЙ узел главнее: это факт, а замер - оценка.
        s.last_good = Some(ids[2].clone());
        assert_eq!(
            s.choose_server(TransportPref::Auto),
            Some(2),
            "последний подтвердившийся узел важнее самого быстрого на бумаге"
        );

        // Но только пока он жив: наказанный узел уступает.
        s.demoted.insert(ids[2].clone(), Instant::now() + Duration::from_secs(60));
        assert_eq!(s.choose_server(TransportPref::Auto), Some(1), "наказанный узел не выбирают");
    }

    /// Узел без замера не должен выигрывать вслепую у измеренного быстрого.
    #[test]
    fn an_unmeasured_node_does_not_outrank_a_measured_fast_one() {
        let mut s = Session::new();
        s.servers = vec![vless("🇩🇪 Германия"), vless("🇳🇱 Амстердам")];
        let ids: Vec<String> = (0..2).map(|i| s.id_of(i).expect("идентификатор есть")).collect();
        // Германию не мерили (или это Hysteria2, где рукопожатия TCP не бывает).
        s.rtt.insert(ids[1].clone(), 60);
        assert_eq!(
            s.choose_server(TransportPref::Auto),
            Some(1),
            "известное быстрое предпочтительнее неизвестного"
        );
    }

    /// A transport preference is a SOFT filter: it wins a tie inside a tier,
    /// but never leaves the app with no node at all where nothing of the
    /// wanted kind is reachable.
    #[test]
    fn transport_pref_prefers_a_match_but_never_leaves_the_app_with_nothing() {
        let mut xhttp = match vless("🇬🇧 Британия · XHTTP") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        xhttp.transport = crate::subscription::VlessTransport::Xhttp {
            path: "/p".into(),
            mode: crate::subscription::XhttpMode::StreamOne,
            host: None,
        };
        let mut vision = match vless("🇩🇪 Германия") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        vision.flow = "xtls-rprx-vision".into();

        let mut s = Session::new();
        s.servers = vec![ServerConfig::Vless(vision), ServerConfig::Vless(xhttp)];

        // Без предпочтения - прежнее поведение: первый в списке.
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0));
        // «Только XHTTP» пропускает Vision и берёт единственный XHTTP.
        assert_eq!(s.choose_server(TransportPref::XhttpOnly), Some(1));
        // «Только Vision» симметрично берёт индекс 0.
        assert_eq!(s.choose_server(TransportPref::VisionOnly), Some(0));

        // Список из одного Vision-узла: «только XHTTP» не может быть
        // выполнено, но приложение обязано вернуть хоть что-то, а не None.
        let mut only_vision = Session::new();
        only_vision.servers = vec![s.servers[0].clone()];
        assert_eq!(
            only_vision.choose_server(TransportPref::XhttpOnly),
            Some(0),
            "предпочтение не должно оставить приложение вовсе без узла"
        );
    }

    /// Память по сети и «последний удачный» тоже уважают предпочтение: сменив
    /// «Способ подключения», человек не должен молча вернуться на транспорт,
    /// от которого он только что отказался, только потому что тот когда-то
    /// сработал.
    #[test]
    fn transport_pref_overrides_stale_memory_of_the_wrong_transport() {
        let mut xhttp = match vless("🇬🇧 Британия · XHTTP") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        xhttp.transport = crate::subscription::VlessTransport::Xhttp {
            path: "/p".into(),
            mode: crate::subscription::XhttpMode::StreamOne,
            host: None,
        };
        let mut vision = match vless("🇩🇪 Германия") {
            ServerConfig::Vless(c) => c,
            _ => unreachable!(),
        };
        vision.flow = "xtls-rprx-vision".into();

        let mut s = Session::new();
        s.servers = vec![ServerConfig::Vless(vision), ServerConfig::Vless(xhttp)];
        let ids: Vec<String> = (0..2).map(|i| s.id_of(i).expect("id")).collect();

        // Vision «подтвердился последним» - память ядра.
        s.last_good = Some(ids[0].clone());
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0));

        // Человек только что переключился на «только XHTTP» - память о Vision
        // не должна победить свежий выбор.
        assert_eq!(s.choose_server(TransportPref::XhttpOnly), Some(1));
    }

    fn hy2(remark: &str) -> ServerConfig {
        ServerConfig::Hy2(Hy2Config {
            password: "p".into(),
            host: "node2.example".into(),
            port: 443,
            sni: "s".into(),
            pin_sha256: String::new(),
            insecure: false,
            remark: remark.into(),
        })
    }

    fn session_with(servers: Vec<ServerConfig>) -> Session {
        let mut s = Session::new();
        s.servers = servers;
        s
    }

    // ── labels and ids ─────────────────────────────────────────────────────

    #[test]
    fn a_flag_and_a_badge_come_from_the_server_not_from_us() {
        let (flag, label, note) = split_label("🇺🇸 США · 12,4 из 50 ГБ");
        assert_eq!(flag.as_deref(), Some("🇺🇸"));
        assert_eq!(label, "США");
        assert_eq!(note.as_deref(), Some("12,4 из 50 ГБ"));
    }

    #[test]
    fn a_plain_label_has_neither() {
        let (flag, label, note) = split_label("Нидерланды");
        assert!(flag.is_none());
        assert_eq!(label, "Нидерланды");
        assert!(note.is_none());
    }

    #[test]
    fn an_empty_remark_never_becomes_an_address() {
        // The old list put `host` on screen whenever a remark was missing.
        let s = session_with(vec![vless("")]);
        let list = s.locations();
        assert_eq!(list[0].label, "#1");
        assert!(!list[0].label.contains("node.example"));
    }

    #[test]
    fn location_ids_survive_an_address_change_and_are_stable() {
        let first = session_with(vec![vless("🇳🇱 Нидерланды")]).locations();
        // Same label, different node behind it — which is what happens every
        // time an address is burned and replaced.
        let mut moved = vless("🇳🇱 Нидерланды");
        if let ServerConfig::Vless(cfg) = &mut moved {
            cfg.host = "another.example".into();
        }
        let second = session_with(vec![moved]).locations();
        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn two_nodes_behind_one_label_do_not_share_an_id() {
        let s = session_with(vec![vless("Германия"), vless("Германия")]);
        let list = s.locations();
        assert_ne!(list[0].id, list[1].id);
    }

    #[test]
    fn nothing_in_a_location_entry_can_carry_an_address() {
        let s = session_with(vec![vless("🇩🇪 Германия")]);
        let json = serde_json::to_string(&s.locations()).expect("json");
        assert!(!json.contains("node.example"));
        assert!(!json.contains("443"));
    }

    // ── choosing a node ────────────────────────────────────────────────────

    #[test]
    fn a_pinned_country_wins_while_it_is_young() {
        let mut s = session_with(vec![vless("A"), vless("B")]);
        let second = s.id_of(1).expect("id");
        s.pinned = Some(Pinned {
            id: second,
            at: Instant::now(),
        });
        assert_eq!(s.choose_server(TransportPref::Auto), Some(1));
    }

    #[test]
    fn a_pin_older_than_a_day_stops_deciding() {
        let mut s = session_with(vec![vless("A"), vless("B")]);
        let second = s.id_of(1).expect("id");
        s.pinned = Some(Pinned {
            id: second,
            at: Instant::now() - PIN_LIFETIME - Duration::from_secs(1),
        });
        assert_eq!(s.choose_server(TransportPref::Auto), Some(0));
    }

    #[test]
    fn a_demoted_country_is_passed_over_but_not_removed() {
        let mut s = session_with(vec![vless("A"), vless("B")]);
        s.demote(0);
        assert_eq!(s.choose_server(TransportPref::Auto), Some(1));
        // Still on the list, and named honestly rather than hidden.
        assert_eq!(s.locations()[0].quality, LocationQuality::Blocked);
    }

    #[test]
    fn when_everything_is_demoted_we_still_try_something() {
        let mut s = session_with(vec![vless("A"), vless("B")]);
        s.demote(0);
        s.demote(1);
        assert!(s.choose_server(TransportPref::Auto).is_some());
    }

    #[test]
    fn the_last_working_country_is_tried_first() {
        let mut s = session_with(vec![vless("A"), vless("B"), vless("C")]);
        // РАБОТАВШИЙ, а не просто использованный. Тест назывался так с самого
        // начала, но проверял `remember_use` - то есть сам факт попытки. Из-за
        // этого расхождения приложение весь вечер 22.09.2026 бралось за узел,
        // который ни разу не подтвердился: он просто чаще других оказывался в
        // недавних, потому что об него и бились.
        s.remember_use(2);
        assert_eq!(
            s.choose_server(TransportPref::Auto),
            Some(0),
            "одной попытки мало: узел ещё ничего не доказал"
        );

        s.last_good = s.id_of(2);
        assert_eq!(s.choose_server(TransportPref::Auto), Some(2), "подтвердившийся узел берут первым");
    }

    #[test]
    fn the_next_candidate_can_hold_or_change_protocol() {
        let s = session_with(vec![vless("A"), hy2("B"), vless("C")]);
        assert_eq!(s.next_server(0, true), Some(2));
        assert_eq!(s.next_server(0, false), Some(1));
    }

    #[test]
    fn a_single_node_has_no_next() {
        let s = session_with(vec![vless("A")]);
        assert_eq!(s.next_server(0, true), None);
    }

    // ── the subscription link ──────────────────────────────────────────────

    #[test]
    fn only_a_real_subscription_link_is_accepted() {
        assert!(validate_link("https://proxysvpn.com/api/sub/abc").is_ok());
        assert!(validate_link("  https://proxysvpn.com/api/sub/abc  ").is_ok());
        assert!(validate_link("vless://uuid@host:443").is_err());
        assert!(validate_link("https://proxysvpn.com").is_err());
        assert!(
            validate_link("http://proxysvpn.com/api/sub/abc").is_err(),
            "the token must not travel in clear text"
        );
        assert!(validate_link("").is_err());
        assert!(validate_link("not a url").is_err());
    }

    #[test]
    fn being_abroad_is_asked_for_in_the_link_not_invented_locally() {
        let home = effective_sub_url("https://proxysvpn.com/api/sub/t", true).expect("url");
        assert!(!home.contains("split"));
        let away = effective_sub_url("https://proxysvpn.com/api/sub/t", false).expect("url");
        assert!(away.contains("split=0"));
    }

    #[test]
    fn switching_back_removes_the_flag_rather_than_stacking_it() {
        let away = effective_sub_url("https://proxysvpn.com/api/sub/t?split=0", false).expect("a");
        assert_eq!(away.matches("split=0").count(), 1);
        let home = effective_sub_url(&away, true).expect("b");
        assert!(!home.contains("split"));
    }

    #[test]
    fn other_parameters_of_a_link_are_left_alone() {
        let url = effective_sub_url("https://proxysvpn.com/api/sub/t?lang=en", false).expect("url");
        assert!(url.contains("lang=en"));
        assert!(url.contains("split=0"));
    }

    // ── pairing ────────────────────────────────────────────────────────────

    #[test]
    fn only_the_token_shape_the_cabinet_scans_is_accepted() {
        assert!(is_pair_token("0123456789abcdef0123456789abcdef"));
        // The cabinet's own regex is lowercase-only.
        assert!(!is_pair_token("0123456789ABCDEF0123456789ABCDEF"));
        assert!(!is_pair_token("short"));
        assert!(!is_pair_token("0123456789abcdef0123456789abcdeg"));
    }

    #[test]
    fn the_pairing_ladder_and_the_subscription_ladder_name_the_same_sites() {
        // A pair-code link is kept only on a SITE_LADDER name; the
        // subscription is then fetched over RESERVE_HOSTS. A site added to one
        // and not the other would either refuse good links or fetch from a
        // name the code never vouched for.
        let mut pairing: Vec<String> = SITE_LADDER.iter().map(|h| h.to_ascii_lowercase()).collect();
        let mut fetching: Vec<String> =
            subscription::RESERVE_HOSTS.iter().map(|h| h.to_ascii_lowercase()).collect();
        pairing.sort();
        fetching.sort();
        assert_eq!(pairing, fetching);
    }

    #[test]
    fn a_link_a_pair_code_delivers_is_one_the_store_keeps() {
        // pair_code::accept_pair_link is the stricter gate; whatever it lets
        // through must also pass the gate every stored link goes through.
        for host in SITE_LADDER {
            let raw = format!("https://{host}/api/sub/abcdef0123456789?lang=en");
            let accepted = pair_code::accept_pair_link(&raw, &SITE_LADDER).expect("ours");
            assert_eq!(validate_link(&accepted).map_err(|e| e.code), Ok(accepted.clone()));
        }
    }

    // ── local facts ────────────────────────────────────────────────────────

    #[test]
    fn the_region_guess_reads_the_machines_own_timezone() {
        assert_eq!(guess_region("Europe/Moscow"), Some("ru"));
        assert_eq!(guess_region("Asia/Vladivostok"), Some("ru"));
        assert_eq!(guess_region("Europe/Berlin"), Some("abroad"));
        // No timezone is not a guess of "abroad".
        assert_eq!(guess_region(""), None);
    }

    #[test]
    fn a_clock_outside_the_window_is_a_finding_in_both_directions() {
        // A machine that thinks it is 2015 and one that thinks it is 2099
        // both fail every TLS handshake, and both blame us for it.
        assert_eq!(clock_verdict(1_758_400_001), CheckState::Pass);
        assert_eq!(clock_verdict(1_440_000_000), CheckState::Fail);
        assert_eq!(clock_verdict(4_000_000_000), CheckState::Fail);
        assert_eq!(clock_is_plausible(), CheckState::Pass);
    }

    #[test]
    fn repair_backs_off_and_then_stops_growing() {
        assert_eq!(heal_backoff(0), Duration::from_secs(30));
        assert_eq!(heal_backoff(3), Duration::from_secs(900));
        // A network that has been down all day must not index past the table.
        assert_eq!(heal_backoff(99), Duration::from_secs(900));
    }

    #[test]
    fn a_name_the_service_never_sent_has_no_flag_and_no_badge() {
        // Labels arrive from the server; assembling one here is how "США ·
        // 12,4 из 50 ГБ" would drift from what the cabinet shows.
        let (flag, label, note) = split_label("  ");
        assert!(flag.is_none());
        assert!(label.is_empty());
        assert!(note.is_none());
    }

    // ── metric bookkeeping ─────────────────────────────────────────────────

    #[test]
    fn proof_of_life_is_not_erased_by_a_later_failure() {
        // Otherwise "проверено 9 секунд назад" would vanish the moment one
        // probe missed, and the window could no longer age it honestly.
        let mut s = Session::new();
        s.absorb_metric(MetricPayload {
            last_proof_at: Some(1_000),
            ..Default::default()
        });
        s.absorb_metric(MetricPayload {
            rx_bytes: 5,
            ..Default::default()
        });
        assert_eq!(s.metric.last_proof_at, Some(1_000));
        assert_eq!(s.metric.rx_bytes, 5);
    }

    #[test]
    fn a_fresh_proof_replaces_an_older_one() {
        let mut s = Session::new();
        s.absorb_metric(MetricPayload {
            last_proof_at: Some(1_000),
            ..Default::default()
        });
        s.absorb_metric(MetricPayload {
            last_proof_at: Some(2_000),
            ..Default::default()
        });
        assert_eq!(s.metric.last_proof_at, Some(2_000));
    }

    #[test]
    fn quality_words_never_call_an_unmeasurable_link_slow() {
        assert_eq!(
            LocationQuality::from_link(LinkQuality::NotApplicable),
            LocationQuality::Unknown
        );
        assert_eq!(
            LocationQuality::from_link(LinkQuality::Good),
            LocationQuality::Good
        );
    }

    // ── the timeline ───────────────────────────────────────────────────────

    #[test]
    fn the_timeline_keeps_the_newest_five_hundred() {
        let mut s = Session::new();
        for _ in 0..TIMELINE_LIMIT + 10 {
            s.push_event(TimelineCode::Connected, None);
        }
        assert_eq!(s.timeline.len(), TIMELINE_LIMIT);
    }

    #[test]
    fn the_state_payload_only_carries_what_the_contract_names() {
        let s = Session::new();
        let json = serde_json::to_string(&s.state_payload()).expect("json");
        assert_eq!(json, r#"{"phase":"off"}"#);
    }

    #[test]
    fn healing_time_reaches_the_window_so_it_can_stay_silent_then_whisper() {
        let mut s = Session::new();
        s.phase = VpnPhase::Healing;
        s.healing_since = Some(Instant::now());
        let json = serde_json::to_string(&s.state_payload()).expect("json");
        assert!(json.contains("healingForMs"), "{json}");
    }

    // ── shapes the window parses by hand ───────────────────────────────────

    #[test]
    fn the_service_column_says_plainly_that_it_cannot_answer() {
        let json = serde_json::to_string(&CheckService::Unavailable {
            available: false,
            reason: "cabinetOnly",
        })
        .expect("json");
        assert_eq!(json, r#"{"available":false,"reason":"cabinetOnly"}"#);
    }

    #[test]
    fn the_region_field_is_present_even_when_we_have_no_opinion() {
        let json = serde_json::to_string(&RoutingState {
            in_russia: true,
            guess: None,
        })
        .expect("json");
        assert_eq!(json, r#"{"inRussia":true,"guess":null}"#);
    }

    #[test]
    fn log_lines_reach_the_window_in_the_case_it_reads() {
        let json = serde_json::to_string(&LogLineOut {
            ts_ms: 1,
            level: "info".into(),
            source: "app".into(),
            message: "x".into(),
        })
        .expect("json");
        assert!(json.contains("tsMs"), "{json}");
    }

    /// The other half of `CheckService`, which no running code constructs yet.
    ///
    /// It is tested precisely BECAUSE it is unreachable: an untaken branch is
    /// where a wire name rots unnoticed, and this one did — it serialised
    /// `can_fix` while src/bridge.ts read `canFix`, so the repair button would
    /// have been invisible on the first day the backend offered a repair.
    #[test]
    fn the_service_column_keeps_camel_case_in_the_branch_nobody_walks_yet() {
        let json = serde_json::to_string(&CheckService::Available {
            available: true,
            rows: vec![CheckRow {
                id: "balance",
                state: CheckState::Pass,
                via_fallback: false,
            }],
            can_fix: true,
        })
        .expect("json");
        assert!(json.contains(r#""canFix":true"#), "{json}");
        assert!(!json.contains("can_fix"), "{json}");
    }

    fn consent_temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("proxysvpn-onboarding-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Guideline 5.4: the data notice comes before everything else on the
    /// first run, and once accepted it does not come back.
    #[test]
    fn the_data_notice_is_the_first_step_until_it_is_accepted() {
        let dir = consent_temp_dir("first");
        let paths = vec![dir.join(consent::FILE_NAME)];

        let before = onboarding_steps(&paths);
        assert_eq!(before.first(), Some(&"dataNotice"), "{before:?}");

        consent::record(&paths, 1_700_000_000_000).expect("record");
        let after = onboarding_steps(&paths);
        assert!(!after.contains(&"dataNotice"), "{after:?}");
        // Only the notice changed; any platform step is still there.
        assert_eq!(after.len(), before.len() - 1, "{before:?} -> {after:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_older_or_damaged_record_brings_the_notice_back() {
        let dir = consent_temp_dir("again");
        let path = dir.join(consent::FILE_NAME);
        let paths = vec![path.clone()];

        std::fs::write(&path, br#"{"v":0,"at":1}"#).expect("write");
        assert_eq!(onboarding_steps(&paths).first(), Some(&"dataNotice"));

        std::fs::write(&path, b"{\"v\":1,").expect("write");
        assert_eq!(onboarding_steps(&paths).first(), Some(&"dataNotice"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The record sits beside the subscription link, in every place the link
    /// may live, so it survives exactly as long as the link's directory does.
    #[test]
    fn the_record_lives_beside_the_subscription_link() {
        let links = link_paths();
        let records = consent_paths();
        assert_eq!(links.len(), records.len());
        for (link, record) in links.iter().zip(&records) {
            assert_eq!(link.parent(), record.parent());
            assert_eq!(record.file_name().and_then(|n| n.to_str()), Some(consent::FILE_NAME));
        }
    }
}
