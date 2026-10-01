// tests/notices.test.ts
//
// node --test tests/notices.test.ts (Node >= 23.6 strips the types).
//
// The committed notices file as the licences screen will read it. Whether it
// is up to date with the lock files is `node scripts/gen-notices.mjs --check`.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";

import { groupByLicense, parseNotices, type NoticeComponent } from "../src/notices.ts";

const ROOT = new URL("../", import.meta.url);

function committed(): NoticeComponent[] {
  const text = readFileSync(new URL("src/assets/third-party-notices.json", ROOT), "utf8");
  return parseNotices(JSON.parse(text));
}

function sample(overrides: Partial<NoticeComponent>): NoticeComponent {
  return {
    kind: "cargo",
    name: "x",
    version: "1.0.0",
    license: "MIT",
    licenseIds: ["MIT"],
    source: "https://example.com/x",
    platforms: ["ios", "macos"],
    ...overrides,
  };
}

test("the committed notices parse", () => {
  assert.ok(committed().length > 100);
});

test("every licence a component names has its text bundled", () => {
  for (const component of committed()) {
    for (const id of component.licenseIds) {
      const file = new URL(`src/assets/licenses/${id}.txt`, ROOT);
      assert.ok(existsSync(file), `${component.name}: ${id}`);
    }
  }
});

test("the Apple engine is listed as Xray-core and libXray, not sing-box", () => {
  const ios = committed().filter((c) => c.platforms.includes("ios"));
  const xray = ios.find((c) => c.name === "Xray-core");
  const libxray = ios.find((c) => c.name === "libXray");
  assert.equal(xray?.license, "MPL-2.0");
  assert.equal(libxray?.license, "MIT");
  assert.equal(ios.some((c) => /sing-?box|libbox/i.test(c.name)), false);
});

test("the iOS Xray-core names the pinned commit and our patch (MPL-2.0 source)", () => {
  const all = committed();
  const ios = all.find((c) => c.name === "Xray-core" && c.platforms.includes("ios"));
  assert.ok(ios?.version, "pinned by build-libxray.sh");
  assert.match(ios.source, /^https:\/\/github\.com\/XTLS\/Xray-core\/tree\/[0-9a-f]{40}$/);
  // The link points at a file that exists here, under the same path.
  const patch = ios.changes?.match(/\/blob\/main\/(.+)$/)?.[1];
  assert.ok(patch, String(ios.changes));
  assert.ok(existsSync(new URL(patch, ROOT)), patch);
  const libxray = all.find((c) => c.name === "libXray");
  assert.match(libxray?.source ?? "", /^https:\/\/github\.com\/XTLS\/libXray\/tree\/[0-9a-f]{40}$/);
  // The Mac app fetches the latest unpatched release: no version, no patch.
  const mac = all.find((c) => c.name === "Xray-core" && c.platforms.includes("macos"));
  assert.equal(mac?.source, "https://github.com/XTLS/Xray-core");
  assert.equal(mac?.changes, undefined);
  assert.notEqual(mac, ios);
});

test("the engines are not listed twice, and our own code not at all", () => {
  const names = committed().map((c) => c.name);
  assert.ok(!names.includes("github.com/xtls/xray-core"));
  assert.ok(!names.includes("github.com/xtls/libxray"));
  assert.ok(!names.some((name) => /ratelimit/i.test(name)));
  assert.ok(names.includes("golang.org/x/crypto"), "the other Go modules are");
});

test("no GPL component ships in any build", () => {
  for (const component of committed()) {
    assert.ok(!component.licenseIds.some((id) => id.startsWith("GPL")), component.name);
  }
});

test("an iOS screen leaves out what only the Mac app carries", () => {
  const groups = groupByLicense(committed(), "ios");
  const names = groups.flatMap((g) => g.components.map((c) => c.name));
  assert.ok(names.includes("Xray-core"));
  assert.ok(!names.includes("Hysteria"));
  assert.ok(!names.includes("tun2socks"));
});

test("an unknown platform lists everything", () => {
  const all = committed();
  const listed = groupByLicense(all, undefined).reduce((n, g) => n + g.components.length, 0);
  assert.equal(listed, all.length);
});

test("groups: largest first, then by licence; components by name", () => {
  const groups = groupByLicense(
    [
      sample({ name: "b" }),
      sample({ name: "a" }),
      sample({ name: "z", license: "Apache-2.0", licenseIds: ["Apache-2.0"] }),
      sample({ name: "y", license: "ISC", licenseIds: ["ISC"] }),
    ],
    "ios",
  );
  assert.deepEqual(
    groups.map((g) => [g.license, g.components.map((c) => c.name)]),
    [
      ["MIT", ["a", "b"]],
      ["Apache-2.0", ["z"]],
      ["ISC", ["y"]],
    ],
  );
});

test("a malformed file is refused, not half-shown", () => {
  assert.throws(() => parseNotices(null));
  assert.throws(() => parseNotices({ components: "nope" }));
  assert.throws(() => parseNotices({ components: [{ ...sample({}), license: "" }] }));
  assert.throws(() => parseNotices({ components: [{ ...sample({}), source: "http://x" }] }));
  assert.throws(() => parseNotices({ components: [{ ...sample({}), platforms: ["android"] }] }));
  assert.throws(() => parseNotices({ components: [{ ...sample({}), kind: "pip" }] }));
  assert.throws(() => parseNotices({ components: [{ ...sample({}), licenseIds: [] }] }));
  assert.throws(() => parseNotices({ components: [{ ...sample({}), changes: "scripts/x.patch" }] }));
  assert.equal(
    parseNotices({ components: [sample({ changes: "https://example.com/x.patch" })] })[0]?.changes,
    "https://example.com/x.patch",
  );
  assert.deepEqual(parseNotices({ components: [sample({ version: null })] })[0]?.version, null);
});
