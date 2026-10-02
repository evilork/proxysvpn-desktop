// tests/checkPlatform.test.ts
//
// node --test tests/checkPlatform.test.ts
//
// scripts/check-platform.sh cross-checks the platform layer for every target
// in its list. A Mac that lacks one of them (aarch64-pc-windows-msvc arrived
// with the arm64 installer) used to stop the whole script before checking
// the others; now it skips that one with a warning that names the rustup
// command, while CI, which installs every target, still treats a missing one
// as an error. Run here with stand-ins for cargo and rustup on PATH, so the
// test takes milliseconds and checks only the script's own logic.
//
// Skipped on Windows: the script is run on a Mac and on the macOS CI runner.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = resolve(dirname(fileURLToPath(import.meta.url)), "../scripts/check-platform.sh");
const SKIP = process.platform === "win32";

const ALL = ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc", "x86_64-unknown-linux-gnu"];

interface Run {
  status: number | null;
  stdout: string;
  stderr: string;
  /** Every cargo command line the script ran, one per entry. */
  cargo: string[];
}

/** Run the script with `installed` as rustup's answer and CI set to `ci`. */
function run(installed: string[], ci: string | undefined): Run {
  const bin = mkdtempSync(join(tmpdir(), "pvpn-check-platform-"));
  try {
    const log = join(bin, "cargo.log");
    writeFileSync(log, "");
    writeFileSync(join(bin, "cargo"), `#!/bin/sh\nprintf '%s\\n' "$*" >> "${log}"\n`);
    writeFileSync(
      join(bin, "rustup"),
      `#!/bin/sh\n[ "$*" = "target list --installed" ] || exit 2\nprintf '%s\\n' ${installed.map((t) => `'${t}'`).join(" ")}\n`,
    );
    chmodSync(join(bin, "cargo"), 0o755);
    chmodSync(join(bin, "rustup"), 0o755);
    const env: NodeJS.ProcessEnv = { ...process.env, PATH: `${bin}:${process.env.PATH ?? ""}` };
    delete env.CI;
    if (ci !== undefined) env.CI = ci;
    const r = spawnSync("bash", [SCRIPT], { encoding: "utf8", env });
    return {
      status: r.status,
      stdout: r.stdout,
      stderr: r.stderr,
      cargo: readFileSync(log, "utf8").split("\n").filter((line) => line !== ""),
    };
  } finally {
    rmSync(bin, { recursive: true, force: true });
  }
}

function crossChecked(r: Run): string[] {
  return r.cargo
    .filter((line) => line.startsWith("clippy -p pvpn-platform"))
    .map((line) => line.match(/--target (\S+)/)?.[1] ?? "?");
}

test("every installed target is cross-checked", { skip: SKIP }, () => {
  const r = run(ALL, undefined);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(crossChecked(r), ALL);
  assert.match(r.stdout, /All checks that are possible without a Windows or Linux machine passed\./);
  assert.ok(!/warning/.test(r.stderr), r.stderr);
});

test("a missing target is skipped locally, with the rustup command, and the others still run", { skip: SKIP }, () => {
  const r = run(["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"], undefined);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(crossChecked(r), ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]);
  assert.match(r.stderr, /warning: target aarch64-pc-windows-msvc is not installed/);
  assert.match(r.stderr, /rustup target add aarch64-pc-windows-msvc/);
  assert.match(r.stderr, /NOT cross-checked \(target not installed\): aarch64-pc-windows-msvc\./);
  assert.ok(!/All checks that are possible/.test(r.stdout), "it does not claim everything passed");
  // The host checks before it ran as before.
  assert.ok(r.cargo.some((line) => line.startsWith("clippy --workspace")), r.cargo.join("\n"));
});

test("no rustup answer at all skips every cross target locally", { skip: SKIP }, () => {
  const r = run([], undefined);
  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(crossChecked(r), []);
  for (const target of ALL) assert.match(r.stderr, new RegExp(`rustup target add ${target}`));
});

test("in CI a missing target is an error, before any cross-check", { skip: SKIP }, () => {
  for (const ci of ["true", "1"]) {
    const r = run(["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"], ci);
    assert.equal(r.status, 1, `CI=${ci}: ${r.stderr}`);
    assert.match(r.stderr, /error: target aarch64-pc-windows-msvc is not installed/);
    assert.deepEqual(crossChecked(r), []);
  }
  // CI=false is a local run.
  assert.equal(run(["x86_64-pc-windows-msvc"], "false").status, 0);
});
