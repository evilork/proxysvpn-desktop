// tests/locationName.test.ts
//
// node --test tests/locationName.test.ts (Node >= 23.6 strips the types).
//
// The service names locations in Russian whatever the request's `lang`, so an
// English window used to show "Protected / Германия".

import assert from "node:assert/strict";
import { test } from "node:test";

import { displayLocation } from "../src/locationName.ts";

test("an English window shows the service's Russian names in English", () => {
  assert.equal(displayLocation("Германия", "en"), "Germany");
  assert.equal(displayLocation("Нидерланды", "en"), "Netherlands");
  assert.equal(displayLocation("Британия", "en"), "United Kingdom");
  assert.equal(displayLocation("Великобритания", "en"), "United Kingdom");
});

test("what follows the name is kept", () => {
  assert.equal(displayLocation("Германия 2", "en"), "Germany 2");
  assert.equal(displayLocation("Британия · XHTTP", "en"), "United Kingdom · XHTTP");
});

test("a Russian window and unknown names are left exactly as written", () => {
  assert.equal(displayLocation("Германия", "ru"), "Германия");
  assert.equal(displayLocation("Atlantis-1", "en"), "Atlantis-1");
  assert.equal(displayLocation("Германияx", "en"), "Германияx", "a prefix of a word is not the name");
  assert.equal(displayLocation("", "en"), "");
});

// The words AROUND the names. The service's registry writes its badges and
// quota lines in Russian as well, and an English window read
// "Amsterdam · резерв" (Windows VM, 02.10.2026).

test("a badge after the name is English in an English window", () => {
  assert.equal(displayLocation("Амстердам · резерв", "en"), "Amsterdam · reserve");
  assert.equal(displayLocation("резерв", "en"), "reserve", "the badge alone, as LocationsScreen passes it");
  assert.equal(displayLocation("ЛЮКС", "en"), "DELUXE");
  assert.equal(displayLocation("Германия · квота", "en"), "Germany · quota");
  assert.equal(displayLocation("Британия · XHTTP недоступен", "en"), "United Kingdom · XHTTP unavailable");
});

test("quota lines keep their numbers and dates, and the units become English", () => {
  assert.equal(displayLocation("США (50 ГБ/мес)", "en"), "United States (50 GB/mo)");
  assert.equal(displayLocation("США · 0.2/50 ГБ", "en"), "United States · 0.2/50 GB");
  assert.equal(displayLocation("12,4 из 50 ГБ", "en"), "12,4 of 50 GB");
  assert.equal(
    displayLocation("США · использовано 50/50 ГБ · сброс 05.10", "en"),
    "United States · used 50/50 GB · resets 05.10",
  );
  assert.equal(displayLocation("12,4 из 50 ГБ, XHTTP недоступен", "en"), "12,4 of 50 GB, XHTTP unavailable");
  assert.equal(displayLocation("512 МБ", "en"), "512 MB");
  assert.equal(displayLocation("1 ТБ", "en"), "1 TB");
});

test("the case the service wrote a word in is kept", () => {
  assert.equal(displayLocation("Резерв", "en"), "Reserve");
  assert.equal(displayLocation("РЕЗЕРВ", "en"), "RESERVE");
  assert.equal(displayLocation("Люкс", "en"), "Deluxe");
  assert.equal(displayLocation("люкс", "en"), "deluxe");
  assert.equal(displayLocation("50 гб", "en"), "50 GB", "a unit is a unit whatever its case");
  assert.equal(displayLocation("🌐 Автовыбор", "en"), "🌐 Auto");
});

test("names are translated wherever they stand, not only first", () => {
  assert.equal(displayLocation("Россия & Москва", "en"), "Russia & Moscow");
  assert.equal(displayLocation("Нидерланды-2", "en"), "Netherlands-2");
  assert.equal(displayLocation("Швеция <резерв>", "en"), "Sweden <reserve>");
});

test("only whole known words change; Latin badges and unknown words stay", () => {
  assert.equal(displayLocation("Германия · PRO", "en"), "Germany · PRO");
  assert.equal(displayLocation("Амстердам · новый", "en"), "Amsterdam · новый", "an unknown word is shown as written");
  assert.equal(displayLocation("резервный", "en"), "резервный", "a longer word is not «резерв»");
  assert.equal(displayLocation("Гбит", "en"), "Гбит");
  assert.equal(displayLocation("Изумруд", "en"), "Изумруд", "«из» inside a word is not the preposition");
});

test("a Russian window shows badges and quota lines exactly as the service wrote them", () => {
  for (const label of [
    "Амстердам · резерв",
    "США (50 ГБ/мес)",
    "США · 0.2/50 ГБ",
    "12,4 из 50 ГБ",
    "США · использовано 50/50 ГБ · сброс 05.10",
    "ЛЮКС",
  ]) {
    assert.equal(displayLocation(label, "ru"), label);
  }
});

test("no Russian is left in any badge the service is known to send", () => {
  const cyrillic = /[Ѐ-ӿ]/u;
  for (const label of [
    "Амстердам · резерв",
    "Нидерланды · TCP",
    "США (50 ГБ/мес)",
    "США · 0.2/50 ГБ",
    "США · использовано 50/50 ГБ · сброс 05.10",
    "США · 12,4 из 50 ГБ, XHTTP недоступен",
    "Германия · ЛЮКС",
    "Швеция · квота",
  ]) {
    const shown = displayLocation(label, "en");
    assert.ok(!cyrillic.test(shown), `${label} -> ${shown}`);
  }
});
