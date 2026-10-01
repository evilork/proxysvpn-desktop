// tests/releaseSums.test.ts
//
// node --test tests/releaseSums.test.ts
//
// scripts/release-sums.sh is what scripts/publish-release.sh uses to find the
// installers in the unzipped CI artifacts and to check them against the CI
// run's own SHA256SUMS files. The release script itself needs gh, a clean
// main and a prompt, so its file logic is tested here on its own, in bash,
// against scratch folders.
//
// Skipped on Windows: the release is cut on a Mac, and Git Bash would get
// Windows paths from os.tmpdir().

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const LIB = resolve(dirname(fileURLToPath(import.meta.url)), "../scripts/release-sums.sh");
const SKIP = process.platform === "win32";

const DMG = "ProxysVPN_9.9.9_aarch64.dmg";
const EXE = "ProxysVPN_9.9.9_x64-setup.exe";
const DEB = "ProxysVPN_9.9.9_amd64.deb";
const H1 = "a".repeat(64);
const H2 = "b".repeat(64);
const H3 = "c".repeat(64);

interface Run {
  status: number | null;
  stdout: string;
  stderr: string;
}

/** Source the library and run `script` in bash with `set -euo pipefail`. */
function bash(script: string, args: string[] = []): Run {
  const r = spawnSync("bash", ["-c", `set -euo pipefail; source "$0"; ${script}`, LIB, ...args], {
    encoding: "utf8",
  });
  return { status: r.status, stdout: r.stdout, stderr: r.stderr };
}

function scratch(): string {
  return mkdtempSync(join(tmpdir(), "pvpn-release-sums-"));
}

function put(root: string, rel: string, content = ""): string {
  const path = join(root, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  return path;
}

test("an installer is found one folder down, as an old nested artifact unzips", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const path = put(dir, `dmg/${DMG}`, "x");
    const r = bash(`release_find_asset "$1" "$2"`, [dir, DMG]);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout.trim(), path);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("an installer is found at the top of a flat artifact", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const path = put(dir, EXE, "x");
    const r = bash(`release_find_asset "$1" "$2"`, [dir, EXE]);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout.trim(), path);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a missing or doubled installer is an error, never a guess", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    let r = bash(`release_find_asset "$1" "$2"`, [dir, DEB]);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /missing/);

    put(dir, DEB, "one");
    put(dir, `deb/${DEB}`, "two");
    r = bash(`release_find_asset "$1" "$2"`, [dir, DEB]);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /twice/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CI sums files are found at any depth, and our own SHA256SUMS.txt is not one", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const a = put(dir, "SHA256SUMS-proxysvpn-macos-aarch64.txt");
    const b = put(dir, "nsis/SHA256SUMS-proxysvpn-windows-x86_64.txt");
    put(dir, "SHA256SUMS.txt");
    const r = bash(`release_find_ci_sums "$1"`, [dir]);
    assert.equal(r.status, 0, r.stderr);
    assert.deepEqual(r.stdout.trim().split("\n").sort(), [a, b].sort());
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the Windows runner's binary-mode '*name' line is read", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const sums = put(dir, "SHA256SUMS-proxysvpn-windows-x86_64.txt", `${H2} *${EXE}\r\n`);
    const r = bash(`release_ci_sum_for "$1" "$2"`, [EXE, sums]);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.stdout.trim(), H2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("matching installers pass the cross-check", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const mac = put(dir, "SHA256SUMS-proxysvpn-macos-aarch64.txt", `${H1}  ${DMG}\n`);
    const win = put(dir, "SHA256SUMS-proxysvpn-windows-x86_64.txt", `${H2} *${EXE}\n`);
    const lin = put(dir, "SHA256SUMS-proxysvpn-linux-x86_64.txt", `${H3}  ${DEB}\n`);
    const ours = put(dir, "SHA256SUMS.txt", `${H1}  ${DMG}\n${H2}  ${EXE}\n${H3}  ${DEB}\n`);
    const r = bash(`release_check_against_ci "$@"`, [ours, mac, win, lin]);
    assert.equal(r.status, 0, r.stderr);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a different sum fails the cross-check", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const win = put(dir, "SHA256SUMS-proxysvpn-windows-x86_64.txt", `${H2} *${EXE}\n`);
    const ours = put(dir, "SHA256SUMS.txt", `${H3}  ${EXE}\n`);
    const r = bash(`release_check_against_ci "$@"`, [ours, win]);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /differs/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("an installer the CI sums do not mention fails instead of passing quietly", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const mac = put(dir, "SHA256SUMS-proxysvpn-macos-aarch64.txt", `${H1}  ${DMG}\n`);
    const ours = put(dir, "SHA256SUMS.txt", `${H1}  ${DMG}\n${H2}  ${EXE}\n`);
    const r = bash(`release_check_against_ci "$@"`, [ours, mac]);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, new RegExp(`${EXE.replace(/\./g, "\\.")} has no line`));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("two CI lines that disagree about one installer fail", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const a = put(dir, "SHA256SUMS-a.txt", `${H1}  ${DMG}\n`);
    const b = put(dir, "dmg/SHA256SUMS-b.txt", `${H2}  ${DMG}\n`);
    const r = bash(`release_ci_sum_for "$1" "$2" "$3"`, [DMG, a, b]);
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /disagree/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a name is matched whole, not as a suffix of a longer one", { skip: SKIP }, () => {
  const dir = scratch();
  try {
    const sums = put(dir, "SHA256SUMS-x.txt", `${H1}  old-${DMG}\n`);
    const r = bash(`release_ci_sum_for "$1" "$2"`, [DMG, sums]);
    assert.notEqual(r.status, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
