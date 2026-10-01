// src/storeCopy.ts
//
// What an App Store build may say about money — as pure functions with no
// build-time input, so node --test runs them as they are. The screens pass
// `IS_APPSTORE` (src/dist.ts) in.
//
// The app ships without in-app purchases (owner's decision, guideline
// 3.1.3(f)). That leaves no room for a button, a link or a sentence that
// sends people to pay somewhere else: no "top up", no "renew", no price, no
// balance, no "open the cabinet" or "write to the bot" (the bot sells
// top-ups). The direct DMG keeps every word it has today — every function
// here returns the direct answer unchanged when `isAppstore` is false.

import type { MsgKey } from "./i18n";
import type { ErrorAction, ErrorCode } from "./types";

/**
 * Words that make a server-written line a sales line.
 *
 * Matched at the START of a word only (Unicode-aware, since `\b` does not
 * know Cyrillic): "оценка" and "лицензия" contain "цен" and are not about
 * money. The start is "beginning of text or a non-letter before it", not a
 * lookbehind: lookbehind needs Safari 16.4, the app runs on iOS 15 and
 * macOS 11 WebViews, and there an unsupported regex literal is a syntax
 * error that takes the whole bundle down. A false alarm hides an outage notice in an App Store build; a miss
 * shows a reviewer a payment prompt — so the list leans towards hiding.
 * Checked against what the service actually sends (frontend
 * `api/sub/[token]/route.ts`, `lib/sub-reserve-notice.ts`): the empty-balance
 * title "⚠️ Нет средств", the balance and PRO-renewal warnings, and every
 * notice that sends people to the bot or the cabinet.
 */
const PAYMENT_WORDS =
  /(?:^|[^\p{L}\p{N}])(?:пополн|оплат|оплач|продл|цен[аеуыой]|стоимост|тариф|баланс|купи|купл|покуп|скидк|промокод|руб(?:л|\.|(?![\p{L}]))|деньг|средств|кабинет|бот(?:а|е|у|ом)?(?![\p{L}])|top[\s-]?up|renew|pay|paid|pric|buy|purchas|billing|balance|fund|discount|promo|tariff|checkout|cabinet|dashboard|bot(?![\p{L}])|usdt?(?![\p{L}])|rub(?![\p{L}]))|[₽$€]|t\.me\/|@\w*bot(?![\p{L}])/iu;

/** True when `text` reads as a purchase prompt or points at a place to pay. */
export function mentionsPayment(text: string): boolean {
  return PAYMENT_WORDS.test(text);
}

/**
 * A line the SERVICE wrote (announce, info text, error detail, profile
 * title), as a build may show it: unchanged in direct builds; in App Store
 * builds dropped entirely when it mentions money. Dropped rather than cut:
 * half a sentence about a top-up is still a sentence about a top-up.
 */
export function shownServerText(
  text: string | null | undefined,
  isAppstore: boolean,
): string | undefined {
  // Same truthiness test the screens used before, so a direct build renders
  // exactly what it rendered then.
  if (!text) return undefined;
  if (isAppstore && mentionsPayment(text)) return undefined;
  return text;
}

/**
 * The one action a failure offers, as an App Store build may offer it.
 *
 * Both actions that leave the app for the web cabinet — to pay ("topUp") or
 * to manage devices ("openCabinet") — become "retry": the person sorts it out
 * wherever they manage their account, then checks again here. `retry` is a
 * fresh connect, which re-reads the subscription, so "check again" is exactly
 * what it does.
 */
export function storeSafeAction(action: ErrorAction): ErrorAction {
  return action === "topUp" || action === "openCabinet" ? "retry" : action;
}

/** `storeSafeAction` over a whole code -> action map. The map stays total. */
export function storeSafeActions(
  map: Readonly<Record<ErrorCode, ErrorAction>>,
): Record<ErrorCode, ErrorAction> {
  const out: Record<ErrorCode, ErrorAction> = { ...map };
  for (const code of Object.keys(out) as ErrorCode[]) out[code] = storeSafeAction(out[code]);
  return out;
}

/**
 * Codes whose direct wording names money, the cabinet or the bot, or reads as
 * entering a key. Each has `err.<CODE>.appstore.title` and `.body` in i18n.ts.
 */
type StoreCopyCode =
  | "NO_SUBSCRIPTION"
  | "SUB_MALFORMED"
  | "SUB_EMPTY"
  | "BALANCE_EMPTY"
  | "EXPIRED"
  | "DEVICE_TAKEN"
  | "NO_DEVICES"
  | "SUB_NOTICE";

const STORE_COPY_CODES: ReadonlySet<ErrorCode> = new Set<StoreCopyCode>([
  "NO_SUBSCRIPTION",
  "SUB_MALFORMED",
  "SUB_EMPTY",
  "BALANCE_EMPTY",
  "EXPIRED",
  "DEVICE_TAKEN",
  "NO_DEVICES",
  "SUB_NOTICE",
]);

/** True when an App Store build has its own wording for this code. */
export function hasStoreCopy(code: ErrorCode): code is StoreCopyCode {
  return STORE_COPY_CODES.has(code);
}

/** i18n key of the failure's title for this build. */
export function errorTitleKey(code: ErrorCode, isAppstore: boolean): MsgKey {
  return isAppstore && hasStoreCopy(code) ? `err.${code}.appstore.title` : `err.${code}.title`;
}

/** i18n key of the failure's paragraph for this build. */
export function errorBodyKey(code: ErrorCode, isAppstore: boolean): MsgKey {
  return isAppstore && hasStoreCopy(code) ? `err.${code}.appstore.body` : `err.${code}.body`;
}
