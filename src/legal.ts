// src/legal.ts
//
// The service's legal and support pages, as the app opens them.
//
// One module for every such address, so the list the App Store build may open
// (src/externalUrl.ts) and the buttons that open it cannot drift apart: each
// function here returns only addresses that allowlist accepts, and
// tests/legal.test.ts checks that for every language.
//
// Russian gets the site's root pages; every other language of the window gets
// the English ones under /en/ — the site serves those routes, and English is
// the one translation of the policy and terms that exists for all of them.
//
// No query strings (`?lang=en`) on purpose: the App Store allowlist refuses
// any query, because one can turn an allowed page into a redirect.
//
// Pure, with no build-time input: node --test imports it as is.

import type { Lang } from "./i18n";

/**
 * True only once the owner has confirmed that the VPN servers keep no access
 * logs (which sites a person opened). Until then no screen may say "we keep no
 * logs" or "we do not record sites": guideline 5.1.1 and 2.3.1 want privacy
 * statements that are true, and this one is not verified yet.
 */
export const NO_ACTIVITY_LOGS_CONFIRMED = false;

const SITE = "https://proxysvpn.com";

type LegalPage = "privacy" | "terms" | "support";

function pageUrl(page: LegalPage, lang: Lang): string {
  return lang === "ru" ? `${SITE}/${page}` : `${SITE}/en/${page}`;
}

/** Privacy policy. */
export function privacyUrl(lang: Lang): string {
  return pageUrl("privacy", lang);
}

/** Terms of use. */
export function termsUrl(lang: Lang): string {
  return pageUrl("terms", lang);
}

/** The public support page: how to reach a person, without the bot. */
export function supportUrl(lang: Lang): string {
  return pageUrl("support", lang);
}

/**
 * Deleting the account (guideline 5.1.1(v)): a page that deletes it, not the
 * cabinet in general. The page asks the person to confirm again and signs
 * them in if needed; the app only sends them there after its own warning.
 */
export const ACCOUNT_DELETE_URL = `${SITE}/dashboard/account/delete`;
