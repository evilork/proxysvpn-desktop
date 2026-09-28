// src/types.ts
//
// The window's half of the contract in src-tauri/src/events.rs and errors.rs.
//
// Keep the two files in step. A renamed variant here is a silent screen bug,
// not a compile error, because the payloads arrive as JSON — so every union
// below is spelled out rather than inferred, and `ERROR_ACTION` is exhaustive
// by construction (a missing code fails `tsc`).

import { IS_APPSTORE } from "./dist";
import { storeSafeActions } from "./storeCopy";

/** Phase of the tunnel. Five, not three — see events.rs for why. */
export type VpnPhase =
  | "off"
  | "starting"
  | "on"
  | "unconfirmed"
  | "healing"
  | "failed";

/** Where we are inside `starting`. Shown as a changing line, honestly. */
export type VpnStep =
  | "fetchingSub"
  | "pickingServer"
  | "startingEngine"
  | "raisingTun"
  | "probing";

/**
 * Link quality in words. Numbers live in the details sheet, not on the shield.
 *
 * `unknown` means "not measured yet, it may still arrive"; `notApplicable`
 * means "this link has no such number and never will" — Hysteria2 is pure UDP,
 * so a TCP handshake to its port never completes. The old client could not
 * tell the two apart and left the Netherlands node on "измерение…" forever.
 */
export type LinkQuality =
  | "good"
  | "ok"
  | "poor"
  | "unknown"
  | "notApplicable";

export type ErrorCode =
  | "NO_SUBSCRIPTION"
  | "SUB_MALFORMED"
  | "SUB_UNREACHABLE"
  | "SUB_INVALID"
  | "SUB_EMPTY"
  // Readable settings, but every location needs a transport this build's
  // engine lacks (iOS: sing-box has no XHTTP). The app is behind, not the link.
  | "ENGINE_UNSUPPORTED"
  | "BALANCE_EMPTY"
  | "EXPIRED"
  | "DEVICE_TAKEN"
  | "NO_DEVICES"
  | "SUB_NOTICE"
  | "PERMISSION_DENIED"
  | "ENGINE_START_FAILED"
  | "ENGINE_DIED"
  | "PORT_BUSY"
  | "TUN_FAILED"
  | "NETWORK_OFFLINE"
  | "NO_ROUTE"
  | "BLOCKED"
  | "PROBE_UNCONFIRMED"
  // A metric state, not a screen: there is no latency figure for this link
  // and there never will be. Its action is to offer nothing.
  | "PING_NOT_APPLICABLE"
  | "UNKNOWN";

export interface AppError {
  code: ErrorCode;
  /** Text the SERVER wrote. Rendered under the phrase, never instead of it. */
  detail?: string;
}

export interface StatePayload {
  phase: VpnPhase;
  location?: string;
  proto?: string;
  error?: AppError;
  healingForMs?: number;
}

export interface MetricPayload {
  rttMs?: number;
  quality?: LinkQuality;
  rxBytes: number;
  txBytes: number;
  /** Unix ms of the last proof a byte crossed. The UI ages this out loud. */
  lastProofAt?: number;
}

export interface SubMeta {
  title?: string;
  announce?: string;
  announceUrl?: string;
  expiresAt?: number;
  infoText?: string;
  supportUrl?: string;
  routingEnabled?: boolean;
  updateIntervalHours?: number;
}

export interface VpnSnapshot {
  state: StatePayload;
  metric: MetricPayload;
  meta: SubMeta;
  step?: VpnStep;
}

export interface EventPayload {
  error: AppError;
  /** The core is already on it; the user is not being asked to do anything. */
  handled: boolean;
}

/**
 * Answer of an on-demand latency measurement (ping.rs).
 *
 * `measured` is the only outcome carrying a number. The other three exist so
 * the window can say something true instead of spinning:
 *   notApplicable — pure-UDP link, TCP cannot measure it
 *   unreachable   — the endpoint answered nothing, or refused
 *   noTarget      — nothing is connected, so there is nothing to measure
 */
export type PingOutcome =
  | "measured"
  | "notApplicable"
  | "unreachable"
  | "noTarget";

export interface PingPayload {
  outcome: PingOutcome;
  /** Present only when `outcome === "measured"`. */
  rttMs?: number;
  quality: LinkQuality;
  /** True when this answer came from the short-lived cache, not a new socket. */
  cached: boolean;
}

export interface ServerEntry {
  index: number;
  remark: string;
  /** Deliberately NOT rendered anywhere: node addresses are not for showing. */
  host: string;
  port: number;
  proto: string;
}

/** Event names. Must match the `EV_*` constants in events.rs. */
export const EV = {
  state: "vpn:state",
  step: "vpn:step",
  metric: "vpn:metric",
  event: "vpn:event",
  meta: "vpn:meta",
} as const;

/**
 * The ONE thing a failure offers the user.
 *
 * A screen with five suggestions reads as "everything is broken" and sends the
 * person to support just as reliably as silence does. So every code maps to
 * exactly one action, and the map is total: add a code to `ErrorCode` without
 * adding it here and the build fails.
 */
export type ErrorAction =
  | "addLink" // open the paste-link screen
  | "retry" // try again, right now
  | "openCabinet" // the answer lives in the web cabinet
  | "topUp" // money
  | "diagnose" // run the checks and show what they found
  | "waitAndSee" // we are already handling it; offer nothing
  | "contactSupport";

const DIRECT_ERROR_ACTION: Record<ErrorCode, ErrorAction> = {
  NO_SUBSCRIPTION: "addLink",
  SUB_MALFORMED: "addLink",
  SUB_UNREACHABLE: "retry",
  SUB_INVALID: "contactSupport",
  SUB_EMPTY: "openCabinet",
  // Nothing on the person's side fixes it; support knows when the update lands.
  ENGINE_UNSUPPORTED: "contactSupport",

  BALANCE_EMPTY: "topUp",
  EXPIRED: "topUp",
  // Not "reset the binding": resetting from the app would make breaking
  // "one link — one device" the easiest button on the screen. The cabinet
  // asks for a session first, and that is the point.
  DEVICE_TAKEN: "openCabinet",
  NO_DEVICES: "openCabinet",
  SUB_NOTICE: "openCabinet",

  PERMISSION_DENIED: "retry",
  ENGINE_START_FAILED: "retry",
  ENGINE_DIED: "waitAndSee",
  PORT_BUSY: "retry",
  TUN_FAILED: "diagnose",

  NETWORK_OFFLINE: "waitAndSee",
  NO_ROUTE: "diagnose",
  BLOCKED: "diagnose",
  PROBE_UNCONFIRMED: "diagnose",
  // Nothing is broken and nothing can be pressed: the protocol simply has no
  // latency figure. Rendering this as a failure screen would be a lie.
  PING_NOT_APPLICABLE: "waitAndSee",
  UNKNOWN: "contactSupport",
};

/**
 * The map this build uses. App Store builds carry no purchase link of any
 * kind (src/distPolicy.ts), so "topUp" and "openCabinet" — both of which
 * leave for the web cabinet — become "retry" there: BALANCE_EMPTY and EXPIRED
 * offer "Проверить снова" instead of "Пополнить", and DEVICE_TAKEN no longer
 * offers a cabinet the app may not open. The direct DMG keeps the map above.
 */
export const ERROR_ACTION: Record<ErrorCode, ErrorAction> = IS_APPSTORE
  ? storeSafeActions(DIRECT_ERROR_ACTION)
  : DIRECT_ERROR_ACTION;

/**
 * Turn the string a Tauri command rejected with back into an `AppError`.
 *
 * Commands in this crate answer `Result<T, String>`; the core puts JSON in
 * that string (`AppError::to_payload`). Anything that is not our JSON — a
 * panic message, a plugin error — becomes `UNKNOWN` WITH its text kept, so the
 * support report still carries the truth even when we failed to name it.
 */
export function parseAppError(raw: unknown): AppError {
  if (raw && typeof raw === "object" && "code" in raw) {
    return raw as AppError;
  }
  const text = typeof raw === "string" ? raw : String(raw ?? "");
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed && typeof parsed === "object" && "code" in parsed) {
      return parsed as AppError;
    }
  } catch {
    // Not JSON. Fall through — the text is still worth keeping.
  }
  return { code: "UNKNOWN", detail: text || undefined };
}

/** A phase the user should read as "you are protected". */
export function isProtected(phase: VpnPhase): boolean {
  return phase === "on";
}

/** A phase where the core is working and the user should not be asked to act. */
export function isBusy(phase: VpnPhase): boolean {
  return phase === "starting" || phase === "healing";
}
