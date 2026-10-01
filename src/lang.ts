// src/lang.ts
//
// Which of our two languages a first run starts in, before the person has
// picked one in [12].
//
// The old rule was "English only if the system says English, Russian for
// everyone else", so a German, Spanish or Turkish phone opened a Russian app.
// That is wrong twice over: those people mostly do not read Russian, and an
// App Store build is reviewed in English on a device that may not be set to
// English at all.
//
// The rule now: walk the system's preferred languages in order and take the
// first one we can answer. English answers itself. Russian also answers for
// the languages of the countries around Russia (RU_CIRCLE), where Russian is
// far more widely read than English. Nothing recognised anywhere in the list
// means English.
//
// Pure, no DOM: the caller hands in `navigator.languages`, the tests hand in
// arrays.

import type { Lang } from "./i18n";

/**
 * Primary language subtags that start the app in Russian: Russian itself,
 * then Ukrainian, Belarusian, Kazakh, Kyrgyz, Uzbek, Tajik, Armenian and
 * Azerbaijani.
 */
const RU_CIRCLE: readonly string[] = ["ru", "uk", "be", "kk", "ky", "uz", "tg", "hy", "az"];

/** "ru-RU" -> "ru", "EN_us" -> "en". Tags come from the OS, not from us. */
function primarySubtag(tag: string): string {
  return tag.trim().split(/[-_]/, 1)[0].toLowerCase();
}

/**
 * The first preferred language we can answer, or English.
 *
 * ["en-US", "ru"] is English: the person put English first, and Russian
 * further down the list does not outrank that. ["de-DE", "ru"] is Russian:
 * German we do not have, and Russian is the next thing they asked for.
 */
export function detectLang(languages: readonly string[]): Lang {
  for (const tag of languages) {
    const primary = primarySubtag(tag);
    if (primary === "en") return "en";
    if (RU_CIRCLE.includes(primary)) return "ru";
  }
  return "en";
}
