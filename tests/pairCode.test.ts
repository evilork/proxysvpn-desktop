// tests/pairCode.test.ts
//
// node --test tests/pairCode.test.ts (Node >= 23.6 strips the types).

import assert from "node:assert/strict";
import { test } from "node:test";

import { makeT } from "../src/i18n.ts";
import {
  PAIR_CODE_ALPHABET,
  displayPairCode,
  editPairCode,
  isCompletePairCode,
  normalisePairCode,
  pairCodeProblem,
  pairCodeProblemKey,
  pairCodeRestMs,
  pastedPairCode,
  rateLimitPauseMs,
} from "../src/pairCode.ts";
import { mentionsPayment } from "../src/storeCopy.ts";

/** Type `text` one character at a time at the end of the field. */
function typeInto(text: string): string {
  let value = "";
  for (const ch of text) {
    const raw = value + ch;
    value = editPairCode(value, raw, raw.length, false).value;
  }
  return value;
}

/** Press backspace with the caret at `caret` of `value`. */
function backspaceAt(value: string, caret: number): { value: string; caret: number } {
  const raw = value.slice(0, caret - 1) + value.slice(caret);
  return editPairCode(value, raw, caret - 1, true);
}

// ── normalisation: the same rules as the core and the service ─────────────

test("the shown form, the bare form and a lowercase paste are one code", () => {
  for (const raw of ["K7QM-2XPA", "K7QM2XPA", "k7qm 2xpa", "  k7qm-2xpa\n", "K7QM - 2XPA", "k-7-q-m-2-x-p-a"]) {
    assert.equal(normalisePairCode(raw), "K7QM2XPA", JSON.stringify(raw));
  }
});

test("unicode spaces from a copied page are dropped too", () => {
  assert.equal(normalisePairCode("K7QM 2XPA"), "K7QM2XPA");
  assert.equal(normalisePairCode("﻿K7QM 2XPA"), "K7QM2XPA");
  assert.equal(normalisePairCode("K7QM\u00852XPA"), "K7QM2XPA");
});

test("the wrong length is not a code", () => {
  for (const raw of ["", "   ", "-", "K7QM", "K7QM2XP", "K7QM2XPAB", "K7QM-2XPA-K7QM"]) {
    assert.equal(normalisePairCode(raw), null, JSON.stringify(raw));
  }
});

test("the look-alikes left out of the alphabet are refused", () => {
  for (const raw of ["K7QM2XP0", "K7QM2XP1", "K7QM2XPI", "K7QM2XPL", "K7QM2XPO", "k7qm2xpo"]) {
    assert.equal(normalisePairCode(raw), null, raw);
  }
  for (const ch of "01ILO") assert.equal(PAIR_CODE_ALPHABET.includes(ch), false, ch);
  assert.equal(PAIR_CODE_ALPHABET.length, 31);
});

test("punctuation and other scripts are not stripped into a code", () => {
  // Cyrillic К, М, Х, Р and А look exactly like the Latin capitals.
  for (const raw of ["K7QM_2XPA", "K7QM.2XPA", "К7QМ2ХРА", "K7QM2XP😀", "K7QM–2XPA"]) {
    assert.equal(normalisePairCode(raw), null, raw);
  }
});

test("a pasted paragraph is refused before it is read", () => {
  assert.equal(normalisePairCode(`K7QM2XPA${" ".repeat(64)}`), null);
});

test("the dash goes in only past the fourth character", () => {
  assert.equal(displayPairCode(""), "");
  assert.equal(displayPairCode("K7Q"), "K7Q");
  assert.equal(displayPairCode("K7QM"), "K7QM");
  assert.equal(displayPairCode("K7QM2"), "K7QM-2");
  assert.equal(displayPairCode("K7QM2XPA"), "K7QM-2XPA");
});

// ── the field as it is typed ───────────────────────────────────────────────

test("typing uppercases and puts the dash in after four characters", () => {
  assert.equal(typeInto("k"), "K");
  assert.equal(typeInto("k7qm"), "K7QM");
  assert.equal(typeInto("k7qm2"), "K7QM-2");
  assert.equal(typeInto("k7qm2xpa"), "K7QM-2XPA");
});

test("typing the dash yourself does not double it", () => {
  assert.equal(typeInto("k7qm-2xpa"), "K7QM-2XPA");
  assert.equal(typeInto("k7qm--2xpa"), "K7QM-2XPA");
});

test("characters that cannot be in a code are not taken", () => {
  assert.equal(typeInto("k7q0m"), "K7QM");
  assert.equal(typeInto("k7qm 2xpa"), "K7QM-2XPA");
  assert.equal(typeInto("ilo01"), "");
});

test("a ninth character is not taken", () => {
  assert.equal(typeInto("k7qm2xpab"), "K7QM-2XPA");
});

test("a pasted code with a space lands in the shown form", () => {
  assert.equal(pastedPairCode("k7qm 2xpa"), "K7QM-2XPA");
  assert.equal(pastedPairCode("  K7QM-2XPA \n"), "K7QM-2XPA");
  // Not a whole code: the browser pastes, the edit tidies.
  assert.equal(pastedPairCode("k7qm"), null);
  assert.equal(pastedPairCode("https://proxysvpn.com/api/sub/x"), null);
  // The same paste, left to the browser, still reads right.
  const raw = "k7qm 2xpa";
  assert.deepEqual(editPairCode("", raw, raw.length, false), { value: "K7QM-2XPA", caret: 9 });
});

test("the caret stays where the person was typing", () => {
  // "K7QM-2XPA" with the caret after M; typing B pushes the rest along and
  // the last character falls off the end.
  const raw = "K7QMB-2XPA";
  assert.deepEqual(editPairCode("K7QM-2XPA", raw, 5, false), { value: "K7QM-B2XP", caret: 6 });
  // A refused character leaves the caret where it was.
  assert.deepEqual(editPairCode("K7Q", "K7Q0", 4, false), { value: "K7Q", caret: 3 });
  // Editing at the very start.
  assert.deepEqual(editPairCode("7QM", "K7QM", 1, false), { value: "K7QM", caret: 1 });
});

test("backspace at the end takes one character and the dash with the fifth", () => {
  assert.deepEqual(backspaceAt("K7QM-2XPA", 9), { value: "K7QM-2XP", caret: 8 });
  assert.deepEqual(backspaceAt("K7QM-2", 6), { value: "K7QM", caret: 4 });
});

test("backspace right after the dash takes the character before it", () => {
  // Otherwise the dash would come straight back and the key would seem dead.
  assert.deepEqual(backspaceAt("K7QM-2XPA", 5), { value: "K7Q2-XPA", caret: 3 });
});

test("an emptied field is empty", () => {
  assert.deepEqual(backspaceAt("K", 1), { value: "", caret: 0 });
});

test("the button waits for a whole code", () => {
  assert.equal(isCompletePairCode(""), false);
  assert.equal(isCompletePairCode("K7QM-2XP"), false);
  assert.equal(isCompletePairCode("K7QM-2XPA"), true);
  assert.equal(isCompletePairCode("K7QM2XPA"), true);
});

// ── refusals in words ──────────────────────────────────────────────────────

test("wrong, expired, used and a refused link read the same", () => {
  for (const code of ["PAIR_CODE_NOT_FOUND", "PAIR_CODE_MALFORMED", "SUB_INVALID", "SUB_MALFORMED"] as const) {
    assert.equal(pairCodeProblem(code), "notFound", code);
    assert.equal(pairCodeProblemKey(code, false), "pair.code.notFound", code);
  }
});

test("too many attempts, the network and everything else each have their line", () => {
  assert.equal(pairCodeProblemKey("PAIR_RATE_LIMITED", false), "pair.code.rateLimited");
  assert.equal(pairCodeProblemKey("SUB_UNREACHABLE", false), "pair.code.network");
  assert.equal(pairCodeProblemKey("NETWORK_OFFLINE", false), "pair.code.network");
  assert.equal(pairCodeProblemKey("UNKNOWN", false), "pair.code.failed");
  assert.equal(pairCodeProblemKey("TUN_FAILED", false), "pair.code.failed");
});

test("App Store builds send the person to their account, not the cabinet or the bot", () => {
  assert.equal(pairCodeProblemKey("PAIR_CODE_NOT_FOUND", true), "pair.appstore.code.notFound");
  // The other lines name neither, so both builds share them.
  assert.equal(pairCodeProblemKey("PAIR_RATE_LIMITED", true), "pair.code.rateLimited");
  assert.equal(pairCodeProblemKey("SUB_UNREACHABLE", true), "pair.code.network");
});

test("the contract's Russian wording is what the screen says", () => {
  const t = makeT("ru");
  assert.equal(t("pair.code.open"), "Ввести код с сайта");
  assert.equal(t("pair.code.placeholder"), "XXXX-XXXX");
  assert.equal(t("pair.code.submit"), "Подключить");
  assert.equal(
    t("pair.code.notFound"),
    "Код не подошёл. Он действует 10 минут и срабатывает один раз — возьмите новый в кабинете или в боте.",
  );
  assert.equal(t("pair.code.rateLimited"), "Слишком много попыток. Подождите минуту.");
});

test("every line an App Store build shows here passes the store's payment-word check", () => {
  for (const lang of ["ru", "en"] as const) {
    const t = makeT(lang);
    // Why the store build needs its own "not found" line at all.
    assert.equal(mentionsPayment(t("pair.code.notFound")), true, `${lang} direct`);
    for (const key of [
      "pair.appstore.code.open",
      "pair.appstore.code.submit",
      pairCodeProblemKey("PAIR_CODE_NOT_FOUND", true),
      pairCodeProblemKey("PAIR_RATE_LIMITED", true),
      pairCodeProblemKey("SUB_UNREACHABLE", true),
      pairCodeProblemKey("UNKNOWN", true),
      "pair.code.placeholder",
      "pair.code.label",
    ] as const) {
      assert.equal(mentionsPayment(t(key)), false, `${lang} ${key}: ${t(key)}`);
    }
  }
});

test("the button rests for Retry-After, at most the minute the line promises", () => {
  assert.equal(rateLimitPauseMs({ code: "PAIR_RATE_LIMITED", detail: "12" }), 12_000);
  assert.equal(rateLimitPauseMs({ code: "PAIR_RATE_LIMITED", detail: "3600" }), 60_000);
  assert.equal(rateLimitPauseMs({ code: "PAIR_RATE_LIMITED", detail: "0" }), 1_000);
  assert.equal(rateLimitPauseMs({ code: "PAIR_RATE_LIMITED" }), 60_000);
  assert.equal(rateLimitPauseMs({ code: "PAIR_RATE_LIMITED", detail: "soon" }), 60_000);
  assert.equal(rateLimitPauseMs({ code: "PAIR_RATE_LIMITED", detail: "-5" }), 60_000);
});

// ── pressing again (pair-code v1.1) ────────────────────────────────────────

test("no answer from any site reads as the network, never as a spent code", () => {
  // The answer may have been lost after the service spent the code; saying
  // "Код не подошёл" then would send the person for a new code they do not need.
  for (const code of ["SUB_UNREACHABLE", "NETWORK_OFFLINE"] as const) {
    assert.equal(pairCodeProblem(code), "network", code);
    for (const appstore of [false, true]) {
      assert.equal(pairCodeProblemKey(code, appstore), "pair.code.network", `${code} ${appstore}`);
    }
  }
  const t = makeT("ru");
  assert.equal(
    t("pair.code.network"),
    "Не удалось связаться с сервисом. Проверьте интернет и попробуйте ещё раз.",
  );
});

test("after a network failure the button can be pressed again at once", () => {
  assert.equal(pairCodeRestMs({ code: "SUB_UNREACHABLE" }), 0);
  assert.equal(pairCodeRestMs({ code: "NETWORK_OFFLINE" }), 0);
});

test("only too many attempts rests the button", () => {
  assert.equal(pairCodeRestMs({ code: "PAIR_RATE_LIMITED", detail: "12" }), 12_000);
  assert.equal(pairCodeRestMs({ code: "PAIR_RATE_LIMITED" }), 60_000);
  for (const code of [
    "PAIR_CODE_NOT_FOUND",
    "PAIR_CODE_MALFORMED",
    "SUB_INVALID",
    "SUB_MALFORMED",
    "UNKNOWN",
  ] as const) {
    assert.equal(pairCodeRestMs({ code }), 0, code);
  }
  // A Retry-After in the detail of anything but a 429 is not a reason to wait.
  assert.equal(pairCodeRestMs({ code: "SUB_UNREACHABLE", detail: "60" }), 0);
});
