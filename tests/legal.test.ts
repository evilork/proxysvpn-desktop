// tests/legal.test.ts
//
// node --test tests/legal.test.ts (Node >= 23.6 strips the types).
//
// The addresses are spelled out literally on purpose: a builder that drifts
// (a stray `?lang=`, a lost `?src=app`, a missing /en/, another host) must
// fail here, not in App Review. Each one must also pass the App Store allowlist, both as the real
// function answers and against the rules restated below.

import assert from "node:assert/strict";
import { test } from "node:test";

import { isAllowedExternal } from "../src/externalUrl.ts";
import {
  ACCOUNT_DELETE_URL,
  APP_VIEW_QUERY,
  privacyUrl,
  supportUrl,
  termsUrl,
} from "../src/legal.ts";

const EXPECTED = {
  ru: {
    privacy: "https://proxysvpn.com/privacy?src=app",
    terms: "https://proxysvpn.com/terms?src=app",
    support: "https://proxysvpn.com/support?src=app",
  },
  en: {
    privacy: "https://proxysvpn.com/en/privacy?src=app",
    terms: "https://proxysvpn.com/en/terms?src=app",
    support: "https://proxysvpn.com/en/support?src=app",
  },
} as const;

/**
 * The App Store rules of src/externalUrl.ts, restated as literal
 * expectations: https, no user or password, default port, no fragment,
 * exactly this host and exactly one of these paths with exactly its query —
 * `?src=app` (the site's app view) on a legal page, none on account deletion.
 */
const APPSTORE_HOST = "proxysvpn.com";
const APPSTORE_PAGES = new Map([
  ["/privacy", "?src=app"],
  ["/terms", "?src=app"],
  ["/support", "?src=app"],
  ["/en/privacy", "?src=app"],
  ["/en/terms", "?src=app"],
  ["/en/support", "?src=app"],
  ["/dashboard/account/delete", ""],
]);

function passesRestatedRules(url: string): boolean {
  const parsed = new URL(url);
  return (
    parsed.protocol === "https:" &&
    parsed.username === "" &&
    parsed.password === "" &&
    parsed.port === "" &&
    parsed.hash === "" &&
    parsed.hostname === APPSTORE_HOST &&
    APPSTORE_PAGES.get(parsed.pathname) === parsed.search
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

test("every legal page asks for the app view, and nothing carries a fragment", () => {
  for (const url of everyUrl()) {
    const expected = url === ACCOUNT_DELETE_URL ? "" : APP_VIEW_QUERY;
    assert.equal(new URL(url).search, expected, url);
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
