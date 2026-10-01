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
