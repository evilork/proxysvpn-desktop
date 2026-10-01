// tests/platformCopy.test.ts
//
// node --test tests/platformCopy.test.ts (Node >= 23.6 strips the types).
//
// What the data notice and the Tunnel screen say about DNS must be true on
// the platform that shows it. Windows does not point the system resolver
// into the tunnel (net/windows.rs) and its tunnel is IPv4 only, so the
// desktop sentence "names go through the VPN server" is false there.

import assert from "node:assert/strict";
import { test } from "node:test";

import { makeT } from "../src/i18n.ts";
import { dnsNoticeKey, tunnelHintKeys } from "../src/platformCopy.ts";

test("iOS names the Apple engine's resolvers, every build", () => {
  assert.equal(dnsNoticeKey("ios", true), "notice.dns.apple");
  assert.equal(dnsNoticeKey("ios", false), "notice.dns.apple");
});

test("macOS and Linux keep the desktop sentence", () => {
  assert.equal(dnsNoticeKey("macos", false), "notice.dns.desktop");
  assert.equal(dnsNoticeKey("linux", false), "notice.dns.desktop");
});

test("Windows does not claim that lookups go through the VPN server", () => {
  assert.equal(dnsNoticeKey("windows", false), "notice.dns.windows");
  for (const lang of ["ru", "en"] as const) {
    const text = makeT(lang)("notice.dns.windows");
    assert.notEqual(text, "notice.dns.windows", `${lang}: the key has copy`);
    assert.notEqual(text, makeT(lang)("notice.dns.desktop"), lang);
  }
  // The English text is its own, not the Russian fallback.
  assert.notEqual(makeT("en")("notice.dns.windows"), makeT("ru")("notice.dns.windows"));
});

test("before app_info answers, the build decides", () => {
  assert.equal(dnsNoticeKey(undefined, true), "notice.dns.apple");
  assert.equal(dnsNoticeKey(undefined, false), "notice.dns.desktop");
  assert.equal(dnsNoticeKey("other", false), "notice.dns.desktop");
});

test("the Tunnel screen says on Windows that its DNS and IP choices stay inside the engine", () => {
  assert.deepEqual(tunnelHintKeys("windows"), {
    ip: "tun.ipHintWindows",
    dns: "tun.dnsHintWindows",
  });
  for (const platform of ["macos", "linux", "ios", "other", undefined] as const) {
    assert.deepEqual(tunnelHintKeys(platform), { ip: "tun.ipHint", dns: "tun.dnsHint" }, String(platform));
  }
  for (const lang of ["ru", "en"] as const) {
    const t = makeT(lang);
    assert.notEqual(t("tun.ipHintWindows"), t("tun.ipHint"), lang);
    assert.notEqual(t("tun.dnsHintWindows"), t("tun.dnsHint"), lang);
  }
});
