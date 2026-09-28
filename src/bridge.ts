// src/bridge.ts
//
// The ONLY place the window talks to the core.
//
// Two implementations behind one interface:
//   • Tauri — real `invoke` + `listen`, used inside the app;
//   • Mock  — a scripted core with believable data, used whenever
//     `window.__TAURI_INTERNALS__` is absent (plain `npm run dev`).
//
// Why a mock at all. The screens have to be shown to the owner and clicked
// through before the Rust half of DESIGN.md exists, and a screen that can only
// be seen in a signed build is a screen nobody reviews. The mock plays every
// state the design names — off, starting step by step, protected, healing,
// unconfirmed, failed — plus every code in `ERROR_ACTION`.
//
//   Pick a scenario with `?mock=<name>`, e.g.
//     ?mock=healing        — drops into recovery and comes back
//     ?mock=healing-long   — recovery that drags past 8 s and then fails
//     ?mock=DEVICE_TAKEN   — any ErrorCode works as a scenario name
//   `MOCK_SCENARIOS` below is the full list; the demo strip in the corner
//   (dev builds only) switches between them without typing.
//
// ── The command surface the core must provide ──────────────────────────────
// Every `invoke` name used here, in one place, so the Rust side has a list:
//
//   vpn_snapshot   -> VpnSnapshot      everything at once, for a fresh window
//   vpn_connect    -> ()               phase changes arrive as events
//   vpn_disconnect -> ()               also used as "cancel" while starting
//   list_locations -> LocationEntry[]  labels and quality WORDS, never addresses
//   select_location{ id: string|null } -> ()   null = automatic
//   sub_state      -> SubState
//   sub_set{ link } -> ()
//   sub_clear      -> ()
//   sub_refresh    -> SubMeta
//   pair_start     -> PairSession
//   pair_poll{ token } -> PairStatus
//   routing_get    -> RoutingState
//   routing_set{ inRussia } -> ()
//   traffic_split  -> TrafficSplit | null   connection counts, never addresses
//   diag_check     -> CheckReport
//   diag_fix       -> CheckReport
//   report_build   -> SupportReport
//   timeline_list{ limit } -> TimelineEntry[]
//   logs_page{ offset, limit } -> LogLine[]
//   onboarding_state -> OnboardingState
//   onboarding_run{ step } -> ()
//   app_info       -> AppInfo
//
// Commands answer `Result<T, String>` with `AppError::to_payload()` in the
// error, exactly as errors.rs describes; `call()` below turns that back into
// an `AppError`.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";

import { IS_APPSTORE } from "./dist";
import { isAllowedExternal } from "./externalUrl";
import {
  ERROR_ACTION,
  EV,
  parseAppError,
  type AppError,
  type ErrorCode,
  type EventPayload,
  type MetricPayload,
  type StatePayload,
  type SubMeta,
  type VpnSnapshot,
  type VpnStep,
} from "./types";

// ── Types that are NOT part of the core contract ────────────────────────────
//
// events.rs and types.ts own the tunnel's state. Everything below is transport
// for a single screen and deliberately lives here instead of being bolted onto
// the contract — src/types.ts may only grow together with Rust, and that file
// is not ours to edit.
//
// Note what a `LocationEntry` does NOT carry: no host, no port. The rule "node
// addresses are never shown" is enforced by the shape of the data, not by
// remembering to leave a field out of the JSX. The protocol name IS carried
// since 27.09.2026 (owner's decision): it is a technology, not an address.

export type LocationQuality = "good" | "ok" | "poor" | "blocked" | "unknown";

/** Какие адреса спрашивать у резолвера. */
export type IpKind = "ipv4" | "ipv6" | "both";

/** Чей резолвер спрашивает ядро. */
export type DnsChoice = "internal" | "system" | "custom";

/**
 * «Способ подключения» (TunnelScreen). `auto` - сегодняшнее поведение: первый
 * кандидат, который умеет этот движок. Остальные два ограничивают выбор одним
 * транспортом; локация, где его нет, не пропадает из списка - она подключится
 * тем, что есть, и список это покажет.
 */
export type TransportPref = "auto" | "xhttpOnly" | "visionOnly";

/**
 * Настройки туннеля.
 *
 * Здесь только то, что ДЕЙСТВИТЕЛЬНО меняет работу. Мультиплексирования нет
 * намеренно: наши узлы идут с XTLS Vision, а он с ним несовместим, и тумблер
 * не делал бы ничего. Окно говорит об этом прямо, вместо того чтобы прятать.
 */
export interface TunnelPrefs {
  /** Дробить первое приветствие TLS - против DPI, который ищет имя по образцу. */
  fragment: boolean;
  ipKind: IpKind;
  dns: DnsChoice;
  /** Адрес для dns === "custom". Пустая строка = выбран, но не введён. */
  customDns: string;
  transport: TransportPref;
  /** «Свои правила» → «Всегда напрямую», уже проверенные строки в форме xray. */
  directDomains: string[];
  /** «Свои правила» → «Всегда через VPN», та же форма. */
  proxyDomains: string[];
}

/** Итог сохранения «Своих правил»: что не приняли, по каждому списку. */
export interface CustomRulesOutcome {
  directIgnored: string[];
  proxyIgnored: string[];
  /** true - туннель был поднят и переподключается. */
  reconnected: boolean;
}

/** «Уведомления» (MoreScreen): системные тосты о падении/восстановлении защиты. */
export interface NotifyPrefs {
  enabled: boolean;
}

export interface LocationEntry {
  /** Stable id from the core. Never an address. */
  id: string;
  /** Country as the subscription named it: "Нидерланды". */
  label: string;
  /** Flag emoji; the UI falls back to a globe when the core omits it. */
  flag?: string;
  quality: LocationQuality;
  /**
   * Рукопожатие TCP до узла, миллисекунды.
   *
   * Отсутствует, пока не померили, и у локаций на Hysteria2 - там порт UDP,
   * и рукопожатию TCP стучать некуда. Отсутствие числа рисуется словом, а не
   * нулём и не прочерком-ошибкой.
   */
  rttMs?: number;
  /** Badge the SERVER wrote, e.g. "12,4 из 50 ГБ". Never assembled here. */
  note?: string;
  /**
   * «VLESS · Vision», «VLESS · XHTTP», «Hysteria2» — a technology name from
   * the core, never an address or a port. Shown since 27.09.2026 by the
   * owner's decision: two «Британия» rows had nothing to tell them apart.
   */
  protocol: string;
  /** Among the last three the person used. */
  recent?: boolean;
  /** Pinned by hand for 24 hours. Nothing pinned means automatic. */
  selected?: boolean;
}

export interface SubState {
  hasLink: boolean;
  /** Unix ms of the last successful subscription fetch. */
  lastUpdatedAt?: number;
  /** The ladder had to leave the main address. Which one is not shown. */
  usedFallback?: boolean;
}

export interface AppInfo {
  version: string;
  platform: "macos" | "ios" | "other";
  /** The host that works for THIS person — the ladder already picked it. */
  cabinetUrl: string;
  botUrl: string;
  /** "MacBook" — for [10]. Never a serial number or an identifier. */
  deviceName: string;
  deviceLinkedAt?: number;
}

/** A pairing code: 32 hex characters, alive for five minutes. */
export interface PairSession {
  token: string;
  expiresAt: number;
}

export type PairStatus = "waiting" | "linked" | "expired";

export type CheckRowId =
  | "clock"
  | "internet"
  | "dns"
  | "sub"
  | "handshake"
  | "traffic"
  | "network"
  | "balance"
  | "profile"
  | "freshness"
  | "nodes";

export type CheckState = "idle" | "pending" | "pass" | "fail" | "skip";

export interface CheckRow {
  id: CheckRowId;
  state: CheckState;
  /** "запасной" next to a row that passed the hard way. */
  viaFallback?: boolean;
}

export type CheckService =
  | { available: false; reason: "cabinetOnly" | "failed" }
  | { available: true; rows: CheckRow[]; canFix: boolean };

export interface CheckReport {
  device: CheckRow[];
  service: CheckService;
  /** The one row that decides the verdict; null when everything passed. */
  verdict: CheckRowId | null;
}

export type TimelineCode =
  | "connected"
  | "disconnected"
  | "healed"
  | "trafficStopped"
  | "networkChanged"
  | "subRefreshed"
  | "subRefreshedFallback"
  | "locationSwitched"
  | "locationUnreadable"
  | "failed";

export interface TimelineEntry {
  atMs: number;
  code: TimelineCode;
  location?: string;
  network?: "wifi" | "mobile";
}

export interface LogLine {
  tsMs: number;
  level: "info" | "warn" | "error";
  source: string;
  message: string;
}

/**
 * Connections this session, split at the routing fork. Two numbers and no
 * addresses — the only honest form of "we are transparent about traffic".
 * `null` when the core cannot count them; the row then does not appear.
 */
export interface TrafficSplit {
  viaVpn: number;
  direct: number;
}

export interface RoutingState {
  /** true = Russian sites go directly (the person is in Russia). */
  inRussia: boolean;
  /** What we worked out ourselves; null when we have no opinion. */
  guess: "ru" | "abroad" | null;
}

/**
 * `dataNotice` is the declaration of the data the app uses (guideline 5.4);
 * the core puts it first on every platform until it has been seen.
 */
export type OnboardingStep = "dataNotice" | "moveToApplications" | "password" | "iosPermission";

export interface OnboardingState {
  steps: OnboardingStep[];
}

export interface SupportReport {
  text: string;
  bytes: number;
}

export interface BridgeHandlers {
  onState?: (state: StatePayload) => void;
  onStep?: (step: VpnStep) => void;
  onMetric?: (metric: MetricPayload) => void;
  onEvent?: (event: EventPayload) => void;
  onMeta?: (meta: SubMeta) => void;
}

export interface CoreBridge {
  subscribe(handlers: BridgeHandlers): () => void;

  snapshot(): Promise<VpnSnapshot>;
  connect(): Promise<void>;
  disconnect(): Promise<void>;

  locations(): Promise<LocationEntry[]>;
  tunnelPrefs(): Promise<TunnelPrefs>;
  /** Записать и применить. true - туннель был поднят и переподключается. */
  setTunnelPrefs(prefs: TunnelPrefs): Promise<boolean>;
  /** Проверить и сохранить «Свои правила» (по строке на домен в каждом поле). */
  setCustomRules(direct: string, proxy: string): Promise<CustomRulesOutcome>;
  notifyPrefs(): Promise<NotifyPrefs>;
  setNotifyPrefs(enabled: boolean): Promise<void>;
  /** Сообщить ядру текущий язык окна - для текста системных уведомлений. */
  setUiLang(lang: string): Promise<void>;
  /** Померить рукопожатие до каждой локации. Возвращает тот же список с числами. */
  measureLocations(): Promise<LocationEntry[]>;
  selectLocation(id: string | null): Promise<void>;

  subState(): Promise<SubState>;
  setSubscription(link: string): Promise<void>;
  clearSubscription(): Promise<void>;
  refreshSub(): Promise<SubMeta>;

  pairStart(): Promise<PairSession>;
  pairPoll(token: string): Promise<PairStatus>;

  routing(): Promise<RoutingState>;
  setRouting(inRussia: boolean): Promise<void>;
  trafficSplit(): Promise<TrafficSplit | null>;

  runCheck(): Promise<CheckReport>;
  runFix(): Promise<CheckReport>;
  buildReport(): Promise<SupportReport>;

  timeline(limit: number): Promise<TimelineEntry[]>;
  logs(offset: number, limit: number): Promise<LogLine[]>;

  onboarding(): Promise<OnboardingState>;
  onboardingRun(step: OnboardingStep): Promise<void>;

  appInfo(): Promise<AppInfo>;
  openExternal(url: string): Promise<void>;
  readClipboard(): Promise<string>;
  writeClipboard(text: string): Promise<void>;
}

/** An `AppError` wrapped in a real Error, so `throw` and `catch` behave. */
export class CoreError extends Error {
  readonly app: AppError;
  constructor(app: AppError) {
    super(app.code);
    this.name = "CoreError";
    this.app = app;
  }
}

/** Anything a `catch` caught, as an `AppError`. Never throws itself. */
export function toAppError(err: unknown): AppError {
  if (err instanceof CoreError) return err.app;
  return parseAppError(err);
}

/** True inside the app; false under `npm run dev` in a browser. */
export const IS_TAURI: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// ── The real bridge ─────────────────────────────────────────────────────────

/** Scheme and host of a URL for a log line — never its path or query. */
function urlOrigin(url: string): { scheme: string; host: string } {
  try {
    const parsed = new URL(url);
    return { scheme: parsed.protocol.replace(/:$/, ""), host: parsed.hostname };
  } catch {
    return { scheme: "unparsable", host: "" };
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    // Commands reject with `AppError::to_payload()`; anything else (a missing
    // command, a panic) still arrives as UNKNOWN with its text kept, so the
    // support report carries the truth even when we failed to name it.
    throw new CoreError(parseAppError(raw));
  }
}

class TauriBridge implements CoreBridge {
  subscribe(handlers: BridgeHandlers): () => void {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];

    const attach = <T>(name: string, handler: ((payload: T) => void) | undefined) => {
      if (!handler) return;
      void listen<T>(name, (event) => handler(event.payload))
        .then((unlisten) => {
          // The window can unmount before `listen` resolves; dropping the
          // handle here would leak a listener for the life of the process.
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        })
        .catch(() => {
          // A core too old to emit this event is not worth a broken screen:
          // `snapshot()` still tells the truth on its own.
        });
    };

    attach<StatePayload>(EV.state, handlers.onState);
    attach<{ step: VpnStep }>(EV.step, (payload) => handlers.onStep?.(payload.step));
    attach<MetricPayload>(EV.metric, handlers.onMetric);
    attach<EventPayload>(EV.event, handlers.onEvent);
    attach<SubMeta>(EV.meta, handlers.onMeta);

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) unlisten();
      unlisteners.length = 0;
    };
  }

  snapshot(): Promise<VpnSnapshot> {
    return call<VpnSnapshot>("vpn_snapshot");
  }
  connect(): Promise<void> {
    return call<void>("vpn_connect");
  }
  disconnect(): Promise<void> {
    return call<void>("vpn_disconnect");
  }
  measureLocations(): Promise<LocationEntry[]> {
    return call<LocationEntry[]>("measure_locations");
  }

  tunnelPrefs(): Promise<TunnelPrefs> {
    return call<TunnelPrefs>("tunnel_prefs_get");
  }

  setTunnelPrefs(prefs: TunnelPrefs): Promise<boolean> {
    return call<boolean>("tunnel_prefs_set", { prefs });
  }

  setCustomRules(direct: string, proxy: string): Promise<CustomRulesOutcome> {
    return call<CustomRulesOutcome>("tunnel_rules_set", { direct, proxy });
  }

  notifyPrefs(): Promise<NotifyPrefs> {
    return call<NotifyPrefs>("notify_prefs_get");
  }

  setNotifyPrefs(enabled: boolean): Promise<void> {
    return call<void>("notify_prefs_set", { enabled });
  }

  setUiLang(lang: string): Promise<void> {
    return call<void>("set_ui_lang", { lang });
  }

  locations(): Promise<LocationEntry[]> {
    return call<LocationEntry[]>("list_locations");
  }
  selectLocation(id: string | null): Promise<void> {
    return call<void>("select_location", { id });
  }
  subState(): Promise<SubState> {
    return call<SubState>("sub_state");
  }
  setSubscription(link: string): Promise<void> {
    return call<void>("sub_set", { link });
  }
  clearSubscription(): Promise<void> {
    return call<void>("sub_clear");
  }
  refreshSub(): Promise<SubMeta> {
    return call<SubMeta>("sub_refresh");
  }
  pairStart(): Promise<PairSession> {
    return call<PairSession>("pair_start");
  }
  pairPoll(token: string): Promise<PairStatus> {
    return call<PairStatus>("pair_poll", { token });
  }
  routing(): Promise<RoutingState> {
    return call<RoutingState>("routing_get");
  }
  setRouting(inRussia: boolean): Promise<void> {
    return call<void>("routing_set", { inRussia });
  }
  trafficSplit(): Promise<TrafficSplit | null> {
    return call<TrafficSplit | null>("traffic_split");
  }
  runCheck(): Promise<CheckReport> {
    return call<CheckReport>("diag_check");
  }
  runFix(): Promise<CheckReport> {
    return call<CheckReport>("diag_fix");
  }
  buildReport(): Promise<SupportReport> {
    return call<SupportReport>("report_build");
  }
  timeline(limit: number): Promise<TimelineEntry[]> {
    return call<TimelineEntry[]>("timeline_list", { limit });
  }
  logs(offset: number, limit: number): Promise<LogLine[]> {
    return call<LogLine[]>("logs_page", { offset, limit });
  }
  onboarding(): Promise<OnboardingState> {
    return call<OnboardingState>("onboarding_state");
  }
  onboardingRun(step: OnboardingStep): Promise<void> {
    return call<void>("onboarding_run", { step });
  }
  appInfo(): Promise<AppInfo> {
    return call<AppInfo>("app_info");
  }
  async openExternal(url: string): Promise<void> {
    // App Store builds open only the allowlisted pages (src/externalUrl.ts):
    // announce-url and support-url come from the service and can change after
    // review, and no reviewed button may end on a payment page. A refusal is
    // silent for the person — no screen offers such a link on purpose — and
    // logged without the path or query, which may carry a subscription token.
    // Direct builds keep the opener plugin's own scope as the only gate.
    if (IS_APPSTORE && !isAllowedExternal(url, true)) {
      console.warn({ event: "external_url_blocked", dist: "appstore", ...urlOrigin(url) });
      return;
    }
    try {
      await openUrl(url);
    } catch (raw) {
      throw new CoreError(parseAppError(raw));
    }
  }
  async readClipboard(): Promise<string> {
    try {
      return await readText();
    } catch (raw) {
      throw new CoreError(parseAppError(raw));
    }
  }
  async writeClipboard(text: string): Promise<void> {
    try {
      await writeText(text);
    } catch (raw) {
      throw new CoreError(parseAppError(raw));
    }
  }
}

// ── The mock ────────────────────────────────────────────────────────────────

/** Scenarios that are not simply "fail with this code". */
export const MOCK_SCENARIOS = [
  "off",
  "nolink",
  "onboarding",
  "connecting",
  "on",
  "unconfirmed",
  "healing",
  "healing-long",
  "announce",
  "service-check",
  "pair-expired",
] as const;

export type MockScenario = (typeof MOCK_SCENARIOS)[number] | ErrorCode;

const ERROR_CODES = Object.keys(ERROR_ACTION) as ErrorCode[];

function isErrorCode(value: string): value is ErrorCode {
  return (ERROR_CODES as string[]).includes(value);
}

/** `?mock=…`, validated. Anything unknown falls back to "off". */
export function readScenario(): MockScenario {
  if (typeof window === "undefined") return "off";
  const raw = new URLSearchParams(window.location.search).get("mock");
  if (!raw) return "off";
  if ((MOCK_SCENARIOS as readonly string[]).includes(raw)) return raw as MockScenario;
  const upper = raw.toUpperCase();
  if (isErrorCode(upper)) return upper;
  return "off";
}

/** Used by the demo strip: same page, different scenario. */
export function applyScenario(scenario: MockScenario): void {
  const url = new URL(window.location.href);
  url.searchParams.set("mock", scenario);
  window.location.href = url.toString();
}

const MOCK_LOCATIONS: LocationEntry[] = [
  { id: "nl", label: "Нидерланды", flag: "🇳🇱", quality: "good", recent: true, protocol: "Hysteria2" },
  { id: "de", label: "Германия", flag: "🇩🇪", quality: "ok", recent: true, protocol: "VLESS · Vision" },
  // `note` imitates the server-written label: the app never assembles it.
  { id: "us", label: "США", flag: "🇺🇸", quality: "unknown", note: "12,4 из 50 ГБ", protocol: "VLESS · Vision" },
  { id: "uk", label: "Великобритания", flag: "🇬🇧", quality: "blocked", protocol: "VLESS · Vision" },
  { id: "uk-x", label: "Великобритания", flag: "🇬🇧", quality: "good", note: "XHTTP", protocol: "VLESS · XHTTP" },
  { id: "se", label: "Швеция", flag: "🇸🇪", quality: "unknown", protocol: "VLESS · Vision" },
  { id: "fr", label: "Франция", flag: "🇫🇷", quality: "poor", protocol: "VLESS · Vision" },
];

const CONNECT_STEPS: [VpnStep, number][] = [
  ["fetchingSub", 900],
  ["pickingServer", 600],
  ["startingEngine", 1100],
  ["raisingTun", 1200],
  ["probing", 900],
];

const DEVICE_ROWS: CheckRowId[] = [
  "clock",
  "internet",
  "dns",
  "sub",
  "handshake",
  "traffic",
  "network",
];

class MockBridge implements CoreBridge {
  private readonly scenario: MockScenario;
  /** The core's consent record, for as long as this page lives. */
  private dataNoticeSeen = false;
  private readonly handlers = new Set<BridgeHandlers>();
  private readonly timers = new Set<number>();

  private state: StatePayload = { phase: "off" };
  private step: VpnStep | undefined;
  private metric: MetricPayload = { rxBytes: 0, txBytes: 0 };
  private meta: SubMeta;
  private hasLink: boolean;
  private lastUpdatedAt: number | undefined;
  private selected: string | null = null;
  private current = "nl";
  private routingState: RoutingState = { inRussia: true, guess: "ru" };
  private healingSince = 0;
  private metricTimer: number | undefined;
  private pairIssuedAt = 0;

  constructor(scenario: MockScenario) {
    this.scenario = scenario;
    this.hasLink = scenario !== "nolink" && scenario !== "NO_SUBSCRIPTION";
    this.lastUpdatedAt = this.hasLink ? Date.now() - 42 * 60_000 : undefined;

    const daysLeft = scenario === "announce" ? 3 : 12;
    this.meta = {
      title: "proxysvpn.com",
      expiresAt: Math.floor((Date.now() + daysLeft * 86_400_000) / 1000),
      supportUrl: "https://t.me/proxysvpn_bot",
      routingEnabled: true,
      updateIntervalHours: 6,
      announce:
        scenario === "announce"
          ? "Идут работы на серверах в Германии — переключили всех на Нидерланды."
          : undefined,
    };

    // Let the caller subscribe before anything is emitted.
    this.after(0, () => this.begin());
  }

  // ── plumbing ──────────────────────────────────────────────────────────────

  private after(ms: number, fn: () => void): void {
    const id = window.setTimeout(() => {
      this.timers.delete(id);
      fn();
    }, ms);
    this.timers.add(id);
  }

  private emitState(next: StatePayload): void {
    this.state = next;
    for (const h of this.handlers) h.onState?.(next);
  }

  private emitStep(step: VpnStep): void {
    this.step = step;
    for (const h of this.handlers) h.onStep?.(step);
  }

  private emitMetric(): void {
    for (const h of this.handlers) h.onMetric?.(this.metric);
  }

  private emitMeta(): void {
    for (const h of this.handlers) h.onMeta?.(this.meta);
  }

  private emitEvent(code: ErrorCode, handled: boolean, detail?: string): void {
    const payload: EventPayload = { error: { code, detail }, handled };
    for (const h of this.handlers) h.onEvent?.(payload);
  }

  private locationLabel(): string {
    const id = this.selected ?? this.current;
    return MOCK_LOCATIONS.find((l) => l.id === id)?.label ?? "Нидерланды";
  }

  private startMetrics(): void {
    if (this.metricTimer !== undefined) return;
    this.metricTimer = window.setInterval(() => {
      const live = this.state.phase === "on";
      this.metric = {
        rttMs: 34 + Math.round(Math.random() * 18),
        quality: "good",
        rxBytes: this.metric.rxBytes + Math.round(40_000 + Math.random() * 120_000),
        txBytes: this.metric.txBytes + Math.round(8_000 + Math.random() * 30_000),
        // Only a real proof refreshes the age — that is the whole point of
        // showing it out loud.
        lastProofAt: live ? Date.now() : this.metric.lastProofAt,
      };
      this.emitMetric();
    }, 2000);
  }

  private stopMetrics(): void {
    if (this.metricTimer === undefined) return;
    window.clearInterval(this.metricTimer);
    this.metricTimer = undefined;
    this.metric = { rxBytes: 0, txBytes: 0 };
    this.emitMetric();
  }

  /** Puts the scenario on screen once, at start-up. */
  private begin(): void {
    this.emitMeta();
    switch (this.scenario) {
      case "on":
      case "announce":
      case "service-check":
        this.goProtected();
        break;
      case "unconfirmed":
        this.goProtected();
        this.after(50, () =>
          this.emitState({
            phase: "unconfirmed",
            location: this.locationLabel(),
            proto: "vless",
          }),
        );
        break;
      case "connecting":
        void this.connect();
        break;
      case "healing":
        this.goProtected();
        this.after(4000, () => this.enterHealing(11_000));
        break;
      case "healing-long":
        this.goProtected();
        this.after(3000, () => this.enterHealing(Number.POSITIVE_INFINITY));
        break;
      default:
        break;
    }
  }

  private goProtected(): void {
    this.metric = {
      rttMs: 38,
      quality: "good",
      rxBytes: 12_400_000,
      txBytes: 2_100_000,
      lastProofAt: Date.now(),
    };
    this.emitState({ phase: "on", location: this.locationLabel(), proto: "vless" });
    this.emitMetric();
    this.startMetrics();
  }

  /** `recoverAfterMs = Infinity` means the ladder runs out and we fail. */
  private enterHealing(recoverAfterMs: number): void {
    this.healingSince = Date.now();
    this.emitEvent("ENGINE_DIED", true);
    const tick = () => {
      if (this.state.phase !== "healing" && this.healingSince === 0) return;
      const elapsed = Date.now() - this.healingSince;
      if (elapsed >= recoverAfterMs) {
        this.healingSince = 0;
        this.current = "de";
        this.goProtected();
        return;
      }
      if (elapsed > 56_000) {
        this.healingSince = 0;
        this.stopMetrics();
        this.emitState({
          phase: "failed",
          error: { code: "BLOCKED" },
        });
        return;
      }
      this.emitState({
        phase: "healing",
        location: this.locationLabel(),
        proto: "vless",
        healingForMs: elapsed,
      });
      this.after(600, tick);
    };
    tick();
  }

  // ── CoreBridge ────────────────────────────────────────────────────────────

  subscribe(handlers: BridgeHandlers): () => void {
    this.handlers.add(handlers);
    return () => {
      this.handlers.delete(handlers);
    };
  }

  async snapshot(): Promise<VpnSnapshot> {
    return {
      state: this.state,
      metric: this.metric,
      meta: this.meta,
      step: this.state.phase === "starting" ? this.step : undefined,
    };
  }

  async connect(): Promise<void> {
    if (!this.hasLink) throw new CoreError({ code: "NO_SUBSCRIPTION" });
    this.emitState({ phase: "starting" });
    let delay = 0;
    CONNECT_STEPS.forEach(([step, ms]) => {
      this.after(delay, () => {
        if (this.state.phase === "starting") this.emitStep(step);
      });
      delay += ms;
    });
    this.after(delay, () => {
      if (this.state.phase !== "starting") return; // cancelled meanwhile
      if (typeof this.scenario === "string" && isErrorCode(this.scenario)) {
        this.stopMetrics();
        this.emitState({
          phase: "failed",
          error: {
            code: this.scenario,
            // A few codes arrive with text the SERVER wrote; the screen prints
            // it verbatim under our sentence, so the mock supplies one.
            detail:
              this.scenario === "DEVICE_TAKEN"
                ? "Откройте бот → 📡 Мои устройства → это устройство → 🔄 Сбросить привязку"
                : this.scenario === "SUB_NOTICE"
                  ? "Профиль временно приостановлен. Напишите в поддержку."
                  : undefined,
          },
        });
        return;
      }
      if (this.scenario === "unconfirmed") {
        this.emitState({
          phase: "unconfirmed",
          location: this.locationLabel(),
          proto: "vless",
        });
        this.startMetrics();
        return;
      }
      this.goProtected();
    });
  }

  async disconnect(): Promise<void> {
    this.healingSince = 0;
    this.stopMetrics();
    this.emitState({ phase: "off" });
  }

  async locations(): Promise<LocationEntry[]> {
    await this.pause(350);
    return MOCK_LOCATIONS.map((entry) => ({
      ...entry,
      selected: entry.id === this.selected,
    }));
  }

  /** Макет хранит настройки в памяти: страница перезагрузилась - как новые. */
  private prefs: TunnelPrefs = {
    fragment: false,
    ipKind: "ipv4",
    dns: "internal",
    customDns: "",
    transport: "auto",
    directDomains: [],
    proxyDomains: [],
  };

  private notify: NotifyPrefs = { enabled: true };

  async tunnelPrefs(): Promise<TunnelPrefs> {
    await this.pause(120);
    return { ...this.prefs };
  }

  async setTunnelPrefs(prefs: TunnelPrefs): Promise<boolean> {
    await this.pause(260);
    this.prefs = { ...prefs };
    // В макете «переподключаемся» - это правда, только когда туннель поднят.
    return this.state.phase === "on" || this.state.phase === "unconfirmed";
  }

  /** Та же проверка, что и в Rust (tunnel_prefs.rs), для правдоподобного макета. */
  private static validateDomain(raw: string): string | null {
    const trimmed = raw.trim();
    if (!trimmed) return null;
    let rest = trimmed;
    let prefix = "domain:";
    if (trimmed.startsWith("full:")) {
      prefix = "full:";
      rest = trimmed.slice(5);
    } else if (trimmed.startsWith("domain:")) {
      rest = trimmed.slice(7);
    }
    if (!rest || rest.length > 253) return null;
    // Ни схемы, ни пути, ни порта, ни звёздочки, ни пробела - только имя.
    // ":" здесь же ловит и IPv6 (у него в форме без схемы двоеточие есть
    // всегда), а IPv4 - отдельной проверкой чуть ниже, тем же порядком, что
    // и validate_custom_domain в tunnel_prefs.rs.
    if (/[:/\\*\s]/.test(rest)) return null;
    if (/^\d{1,3}(\.\d{1,3}){3}$/.test(rest)) return null;
    const labels = rest.split(".");
    if (labels.length < 2) return null;
    for (const label of labels) {
      if (!label || label.length > 63 || label.startsWith("-") || label.endsWith("-")) return null;
      if (!/^[a-zA-Z0-9-]+$/.test(label)) return null;
    }
    return `${prefix}${rest.toLowerCase()}`;
  }

  private static parseList(raw: string): { accepted: string[]; ignored: string[] } {
    const accepted: string[] = [];
    const ignored: string[] = [];
    const seen = new Set<string>();
    for (const rawLine of raw.split("\n")) {
      const line = rawLine.trim();
      if (!line) continue;
      if (accepted.length >= 200) {
        ignored.push(line);
        continue;
      }
      const valid = MockBridge.validateDomain(line);
      if (valid && !seen.has(valid)) {
        seen.add(valid);
        accepted.push(valid);
      } else if (!valid) {
        ignored.push(line);
      }
    }
    return { accepted, ignored };
  }

  async setCustomRules(direct: string, proxy: string): Promise<CustomRulesOutcome> {
    await this.pause(260);
    const d = MockBridge.parseList(direct);
    const p = MockBridge.parseList(proxy);
    const directSet = new Set(d.accepted);
    const clashing = p.accepted.filter((x) => directSet.has(x));
    const clashSet = new Set(clashing);
    this.prefs = {
      ...this.prefs,
      directDomains: d.accepted.filter((x) => !clashSet.has(x)),
      proxyDomains: p.accepted.filter((x) => !clashSet.has(x)),
    };
    return {
      directIgnored: [...d.ignored, ...clashing],
      proxyIgnored: [...p.ignored, ...clashing],
      reconnected: this.state.phase === "on" || this.state.phase === "unconfirmed",
    };
  }

  async notifyPrefs(): Promise<NotifyPrefs> {
    await this.pause(80);
    return { ...this.notify };
  }

  async setNotifyPrefs(enabled: boolean): Promise<void> {
    await this.pause(80);
    this.notify = { enabled };
  }

  async setUiLang(_lang: string): Promise<void> {
    // Ядру в макете не о чем говорить на этом языке - системных уведомлений
    // макет не показывает вовсе.
  }

  async measureLocations(): Promise<LocationEntry[]> {
    // Замер дольше выдачи списка - так и в жизни.
    await this.pause(900);
    return MOCK_LOCATIONS.map((entry, i) => ({
      ...entry,
      selected: entry.id === this.selected,
      // Каждая четвёртая без числа: так на экране видно, как выглядит
      // локация на Hysteria2 и не ломается ли вёрстка без миллисекунд.
      rttMs: i % 4 === 3 ? undefined : 28 + i * 17,
    }));
  }

  async selectLocation(id: string | null): Promise<void> {
    await this.pause(400);
    this.selected = id;
    if (id) this.current = id;
    if (this.state.phase === "on" || this.state.phase === "unconfirmed") {
      // Soft switch: routes stay up, so the phase never drops to `off`.
      this.emitState({ ...this.state, location: this.locationLabel() });
    }
  }

  async subState(): Promise<SubState> {
    return {
      hasLink: this.hasLink,
      lastUpdatedAt: this.lastUpdatedAt,
      usedFallback: this.scenario === "announce",
    };
  }

  async setSubscription(link: string): Promise<void> {
    if (!/^https?:\/\/\S+$/i.test(link.trim())) {
      throw new CoreError({ code: "SUB_MALFORMED" });
    }
    await this.pause(500);
    this.hasLink = true;
    this.lastUpdatedAt = Date.now();
    this.emitMeta();
  }

  async clearSubscription(): Promise<void> {
    this.hasLink = false;
    this.lastUpdatedAt = undefined;
    await this.disconnect();
  }

  async refreshSub(): Promise<SubMeta> {
    await this.pause(700);
    if (this.scenario === "SUB_UNREACHABLE") throw new CoreError({ code: "SUB_UNREACHABLE" });
    this.lastUpdatedAt = Date.now();
    this.emitMeta();
    return this.meta;
  }

  async pairStart(): Promise<PairSession> {
    await this.pause(600);
    if (this.scenario === "SUB_UNREACHABLE") throw new CoreError({ code: "SUB_UNREACHABLE" });
    this.pairIssuedAt = Date.now();
    const ttl = this.scenario === "pair-expired" ? 12_000 : 300_000;
    return {
      token: "7f3c1a9e42b85d06cf17ae2b904d6351",
      expiresAt: this.pairIssuedAt + ttl,
    };
  }

  async pairPoll(_token: string): Promise<PairStatus> {
    const age = Date.now() - this.pairIssuedAt;
    if (this.scenario === "pair-expired") return age > 12_000 ? "expired" : "waiting";
    if (age > 9000) {
      this.hasLink = true;
      this.lastUpdatedAt = Date.now();
      return "linked";
    }
    return "waiting";
  }

  async routing(): Promise<RoutingState> {
    return this.routingState;
  }

  async setRouting(inRussia: boolean): Promise<void> {
    await this.pause(500);
    this.routingState = { ...this.routingState, inRussia };
  }

  async trafficSplit(): Promise<TrafficSplit | null> {
    if (this.state.phase === "off") return null;
    return { viaVpn: 2140, direct: this.routingState.inRussia ? 386 : 0 };
  }

  async runCheck(): Promise<CheckReport> {
    await this.pause(1400);
    const failing = this.failingRow();
    const device: CheckRow[] = DEVICE_ROWS.map((id) => ({
      id,
      state: id === failing ? "fail" : "pass",
      viaFallback: id === "sub" && this.scenario === "announce",
    }));
    const service: CheckService =
      this.scenario === "service-check"
        ? {
            available: true,
            rows: [
              { id: "balance", state: "pass" },
              { id: "profile", state: "fail" },
              { id: "freshness", state: "pass" },
              { id: "nodes", state: "pass" },
            ],
            canFix: true,
          }
        : { available: false, reason: "cabinetOnly" };
    const verdict: CheckRowId | null =
      this.scenario === "service-check" ? "profile" : (failing ?? null);
    return { device, service, verdict };
  }

  async runFix(): Promise<CheckReport> {
    await this.pause(1600);
    return {
      device: DEVICE_ROWS.map((id) => ({ id, state: "pass" })),
      service: {
        available: true,
        rows: [
          { id: "balance", state: "pass" },
          { id: "profile", state: "pass" },
          { id: "freshness", state: "pass" },
          { id: "nodes", state: "pass" },
        ],
        canFix: false,
      },
      verdict: null,
    };
  }

  /** Which local row a scenario should trip, so the verdict is not invented. */
  private failingRow(): CheckRowId | undefined {
    switch (this.scenario) {
      case "NETWORK_OFFLINE":
        return "internet";
      case "SUB_UNREACHABLE":
      case "SUB_INVALID":
        return "sub";
      case "NO_ROUTE":
      case "ENGINE_START_FAILED":
      case "TUN_FAILED":
        return "handshake";
      case "BLOCKED":
      case "healing-long":
        return "traffic";
      default:
        return undefined;
    }
  }

  async buildReport(): Promise<SupportReport> {
    await this.pause(500);
    const text = [
      "ProxysVPN 0.2.0 · macOS 15.6 · arm64",
      `Состояние: ${this.state.phase}`,
      `Локация: ${this.locationLabel()} · протокол: vless`,
      "Сеть: Wi-Fi",
      "Подписка: обновлена 14:20, адрес №1 из лестницы",
      "Проба выхода: ответила, 38 мс",
      "Неуспешных рукопожатий за час: 2 из 41",
      "Ядро: запущено, перезапусков 0",
      "",
      "Последние события:",
      "14:02  Соединение восстановлено, сервер сменился на Германию",
      "13:58  Данные перестали идти",
      "11:20  Сеть сменилась на мобильную",
      "09:10  Защита включена, Нидерланды",
    ].join("\n");
    return { text, bytes: new TextEncoder().encode(text).length };
  }

  async timeline(limit: number): Promise<TimelineEntry[]> {
    await this.pause(250);
    const now = Date.now();
    const hour = 3_600_000;
    const entries: TimelineEntry[] = [
      { atMs: now - hour, code: "healed", location: "Германия" },
      { atMs: now - hour - 4 * 60_000, code: "trafficStopped" },
      { atMs: now - 4 * hour, code: "networkChanged", network: "mobile" },
      // The same code WITHOUT `network`, because that is the only shape the
      // real core sends today: it reports that the machine changed how it
      // reaches the internet, not what it changed to. The mock used to show
      // only the richer row, so the screen the owner reviews looked better
      // than the screen that ships — which is how a literal "{network}"
      // survived on the real build.
      { atMs: now - 5 * hour, code: "networkChanged" },
      { atMs: now - 6 * hour, code: "connected", location: "Нидерланды" },
      { atMs: now - 20 * hour, code: "subRefreshedFallback" },
      { atMs: now - 22 * hour, code: "locationUnreadable" },
      { atMs: now - 26 * hour, code: "disconnected" },
      { atMs: now - 50 * hour, code: "failed" },
    ];
    return entries.slice(0, limit);
  }

  async logs(offset: number, limit: number): Promise<LogLine[]> {
    await this.pause(200);
    const total = 640;
    const lines: LogLine[] = [];
    for (let i = offset; i < Math.min(offset + limit, total); i += 1) {
      lines.push({
        tsMs: Date.now() - i * 1500,
        level: i % 37 === 0 ? "error" : i % 11 === 0 ? "warn" : "info",
        source: i % 3 === 0 ? "xray" : "tun",
        message:
          i % 37 === 0
            ? "probe failed, retrying (1/2)"
            : `connection accepted, routed via tunnel (#${total - i})`,
      });
    }
    return lines;
  }

  async onboarding(): Promise<OnboardingState> {
    // A first run, as the core answers it: the data notice until "Продолжить",
    // then whatever barriers the scenario plays.
    const firstRun = this.scenario === "onboarding" || this.scenario === "nolink";
    const steps: OnboardingStep[] = [];
    if (firstRun && !this.dataNoticeSeen) steps.push("dataNotice");
    if (this.scenario === "onboarding") steps.push("moveToApplications", "password");
    return { steps };
  }

  async onboardingRun(step: OnboardingStep): Promise<void> {
    if (step === "dataNotice") {
      await this.pause(150);
      this.dataNoticeSeen = true;
      return;
    }
    await this.pause(900);
  }

  async appInfo(): Promise<AppInfo> {
    return {
      version: "0.2.0",
      platform: "macos",
      cabinetUrl: "https://proxysvpn.com",
      botUrl: "https://t.me/proxysvpn_bot",
      deviceName: "MacBook",
      deviceLinkedAt: Date.now() - 18 * 86_400_000,
    };
  }

  async openExternal(url: string): Promise<void> {
    window.open(url, "_blank", "noopener,noreferrer");
  }

  async readClipboard(): Promise<string> {
    try {
      return await navigator.clipboard.readText();
    } catch {
      throw new CoreError({ code: "UNKNOWN", detail: "clipboard unavailable" });
    }
  }

  async writeClipboard(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      throw new CoreError({ code: "UNKNOWN", detail: "clipboard unavailable" });
    }
  }

  private pause(ms: number): Promise<void> {
    return new Promise((resolve) => this.after(ms, resolve));
  }
}

/**
 * The scenario this page is playing; "off" and irrelevant inside the app.
 *
 * App Store builds never pick the mock, even in a plain browser: `IS_APPSTORE`
 * is a build-time constant, so it goes first and the bundler drops the mock
 * and its demo data (fake notice, fake locations, the bot address) from the
 * reviewed bundle altogether (guideline 2.2).
 */
export const MOCK_SCENARIO: MockScenario = IS_APPSTORE || IS_TAURI ? "off" : readScenario();

export const bridge: CoreBridge =
  IS_APPSTORE || IS_TAURI ? new TauriBridge() : new MockBridge(MOCK_SCENARIO);
