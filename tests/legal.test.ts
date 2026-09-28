// tests/legal.test.ts
//
// node --test tests/legal.test.ts (Node >= 23.6 strips the types).
//
// The addresses are spelled out literally on purpose: a builder that drifts
// (a stray `?lang=`, a missing /en/, another host) must fail here, not in App
// Review. Each one must also pass the App Store allowlist, both as the real
// function answers and against the rules restated below.

import assert from "node:assert/strict";
import { test } from "node:test";

import { isAllowedExternal } from "../src/externalUrl.ts";
import {
  ACCOUNT_DELETE_URL,
  privacyUrl,
  supportUrl,
  termsUrl,
} from "../src/legal.ts";

const EXPECTED = {
  ru: {
    privacy: "https://proxysvpn.com/privacy",
    terms: "https://proxysvpn.com/terms",
    support: "https://proxysvpn.com/support",
  },
  en: {
    privacy: "https://proxysvpn.com/en/privacy",
    terms: "https://proxysvpn.com/en/terms",
    support: "https://proxysvpn.com/en/support",
  },
} as const;

/**
 * The App Store rules of src/externalUrl.ts, restated as literal
 * expectations: https, no user or password, default port, no query, no
 * fragment, exactly this host and exactly one of these paths.
 */
const APPSTORE_HOST = "proxysvpn.com";
const APPSTORE_PATHS = new Set([
  "/privacy",
  "/terms",
  "/support",
  "/en/privacy",
  "/en/terms",
  "/en/support",
  "/dashboard/account/delete",
]);

function passesRestatedRules(url: string): boolean {
  const parsed = new URL(url);
  return (
    parsed.protocol === "https:" &&
    parsed.username === "" &&
    parsed.password === "" &&
    parsed.port === "" &&
    parsed.search === "" &&
    parsed.hash === "" &&
    parsed.hostname === APPSTORE_HOST &&
    APPSTORE_PATHS.has(parsed.pathname)
  );
}

function everyUrl(): string[] {
  return [
    ...(["ru", "en"] as const).flatMap((lang) => [
      privacyUrl(lang),
      termsUrl(lang),
      supportUrl(lang),
    ]),
    ACCOUNT_DELETE_URL,
  ];
}

test("Russian opens the site's root pages", () => {
  assert.equal(privacyUrl("ru"), EXPECTED.ru.privacy);
  assert.equal(termsUrl("ru"), EXPECTED.ru.terms);
  assert.equal(supportUrl("ru"), EXPECTED.ru.support);
});

test("every other language opens the English pages under /en/", () => {
  assert.equal(privacyUrl("en"), EXPECTED.en.privacy);
  assert.equal(termsUrl("en"), EXPECTED.en.terms);
  assert.equal(supportUrl("en"), EXPECTED.en.support);
});

test("account deletion is the one cabinet page, spelled exactly", () => {
  assert.equal(ACCOUNT_DELETE_URL, "https://proxysvpn.com/dashboard/account/delete");
});

test("no address carries a query or a fragment", () => {
  for (const url of everyUrl()) {
    assert.ok(!url.includes("?"), url);
    assert.ok(!url.includes("#"), url);
  }
});

test("every address passes the App Store allowlist", () => {
  for (const url of everyUrl()) {
    assert.equal(isAllowedExternal(url, true), true, url);
    assert.equal(passesRestatedRules(url), true, url);
  }
});

test("the restated rules still refuse what the allowlist refuses", () => {
  for (const url of [
    "https://proxysvpn.com/privacy?lang=en",
    "https://proxysvpn.com/dashboard",
    "http://proxysvpn.com/privacy",
    "https://proxysvpn.com/privacy#top",
    "https://t.me/proxysvpn_bot",
  ]) {
    assert.equal(passesRestatedRules(url), false, url);
    assert.equal(isAllowedExternal(url, true), false, url);
  }
});
