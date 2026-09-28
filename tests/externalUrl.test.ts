// tests/externalUrl.test.ts
//
// node --test tests/externalUrl.test.ts (Node >= 23.6 strips the types).

import assert from "node:assert/strict";
import { test } from "node:test";

import { isAllowedExternal } from "../src/externalUrl.ts";

const SITE_LANGS = ["ru", "en", "zh", "es", "tr", "ar", "ja", "de", "fr", "ko"];

const ALLOWED_IN_APPSTORE = [
  "https://proxysvpn.com/privacy",
  "https://proxysvpn.com/terms",
  "https://proxysvpn.com/support",
  "https://www.proxysvpn.com/privacy",
  ...SITE_LANGS.flatMap((lang) => [
    `https://proxysvpn.com/${lang}/privacy`,
    `https://proxysvpn.com/${lang}/terms`,
    `https://proxysvpn.com/${lang}/support`,
  ]),
  "https://proxysvpn.com/dashboard/account/delete",
  "https://wata.fast/",
  "https://wata.fast",
  "https://wata.fast/privacy",
  "https://wata.fast/terms",
  "https://wata.fast/support",
  // Spellings of an allowed address.
  "https://PROXYSVPN.COM/privacy",
  "https://proxysvpn.com./privacy",
  "https://proxysvpn.com:443/privacy",
  "https://proxysvpn.com/privacy/",
  // The bundled support pages App.tsx falls back to.
  "https://proxysvpn.com/en/support",
];

const BLOCKED_IN_APPSTORE = [
  // The cabinet and anything that sells.
  "https://proxysvpn.com/",
  "https://proxysvpn.com/dashboard",
  "https://proxysvpn.com/dashboard/",
  "https://proxysvpn.com/dashboard/pay",
  "https://proxysvpn.com/pay",
  "https://proxysvpn.com/offers",
  "https://proxysvpn.com/promo",
  "https://proxysvpn.com/dashboard/account/delete/../../pay",
  "https://proxysvpn.com/dashboard/../pay",
  "https://proxysvpn.com/privacy/../dashboard",
  "https://proxysvpn.com/privacy/%2e%2e/dashboard",
  "https://proxysvpn.com/privacy%2F..%2Fdashboard",
  "https://proxysvpn.com/privacy%5C..%5Cdashboard",
  "https://proxysvpn.com/%E0%A4%A",
  "https://proxysvpn.store/",
  "https://proxysvpn.store/privacy",
  "https://wata.fast/pay",
  "https://sub.proxysvpn.com/privacy",
  "https://proxysvpn.com.evil.example/privacy",
  "https://evilproxysvpn.com/privacy",
  // Telegram, the bot sells top-ups; Apple's subscription page (no IAP).
  "https://t.me/proxysvpn_bot",
  "https://t.me/other_bot",
  "https://apps.apple.com/account/subscriptions",
  // Tricks on an allowed page.
  "https://proxysvpn.com@evil.example/privacy",
  "https://user:pass@proxysvpn.com/privacy",
  "https://proxysvpn.com:8443/privacy",
  "http://proxysvpn.com/privacy",
  "https://proxysvpn.com/support?next=/dashboard/pay",
  "https://proxysvpn.com/privacy#pay",
  "https://proxysvpn.com/PRIVACY",
  "https://proxysvpn.com../privacy",
  // Addresses, never names.
  "https://127.0.0.1/privacy",
  "https://0x7f.1/privacy",
  "https://[::1]/privacy",
  // Other schemes.
  "mailto:support@proxysvpn.com",
  "tel:+10000000000",
  "javascript:alert(1)",
  "App-prefs:General&path=VPN",
  "x-apple.systempreferences:com.apple.LoginItems-Settings.extension",
  "proxysvpn.com/privacy",
  "",
  "not a url",
];

test("App Store builds open the legal, support and delete pages", () => {
  for (const url of ALLOWED_IN_APPSTORE) {
    assert.equal(isAllowedExternal(url, true), true, url);
  }
});

test("App Store builds open nothing else", () => {
  for (const url of BLOCKED_IN_APPSTORE) {
    assert.equal(isAllowedExternal(url, true), false, url);
  }
});

test("direct builds allow what the opener plugin's default scope allows", () => {
  for (const url of [
    "https://proxysvpn.com/dashboard",
    "https://t.me/proxysvpn_bot",
    "https://example.com/anything?at=all#really",
    "http://proxysvpn.com/privacy",
    "mailto:support@example.com",
    "tel:+10000000000",
    ...ALLOWED_IN_APPSTORE,
  ]) {
    assert.equal(isAllowedExternal(url, false), true, url);
  }
  for (const url of [
    "javascript:alert(1)",
    "App-prefs:General&path=VPN",
    "file:///etc/hosts",
    "",
    "not a url",
  ]) {
    assert.equal(isAllowedExternal(url, false), false, url);
  }
});
