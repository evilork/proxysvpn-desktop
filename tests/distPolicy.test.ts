// tests/distPolicy.test.ts
//
// node --test tests/distPolicy.test.ts (Node >= 23.6 strips the types).
// src/dist.ts itself reads `import.meta.env`, which exists only under Vite,
// so the rule is tested where it lives.

import assert from "node:assert/strict";
import { test } from "node:test";

import { decidePurchaseLinks } from "../src/distPolicy.ts";

test("direct builds keep every purchase link", () => {
  assert.equal(decidePurchaseLinks(false), true);
});

test("App Store builds carry no purchase link on any storefront", () => {
  assert.equal(decidePurchaseLinks(true), false);
});
