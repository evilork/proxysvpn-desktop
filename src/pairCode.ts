// src/pairCode.ts
//
// Pair code v1.1, the window's half: what the code field does to what a person
// types, and what it says when the core refuses. Pure functions with no
// build-time input, so node --test runs them as they are; the screen passes
// `IS_APPSTORE` (src/dist.ts) in.
//
// The rules match the core (src-tauri/src/pair_code.rs) and the service:
// eight characters of 23456789ABCDEFGHJKMNPQRSTUVWXYZ (no 0 1 I L O), shown
// as XXXX-XXXX; input is uppercased and loses whitespace and dashes first.
//
// The request id of v1.1 never reaches the window: the core keeps one per
// code and resends it on every press, so all the window has to do after "could
// not reach the service" is leave the button pressable (`pairCodeRestMs`).

import type { MsgKey } from "./i18n";
import type { AppError, ErrorCode } from "./types";

export const PAIR_CODE_ALPHABET = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";
export const PAIR_CODE_LENGTH = 8;

/** Where the dash goes in the shown form. */
const HALF = PAIR_CODE_LENGTH / 2;

/** Longer input is a pasted paragraph, not a code with spaces in it. */
const MAX_INPUT_CHARS = 64;

/** The wait the wording promises ("Подождите минуту"), and the most we hold the button for. */
const RATE_LIMIT_PAUSE_MAX_S = 60;

/**
 * The code as the service expects it, or `null` when it cannot be one.
 *
 * Uppercase, drop whitespace (JavaScript's `\s`, plus U+0085 so the set is
 * the core's `char::is_whitespace` plus U+FEFF on both sides) and dashes,
 * then exactly eight characters of the alphabet.
 */
export function normalisePairCode(raw: string): string | null {
  if ([...raw].length > MAX_INPUT_CHARS) return null;
  const code = raw.toUpperCase().replace(/[\s\u0085-]/g, "");
  if (code.length !== PAIR_CODE_LENGTH) return null;
  for (const ch of code) {
    if (!PAIR_CODE_ALPHABET.includes(ch)) return null;
  }
  return code;
}

/** "K7QM2XPA" → "K7QM-2XPA". Shorter input gets the dash only past the half. */
export function displayPairCode(chars: string): string {
  return chars.length > HALF ? `${chars.slice(0, HALF)}-${chars.slice(HALF)}` : chars;
}

/** The alphabet characters of `raw`, uppercased, at most eight. */
function codeChars(raw: string): string {
  let out = "";
  for (const ch of raw.toUpperCase()) {
    if (out.length === PAIR_CODE_LENGTH) break;
    if (PAIR_CODE_ALPHABET.includes(ch)) out += ch;
  }
  return out;
}

/** What the field shows, and where the caret goes. */
export interface PairCodeField {
  value: string;
  caret: number;
}

/**
 * One edit of the code field.
 *
 * `previous` is what the field showed, `raw` what it holds after the edit,
 * `caret` where the browser left the caret in `raw`. The result keeps only
 * alphabet characters, uppercased, eight at most, with the dash after the
 * fourth once there is a fifth - so typing never leaves a dash the next
 * backspace would just put back. A dash is never deleted on its own: a
 * backspace that took only the dash takes the character before it instead,
 * as a person pressing it meant.
 */
export function editPairCode(
  previous: string,
  raw: string,
  caret: number,
  backspace: boolean,
): PairCodeField {
  let text = raw;
  let at = Math.max(0, Math.min(caret, raw.length));
  const tookOnlyTheDash =
    backspace &&
    text.length === previous.length - 1 &&
    previous.charAt(at) === "-" &&
    previous.slice(0, at) + previous.slice(at + 1) === text;
  if (tookOnlyTheDash && at > 0) {
    text = text.slice(0, at - 1) + text.slice(at);
    at -= 1;
  }
  const chars = codeChars(text);
  const before = Math.min(codeChars(text.slice(0, at)).length, chars.length);
  return {
    value: displayPairCode(chars),
    caret: before > HALF ? before + 1 : before,
  };
}

/**
 * A paste that is a whole code replaces the field, whatever was in it:
 * "k7qm 2xpa" from a chat lands as "K7QM-2XPA". `null` lets the browser
 * paste as usual and `editPairCode` tidy the result.
 */
export function pastedPairCode(text: string): string | null {
  const code = normalisePairCode(text);
  return code === null ? null : displayPairCode(code);
}

/** The field holds a whole code: the button may be pressed. */
export function isCompletePairCode(value: string): boolean {
  return normalisePairCode(value) !== null;
}

/** The sentence under the field for a refusal, by kind. */
export type PairCodeProblem = "notFound" | "rateLimited" | "network" | "failed";

export function pairCodeProblem(code: ErrorCode): PairCodeProblem {
  switch (code) {
    // Wrong, expired, used - and a link the core refused to keep: the code is
    // spent either way, and the remedy is the same new one.
    case "PAIR_CODE_NOT_FOUND":
    case "PAIR_CODE_MALFORMED":
    case "SUB_INVALID":
    case "SUB_MALFORMED":
      return "notFound";
    case "PAIR_RATE_LIMITED":
      return "rateLimited";
    // Every site name failed: the person's network, almost always.
    case "SUB_UNREACHABLE":
    case "NETWORK_OFFLINE":
      return "network";
    default:
      return "failed";
  }
}

/**
 * i18n key of that sentence for this build. App Store builds never send the
 * person to the cabinet or the bot (src/storeCopy.ts), so "take a new code"
 * points at their ProxysVPN account there instead.
 */
export function pairCodeProblemKey(code: ErrorCode, isAppstore: boolean): MsgKey {
  switch (pairCodeProblem(code)) {
    case "notFound":
      return isAppstore ? "pair.appstore.code.notFound" : "pair.code.notFound";
    case "rateLimited":
      return "pair.code.rateLimited";
    case "network":
      return "pair.code.network";
    case "failed":
      return "pair.code.failed";
  }
}

/**
 * How long to hold the button after a 429: the service's Retry-After
 * (`detail`, seconds), at most the minute the sentence asks for; a minute
 * when the core could not say.
 */
export function rateLimitPauseMs(error: AppError): number {
  const raw = error.detail?.trim() ?? "";
  const seconds = /^\d+$/.test(raw) ? Number(raw) : RATE_LIMIT_PAUSE_MAX_S;
  return Math.min(Math.max(seconds, 1), RATE_LIMIT_PAUSE_MAX_S) * 1000;
}

/**
 * How long the button rests after a refusal: the 429's wait, and nothing for
 * any other. A network failure in particular is pressed again at once - the
 * core resends the same request id, and the service repeats a link whose
 * first answer was lost on the way.
 */
export function pairCodeRestMs(error: AppError): number {
  return error.code === "PAIR_RATE_LIMITED" ? rateLimitPauseMs(error) : 0;
}
