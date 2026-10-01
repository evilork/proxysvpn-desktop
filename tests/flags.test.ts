// tests/flags.test.ts
//
// node --test tests/flags.test.ts (Node >= 23.6 strips the types).

import assert from "node:assert/strict";
import { test } from "node:test";

import { rendersFlagEmoji } from "../src/flags.ts";

test("Windows draws flag emoji as boxed letters, so the list leaves them out", () => {
  const webview2 =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.0.0";
  assert.equal(rendersFlagEmoji(webview2), false);
});

test("Apple and Linux webviews keep the flags", () => {
  const mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
  const linux = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)";
  assert.equal(rendersFlagEmoji(mac), true);
  assert.equal(rendersFlagEmoji(linux), true);
});
