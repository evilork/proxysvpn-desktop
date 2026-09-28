// src/externalUrl.ts
//
// Which addresses the app may hand to the system browser.
//
// Direct builds: the same four schemes the opener plugin's default scope lets
// through (`opener:default` in capabilities/default.json). The plugin stays the
// gate there; this only mirrors it, so asking "would this open?" has one answer.
//
// App Store builds: a short allowlist, and nothing else. The app ships without
// in-app purchases, so no button may lead to a page where one can pay
// (guidelines 3.1.1 and 3.1.3). Several addresses reach the app from the
// SERVICE — announce-url, support-url — and can change after review without a
// new build; this list is what keeps such an edit from putting the cabinet, a
// payment page or the bot (it sells top-ups) behind a reviewed button.
//
// Pure on purpose, with no build-time input: node --test imports it as is.

/** Languages of proxysvpn.com (frontend/src/i18n/langs.ts). */
const SITE_LANGS = ["ru", "en", "zh", "es", "tr", "ar", "ja", "de", "fr", "ko"] as const;

/** Pages an App Store build may open on proxysvpn.com, in every language. */
const SITE_PAGES = ["privacy", "terms", "support"] as const;

const PROXYSVPN_PATHS: ReadonlySet<string> = new Set<string>([
  ...SITE_PAGES.map((page) => `/${page}`),
  ...SITE_LANGS.flatMap((lang) => SITE_PAGES.map((page) => `/${lang}/${page}`)),
  // Account deletion (guideline 5.1.1(v)). The one cabinet page allowed: it
  // deletes, it does not sell.
  "/dashboard/account/delete",
]);

const WATAFAST_PATHS: ReadonlySet<string> = new Set<string>(["/", "/privacy", "/terms", "/support"]);

/** Exact host -> exact paths. No wildcard hosts, no path prefixes. */
const APPSTORE_ALLOWED: ReadonlyMap<string, ReadonlySet<string>> = new Map([
  ["proxysvpn.com", PROXYSVPN_PATHS],
  ["www.proxysvpn.com", PROXYSVPN_PATHS],
  ["wata.fast", WATAFAST_PATHS],
]);

/** What `opener:default` allows (tauri-plugin-opener, allow-default-urls). */
const DIRECT_SCHEMES: ReadonlySet<string> = new Set(["http:", "https:", "mailto:", "tel:"]);

function parse(url: string): URL | null {
  try {
    return new URL(url);
  } catch {
    return null;
  }
}

/** `proxysvpn.com.` is the same name as `proxysvpn.com`; one dot, not more. */
function normalHost(hostname: string): string {
  const lower = hostname.toLowerCase();
  return lower.endsWith(".") ? lower.slice(0, -1) : lower;
}

function isIpLiteral(host: string): boolean {
  // The URL parser has already turned every IPv4 spelling (hex, octal, short
  // forms) into dotted decimal, and IPv6 always keeps its brackets.
  return host.startsWith("[") || /^\d{1,3}(\.\d{1,3}){3}$/.test(host);
}

/**
 * The path as the server will see it, or null when it hides something.
 *
 * `new URL` already resolves `..` and `%2e%2e` segments, so `/privacy/../pay`
 * arrives here as `/pay` and simply is not on the list. Decoding once more
 * catches what the parser leaves alone — an encoded slash or backslash that
 * a server may still treat as a separator (`/privacy%2F..%2Fpay`).
 */
function normalPath(pathname: string): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (decoded.includes("\\") || decoded.includes("\0")) return null;
  const segments = decoded.split("/");
  if (segments.some((segment) => segment === ".." || segment === ".")) return null;
  // `/privacy/` is `/privacy`; the root stays `/`.
  return decoded.length > 1 && decoded.endsWith("/") ? decoded.slice(0, -1) : decoded;
}

/**
 * May the app open `url` in the system browser?
 *
 * App Store builds: https only, no user or password part, the default port,
 * a named host (never an address), no query and no fragment — then the exact
 * host and the exact path must both be on the list above.
 */
export function isAllowedExternal(url: string, isAppstore: boolean): boolean {
  const parsed = parse(url.trim());
  if (!parsed) return false;
  if (!isAppstore) return DIRECT_SCHEMES.has(parsed.protocol);

  if (parsed.protocol !== "https:") return false;
  if (parsed.username !== "" || parsed.password !== "") return false;
  // The parser drops an explicit :443, so any port left here is not the default.
  if (parsed.port !== "") return false;
  // A query or a fragment can turn an allowed page into a redirect to one that
  // is not (`/support?next=/pay`); none of the allowed pages needs either.
  if (parsed.search !== "" || parsed.hash !== "") return false;

  const host = normalHost(parsed.hostname);
  if (host === "" || isIpLiteral(host)) return false;
  const paths = APPSTORE_ALLOWED.get(host);
  if (!paths) return false;

  const path = normalPath(parsed.pathname);
  return path !== null && paths.has(path);
}
