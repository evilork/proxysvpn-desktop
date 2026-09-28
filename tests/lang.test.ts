// tests/lang.test.ts
//
// node --test tests/lang.test.ts (Node >= 23.6 strips the types).

import assert from "node:assert/strict";
import { test } from "node:test";

import { detectLang } from "../src/lang.ts";

test("a language we do not have falls back to English, not Russian", () => {
  assert.equal(detectLang(["de-DE"]), "en");
  assert.equal(detectLang(["es-419", "fr-FR", "tr"]), "en");
});

test("Russian and the languages around it start in Russian", () => {
  assert.equal(detectLang(["ru-RU"]), "ru");
  assert.equal(detectLang(["uk-UA"]), "ru");
  assert.equal(detectLang(["kk"]), "ru");
  for (const tag of ["be-BY", "ky-KG", "uz-Latn-UZ", "tg-TJ", "hy-AM", "az-Latn-AZ"]) {
    assert.equal(detectLang([tag]), "ru", tag);
  }
});

test("the first language we can answer wins", () => {
  assert.equal(detectLang(["en-US", "ru"]), "en");
  assert.equal(detectLang(["ru", "en-US"]), "ru");
  // German first is skipped, Russian second is what the person asked for next.
  assert.equal(detectLang(["de-DE", "ru-RU"]), "ru");
  assert.equal(detectLang(["de-DE", "en-GB", "ru-RU"]), "en");
});

test("an empty list means English", () => {
  assert.equal(detectLang([]), "en");
});

test("tags are matched case-insensitively and with either separator", () => {
  assert.equal(detectLang(["RU"]), "ru");
  assert.equal(detectLang(["EN_us"]), "en");
  assert.equal(detectLang(["uk_UA"]), "ru");
});

test("a subtag is matched whole, never by prefix", () => {
  // "rue" (Rusyn) and "ukr"-like junk must not pass for Russian or Ukrainian.
  assert.equal(detectLang(["rue"]), "en");
  assert.equal(detectLang(["ukr"]), "en");
  assert.equal(detectLang(["english"]), "en");
});

test("blank and malformed entries are skipped", () => {
  assert.equal(detectLang(["", "  ", "-", "ru"]), "ru");
  assert.equal(detectLang(["*"]), "en");
});
