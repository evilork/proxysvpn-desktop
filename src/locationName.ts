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
 * The name to show for `label` in `lang`. A known Russian name, alone or
 * followed by more ("Германия 2", "Германия · XHTTP"), is translated for an
 * English window; anything else is returned unchanged.
 */
export function displayLocation(label: string, lang: Lang): string {
  if (lang !== "en") return label;
  for (const [ru, en] of EN_NAMES) {
    if (label === ru) return en;
    if (label.startsWith(`${ru} `)) return `${en}${label.slice(ru.length)}`;
  }
  return label;
}
