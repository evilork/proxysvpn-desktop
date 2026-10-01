// src/locationName.ts
//
// Location names as the window shows them.
//
// The service names every location in Russian ("Германия", "Нидерланды"):
// those names come from the server's own remarks, which do not depend on the
// `lang` of the request. An English window therefore read
// "Protected / Германия · 234 ms" (screenshot linux-03, 01.10). Until the
// service localises its remarks, the window translates the names it knows and
// shows anything else exactly as the service wrote it. Display only: the core
// keeps matching locations by the service's own name.
//
// The same goes for the words AROUND the names. The service's registry writes
// badges and quota lines in Russian too — «Амстердам · резерв», «США (50
// ГБ/мес)», «США · 0.2/50 ГБ», «США · использовано 50/50 ГБ · сброс 05.10»,
// «12,4 из 50 ГБ» — and this app adds «XHTTP недоступен» when a location has
// to fall back to another transport (manifest.rs). An English window read
// "Amsterdam · резерв" (Windows VM, 02.10.2026). Every word is translated on
// its own, so the numbers, dates and punctuation between them stay exactly as
// the service wrote them; only "ГБ" becomes "GB" and so on.
//
// src-tauri/src/location_name.rs is the tray's copy of both tables, and a test
// there fails the build when the two drift apart.

import type { Lang } from "./i18n";

/** The service's Russian names, and what an English window calls them. */
const EN_NAMES: readonly (readonly [string, string])[] = [
  ["Нидерланды", "Netherlands"],
  ["Германия", "Germany"],
  ["Великобритания", "United Kingdom"],
  ["Британия", "United Kingdom"],
  ["США", "United States"],
  ["Франция", "France"],
  ["Финляндия", "Finland"],
  ["Швеция", "Sweden"],
  ["Польша", "Poland"],
  ["Казахстан", "Kazakhstan"],
  ["Турция", "Türkiye"],
  ["Россия", "Russia"],
  ["Латвия", "Latvia"],
  ["Литва", "Lithuania"],
  ["Эстония", "Estonia"],
  ["Швейцария", "Switzerland"],
  ["Испания", "Spain"],
  ["Италия", "Italy"],
  ["Австрия", "Austria"],
  ["Норвегия", "Norway"],
  ["Дания", "Denmark"],
  ["Чехия", "Czechia"],
  ["Румыния", "Romania"],
  ["Болгария", "Bulgaria"],
  ["Сербия", "Serbia"],
  ["Грузия", "Georgia"],
  ["Армения", "Armenia"],
  ["Молдова", "Moldova"],
  ["Украина", "Ukraine"],
  ["Ирландия", "Ireland"],
  ["Бельгия", "Belgium"],
  ["Люксембург", "Luxembourg"],
  ["Венгрия", "Hungary"],
  ["Португалия", "Portugal"],
  ["Израиль", "Israel"],
  ["ОАЭ", "UAE"],
  ["Индия", "India"],
  ["Япония", "Japan"],
  ["Сингапур", "Singapore"],
  ["Гонконг", "Hong Kong"],
  ["Корея", "Korea"],
  ["Канада", "Canada"],
  ["Бразилия", "Brazil"],
  ["Австралия", "Australia"],
  ["Москва", "Moscow"],
  ["Амстердам", "Amsterdam"],
  ["Франкфурт", "Frankfurt"],
  ["Лондон", "London"],
  ["Хельсинки", "Helsinki"],
  ["Стокгольм", "Stockholm"],
];

/**
 * Words the service writes around the names, keyed in lower case. Matched as
 * whole words in any case; the case of the service's word is kept: «резерв»
 * → "reserve", «Резерв» → "Reserve", «ЛЮКС» → "DELUXE". Units are written as
 * they are in English whatever the case they came in.
 */
const EN_WORDS: readonly (readonly [string, string])[] = [
  ["резерв", "reserve"],
  ["люкс", "deluxe"],
  ["квота", "quota"],
  ["использовано", "used"],
  ["сброс", "resets"],
  ["из", "of"],
  ["недоступен", "unavailable"],
  ["автовыбор", "auto"],
  ["гб", "GB"],
  ["мб", "MB"],
  ["тб", "TB"],
  ["мес", "mo"],
];

const NAMES = new Map<string, string>(EN_NAMES.map(([ru, en]) => [ru, en]));
const WORDS = new Map<string, string>(EN_WORDS.map(([ru, en]) => [ru, en]));

/** `en` in the case the service wrote `word` in. */
function inCaseOf(word: string, en: string): string {
  const upper = word.toLocaleUpperCase("ru");
  if (word.length > 1 && word === upper && word !== word.toLocaleLowerCase("ru")) {
    return en.toUpperCase();
  }
  const first = word.charAt(0);
  if (first !== first.toLocaleLowerCase("ru")) {
    return en.charAt(0).toUpperCase() + en.slice(1);
  }
  return en;
}

/** One whole word: a known name exactly as the service spells it, then a known word in any case. */
function translateWord(word: string): string {
  const name = NAMES.get(word);
  if (name !== undefined) return name;
  const known = WORDS.get(word.toLocaleLowerCase("ru"));
  return known === undefined ? word : inCaseOf(word, known);
}

/**
 * The name to show for `label` in `lang`. In an English window every known
 * Russian word — a location name ("Германия 2", "Германия · XHTTP") or a word
 * of a badge ("Амстердам · резерв", "США (50 ГБ/мес)") — is translated where
 * it stands; everything else, numbers included, is returned unchanged. A word
 * is a whole run of letters, so "Германияx" is not "Германия".
 */
export function displayLocation(label: string, lang: Lang): string {
  if (lang !== "en") return label;
  return label.replace(/\p{L}+/gu, translateWord);
}
