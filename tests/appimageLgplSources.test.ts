// tests/appimageLgplSources.test.ts
//
// node --test tests/appimageLgplSources.test.ts
//
// scripts/appimage-lgpl-sources.sh writes, for the Linux AppImage, which
// Ubuntu package and version each library in its usr/lib came from, and with
// --check refuses an image whose libraries or carried list differ. It runs in
// CI on the machine that built the image, with dpkg and a real AppImage; here
// both are stand-ins on PATH and in a scratch folder: an "AppImage" that
// extracts a few files, and a dpkg / dpkg-query that answer for them the way
// the real ones print.
//
// Skipped on Windows, like the other shell-script tests.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = resolve(dirname(fileURLToPath(import.meta.url)), "../scripts/appimage-lgpl-sources.sh");
const SKIP = process.platform === "win32";
const INSIDE = "usr/share/doc/proxysvpn-desktop/LGPL-SOURCES.txt";
const IMAGE = "ProxysVPN_9.9.9_amd64.AppImage";

function executable(path: string, text: string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
  chmodSync(path, 0o755);
}

/**
 * A scratch folder with the stand-ins: `bin/dpkg`, `bin/dpkg-query` and the
 * image, which extracts `usr/lib` with the files named in $LIBS and, when
 * $CARRIED names a file, that file as the list inside it.
 */
function bench(): string {
  const dir = mkdtempSync(join(tmpdir(), "pvpn-lgpl-sources-"));
  executable(
    join(dir, IMAGE),
    `#!/bin/sh
[ "$1" = --appimage-extract ] || exit 2
for rel in $LIBS; do
    mkdir -p "squashfs-root/$(dirname "$rel")"
    : > "squashfs-root/$rel"
done
if [ -n "\${CARRIED:-}" ]; then
    mkdir -p squashfs-root/usr/share/doc/proxysvpn-desktop
    cp "$CARRIED" squashfs-root/${INSIDE}
fi
`,
  );
  // dpkg -S "*/<name>": owners as dpkg prints them, a diversion line, and a
  // longer name that a glob would not match but a careless reader would.
  executable(
    join(dir, "bin/dpkg"),
    `#!/bin/sh
[ "$1" = -S ] || exit 2
case $2 in
"*/libgtk-3.so.0")
    echo "diversion by fake from: /usr/lib/x86_64-linux-gnu/libgtk-3.so.0"
    echo "libgtk-3-0:amd64: /usr/lib/x86_64-linux-gnu/libgtk-3.so.0"
    echo "libgtk-3-dev:amd64: /usr/lib/x86_64-linux-gnu/libgtk-3.so.0.debug"
    ;;
"*/libpixbufloader-png.so")
    echo "libgdk-pixbuf-2.0-0:amd64: /usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-png.so"
    ;;
"*/libglib-2.0.so.0")
    echo "libglib2.0-0:amd64, libglib2.0-0:i386: /usr/lib/x86_64-linux-gnu/libglib-2.0.so.0"
    ;;
*)
    echo "dpkg-query: no path found matching pattern $2" >&2
    exit 1
    ;;
esac
`,
  );
  executable(
    join(dir, "bin/dpkg-query"),
    `#!/bin/sh
[ "$1" = -W ] || exit 2
case $2 in -f=*) ;; *) exit 2 ;; esac
shift 2
for pkg in "$@"; do
    case $pkg in
    libgtk-3-0) echo "libgtk-3-0=3.24.33-1ubuntu2.2 gtk+3.0=3.24.33-1ubuntu2.2" ;;
    libgdk-pixbuf-2.0-0) echo "libgdk-pixbuf-2.0-0=2.42.8+dfsg-1ubuntu0.3 gdk-pixbuf=2.42.8+dfsg-1ubuntu0.3" ;;
    libglib2.0-0) echo "libglib2.0-0=2.72.4-0ubuntu2.3 glib2.0=2.72.4-0ubuntu2.3" ;;
    *) echo "dpkg-query: no packages found matching $pkg" >&2; exit 1 ;;
    esac
done
`,
  );
  return dir;
}

const LIBS = [
  "usr/lib/libgtk-3.so.0",
  "usr/lib/libglib-2.0.so.0",
  "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-png.so",
  "usr/lib/libproxysvpn-own.so",
].join(" ");

function run(dir: string, args: string[], env: Record<string, string> = {}) {
  const r = spawnSync("bash", [SCRIPT, ...args], {
    cwd: dir,
    encoding: "utf8",
    env: { ...process.env, PATH: `${join(dir, "bin")}:${process.env.PATH ?? ""}`, LIBS, ...env },
  });
  return { status: r.status, stdout: r.stdout, stderr: r.stderr };
}

test("the list names each library's package and source package at their exact versions", { skip: SKIP }, () => {
  const dir = bench();
  try {
    const list = join(dir, "out/LGPL-SOURCES.txt");
    const r = run(dir, [join(dir, IMAGE), list]);
    assert.equal(r.status, 0, r.stderr);
    const text = readFileSync(list, "utf8");

    const section = (name: string): string[] => {
      const lines = text.split("\n");
      const start = lines.indexOf(`[${name}]`);
      assert.ok(start >= 0, `[${name}] in\n${text}`);
      const out: string[] = [];
      for (const line of lines.slice(start + 1)) {
        if (line === "" || line.startsWith("[")) break;
        if (!line.startsWith("#")) out.push(line);
      }
      return out;
    };
    assert.deepEqual(section("packages"), [
      "libgdk-pixbuf-2.0-0=2.42.8+dfsg-1ubuntu0.3 gdk-pixbuf=2.42.8+dfsg-1ubuntu0.3",
      "libglib2.0-0=2.72.4-0ubuntu2.3 glib2.0=2.72.4-0ubuntu2.3",
      "libgtk-3-0=3.24.33-1ubuntu2.2 gtk+3.0=3.24.33-1ubuntu2.2",
    ]);
    // The diversion line and the longer file name are not owners; two
    // architectures of one package are one package.
    assert.deepEqual(section("libraries"), [
      "usr/lib/gdk-pixbuf-2.0/2.10.0/loaders/libpixbufloader-png.so libgdk-pixbuf-2.0-0",
      "usr/lib/libglib-2.0.so.0 libglib2.0-0",
      "usr/lib/libgtk-3.so.0 libgtk-3-0",
    ]);
    assert.deepEqual(section("not from a package"), ["usr/lib/libproxysvpn-own.so"]);

    // Says what the binaries are and where their source is, honestly.
    assert.match(text, new RegExp(`^# ${IMAGE.replaceAll(".", "\\.")}: `, "m"));
    assert.match(text, /rewrote its RUNPATH and may have stripped it/);
    assert.match(text, /source\s+# package of the version given, unmodified/);
    assert.match(text, /apt-get source <source package>=<version>/);
    assert.ok(text.includes(INSIDE) && text.includes("LGPL-SOURCES-linux-appimage.txt"));
    assert.ok(!/\d{4}-\d{2}-\d{2}T|\d{2}:\d{2}:\d{2}/.test(text), "no time in it: --check compares bytes");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--check passes only for an image that carries that very list and those libraries", { skip: SKIP }, () => {
  const dir = bench();
  try {
    const list = join(dir, "LGPL-SOURCES.txt");
    assert.equal(run(dir, [join(dir, IMAGE), list]).status, 0);

    const ok = run(dir, ["--check", join(dir, IMAGE), list], { CARRIED: list });
    assert.equal(ok.status, 0, ok.stderr);

    // The image from the first build, before the list was put into it.
    const bare = run(dir, ["--check", join(dir, IMAGE), list]);
    assert.notEqual(bare.status, 0);
    assert.match(bare.stderr, /does not carry/);

    // An image whose libraries are not the ones listed.
    const other = run(dir, ["--check", join(dir, IMAGE), list], {
      CARRIED: list,
      LIBS: `${LIBS} usr/lib/libgtk-3.so.0.extra`,
    });
    assert.notEqual(other.status, 0);
    assert.match(other.stderr, /are not the ones/);

    const missing = run(dir, ["--check", join(dir, IMAGE), join(dir, "nowhere.txt")], { CARRIED: list });
    assert.notEqual(missing.status, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("an image without a library from a package, or no image at all, is an error", { skip: SKIP }, () => {
  const dir = bench();
  try {
    const list = join(dir, "LGPL-SOURCES.txt");
    const unowned = run(dir, [join(dir, IMAGE), list], { LIBS: "usr/lib/libproxysvpn-own.so" });
    assert.notEqual(unowned.status, 0);
    assert.match(unowned.stderr, /belongs to an installed package/);

    const none = run(dir, [join(dir, "missing.AppImage"), list]);
    assert.notEqual(none.status, 0);
    assert.match(none.stderr, /no AppImage/);

    const usage = run(dir, [join(dir, IMAGE)]);
    assert.notEqual(usage.status, 0);
    assert.match(usage.stderr, /usage/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// The committed placeholder ships in builds that are not CI's, and says so.
test("a local build carries a note instead of a list", () => {
  const note = readFileSync(resolve(dirname(SCRIPT), "../src-tauri/linux/LGPL-SOURCES.txt"), "utf8");
  assert.match(note, /Not recorded for this build/);
  assert.ok(note.includes("scripts/appimage-lgpl-sources.sh"));
  assert.ok(!note.includes("[packages]"), "no list pretends to be one");
  const conf = JSON.parse(readFileSync(resolve(dirname(SCRIPT), "../src-tauri/tauri.linux.conf.json"), "utf8"));
  assert.equal(conf.bundle.linux.appimage.files[`/${INSIDE}`], "linux/LGPL-SOURCES.txt");
});
