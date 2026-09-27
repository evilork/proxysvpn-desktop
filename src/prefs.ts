// src/prefs.ts
//
// The two preferences the window owns — language and appearance — and the one
// rule about where they are applied.
//
// Why appearance is resolved HERE and not in CSS. It used to be split: the CSS
// carried a `@media (prefers-color-scheme: light)` copy of the light palette
// for "как в системе", and a `[data-theme="light"]` copy for a forced choice.
// Two copies of the same forty values, kept in step by hand. They did not stay
// in step: the forced copy never received the softened relief, inherited the
// dark theme's near-black shadow (#050507) and its near-black "highlight"
// (#18181f), and painted a black frame around every card on a white ground.
//
// So the resolution moved into code: `applyTheme` turns "system" into an
// explicit light|dark and always writes `data-theme`. The stylesheet then
// needs exactly one light rule, which cannot drift from anything.

import type { ThemePref } from "./components/ui";

export type { ThemePref };

/** What `data-theme` can actually say. "system" never reaches the DOM. */
export type ResolvedTheme = "light" | "dark";

export const LANG_KEY = "proxysvpn_lang";
export const THEME_KEY = "proxysvpn_theme";
/** Connect the tunnel the moment the window opens, with no button press. Not
 *  the same question as launching the APP at login — that one stays out of
 *  the settings screen on purpose (see MoreScreen.tsx) because macOS will
 *  not do it for us without a privileged helper. This is only about what the
 *  app does with itself once it is already open, by whatever means. */
export const AUTO_CONNECT_KEY = "proxysvpn_autoconnect";

/** The query is asked for LIGHT on purpose: anything else — no preference, an
 *  engine that does not know the feature — must land on dark, which is this
 *  app's ground state and was the CSS's behaviour before. */
const LIGHT_QUERY = "(prefers-color-scheme: light)";

export function readStored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    // Private windows and locked-down WebViews throw instead of returning
    // null; a preference is never worth breaking the app for.
    return null;
  }
}

export function writeStored(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Same reason: losing a stored preference is survivable.
  }
}

/** The stored choice, or "system" when there is nothing usable stored. */
export function readThemePref(): ThemePref {
  const stored = readStored(THEME_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

function lightQuery(): MediaQueryList | null {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return null;
  }
  return window.matchMedia(LIGHT_QUERY);
}

/** Turn a preference into the theme actually painted right now. */
export function resolveTheme(pref: ThemePref): ResolvedTheme {
  if (pref === "light" || pref === "dark") return pref;
  return lightQuery()?.matches ? "light" : "dark";
}

/**
 * Write the resolved theme onto `<html>`.
 *
 * Called once before React mounts (main.tsx) so the first frame is already the
 * right colour, and again whenever the choice or the system changes.
 */
export function applyTheme(pref: ThemePref): ResolvedTheme {
  const resolved = resolveTheme(pref);
  if (typeof document !== "undefined") {
    document.documentElement.setAttribute("data-theme", resolved);
  }
  return resolved;
}

/**
 * Follow the system while the choice is "system".
 *
 * Returns the unsubscribe. `addListener` is the deprecated form kept for
 * WebKit older than Safari 14, which some of our macOS users still run; it is
 * the only way to hear the change there.
 */
export function watchSystemTheme(onChange: () => void): () => void {
  const query = lightQuery();
  if (!query) return () => undefined;

  const handler = (): void => onChange();

  if (typeof query.addEventListener === "function") {
    query.addEventListener("change", handler);
    return () => query.removeEventListener("change", handler);
  }

  query.addListener(handler);
  return () => query.removeListener(handler);
}
