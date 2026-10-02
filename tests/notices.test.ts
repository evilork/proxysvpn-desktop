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
  // The desktop apps ship the pinned unpatched release: no patch.
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

test("no GPL component ships in the iOS build", () => {
  for (const component of committed().filter((c) => c.platforms.includes("ios"))) {
    assert.ok(!component.licenseIds.some((id) => id.startsWith("GPL")), component.name);
  }
});

// The stock desktop Xray-core and Hysteria link sagernet/sing and friends,
// which are GPL-3.0-or-later; the notices must say so and carry the text.
test("every desktop build names the GPL code inside its engines", () => {
  for (const platform of ["macos", "windows", "linux"] as const) {
    const gpl = groupByLicense(committed(), platform).find((g) => g.license === "GPL-3.0-or-later");
    const names = gpl?.components.map((c) => c.name) ?? [];
    assert.ok(names.some((n) => n.startsWith("github.com/sagernet/sing ") && n.includes("Xray-core")), `${platform}: ${names}`);
    assert.ok(names.some((n) => n.includes("Hysteria")), `${platform}: ${names}`);
  }
  assert.ok(existsSync(new URL("src/assets/licenses/GPL-3.0-or-later.txt", ROOT)));
});

test("tun2socks is MIT, as upstream says", () => {
  const tun = committed().find((c) => c.name === "tun2socks");
  assert.equal(tun?.license, "MIT");
});

// Windows and Linux link crates the Apple builds do not; their screens must
// list them, and must not list the iOS-only engine.
test("Windows and Linux list their own crates, not the iOS engine", () => {
  const names = (platform: string) =>
    groupByLicense(committed(), platform).flatMap((g) => g.components.map((c) => c.name));
  const windows = names("windows");
  assert.ok(windows.includes("windows"), "the windows crate");
  assert.ok(windows.includes("webview2-com"));
  assert.ok(windows.includes("tauri-plugin-single-instance"));
  assert.ok(!windows.includes("libXray"));
  const linux = names("linux");
  assert.ok(linux.includes("gtk"));
  assert.ok(linux.includes("webkit2gtk"));
  assert.ok(!linux.includes("libXray"));
  assert.ok(!linux.includes("webview2-com"));
});

// The AppImage carries WebKitGTK, GTK and the rest of what they link in its
// usr/lib; the LGPL asks that they be named with their licence and source.
test("the Linux screen names the LGPL libraries the AppImage carries, and only Linux", () => {
  const all = committed();
  const libraries = all.filter((c) => c.kind === "library");
  const names = libraries.map((c) => c.name);
  for (const expected of ["WebKitGTK", "GTK 3", "GLib", "GDK-Pixbuf", "Pango", "cairo", "libsoup", "GStreamer", "librsvg"]) {
    assert.ok(names.some((n) => n.startsWith(expected)), `${expected} in ${names.join(", ")}`);
  }
  for (const library of libraries) {
    assert.deepEqual(library.platforms, ["linux"], library.name);
    assert.ok(library.licenseIds.some((id) => id.startsWith("LGPL-")), `${library.name}: ${library.license}`);
    assert.match(library.source, /^https:\/\//, library.name);
  }
  const webkit = libraries.find((c) => c.name.startsWith("WebKitGTK"));
  assert.equal(webkit?.license, "BSD-2-Clause AND LGPL-2.1-only");
  assert.equal(libraries.find((c) => c.name.startsWith("cairo"))?.license, "LGPL-2.1-only OR MPL-1.1");
  // No other build's screen lists them.
  for (const platform of ["ios", "macos", "windows"]) {
    const listed = groupByLicense(all, platform).flatMap((g) => g.components);
    assert.ok(!listed.some((c) => c.kind === "library"), platform);
  }
  assert.ok(groupByLicense(all, "linux").some((g) => g.components.some((c) => c.kind === "library")));
  assert.deepEqual(parseNotices({ components: [sample({ kind: "library" })] })[0]?.kind, "library");
});

test("an iOS screen leaves out what only the desktop apps carry", () => {
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
