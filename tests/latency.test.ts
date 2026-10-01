// tests/latency.test.ts
//
// node --test tests/latency.test.ts
//
// With the VPN on, the Countries list read 1-5 ms for every location: the
// handshake went into our own tunnel. The core now measures around the tunnel
// or keeps the last number measured outside it; the window says how old such
// a kept number is instead of passing it off as fresh.

import assert from "node:assert/strict";
import { test } from "node:test";

import { makeT, formatAge } from "../src/i18n.ts";
import { RTT_AGE_SHOWN_AFTER_MS, rttMeasuredAtToShow } from "../src/latency.ts";

const NOW = 1_790_000_000_000;

test("a number from the round that just ran is shown plainly", () => {
  assert.equal(rttMeasuredAtToShow({ rttMs: 234, rttAtMs: NOW - 3_000 }, NOW), null);
  assert.equal(rttMeasuredAtToShow({ rttMs: 234, rttAtMs: NOW }, NOW), null);
});

test("a number kept from an earlier round carries its age", () => {
  const at = NOW - 12 * 60_000;
  assert.equal(rttMeasuredAtToShow({ rttMs: 234, rttAtMs: at }, NOW), at);
  assert.equal(
    rttMeasuredAtToShow({ rttMs: 234, rttAtMs: NOW - RTT_AGE_SHOWN_AFTER_MS }, NOW),
    NOW - RTT_AGE_SHOWN_AFTER_MS,
    "the threshold itself already counts as old",
  );
});

test("no number, or no time from an older core, means no age", () => {
  assert.equal(rttMeasuredAtToShow({ rttAtMs: NOW - 12 * 60_000 }, NOW), null);
  assert.equal(rttMeasuredAtToShow({ rttMs: 234 }, NOW), null);
});

test("the row reads as a measurement with its age in both languages", () => {
  const at = NOW - 12 * 60_000;
  const en = makeT("en");
  assert.equal(en("loc.msAged", { ms: 234, ago: formatAge(en, at, NOW) }), "234 ms · measured 12 minutes ago");
  const ru = makeT("ru");
  assert.equal(ru("loc.msAged", { ms: 234, ago: formatAge(ru, at, NOW) }), "234 мс · замер 12 минут назад");
});
