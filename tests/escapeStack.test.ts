// tests/escapeStack.test.ts
//
// node --test tests/escapeStack.test.ts (Node >= 23.6 strips the types).

import assert from "node:assert/strict";
import { test } from "node:test";

import { createEscapeStack } from "../src/escapeStack.ts";

test("Esc on a sheet over a screen closes the sheet only", () => {
  const stack = createEscapeStack();
  const closed: string[] = [];
  stack.push(() => closed.push("more screen"));
  const removeSheet = stack.push(() => closed.push("delete-account sheet"));

  assert.equal(stack.handle("Escape"), true);
  assert.deepEqual(closed, ["delete-account sheet"]);

  removeSheet();
  stack.handle("Escape");
  assert.deepEqual(closed, ["delete-account sheet", "more screen"]);
});

test("other keys and an empty stack are left alone", () => {
  const stack = createEscapeStack();
  assert.equal(stack.handle("Escape"), false);
  let closed = 0;
  stack.push(() => closed++);
  assert.equal(stack.handle("Enter"), false);
  assert.equal(closed, 0);
});

test("removing a layer twice, or out of order, removes only that layer", () => {
  const stack = createEscapeStack();
  const closed: string[] = [];
  const removeA = stack.push(() => closed.push("a"));
  stack.push(() => closed.push("b"));
  removeA();
  removeA();
  stack.handle("Escape");
  assert.deepEqual(closed, ["b"]);
});
