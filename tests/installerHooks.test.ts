// tests/installerHooks.test.ts
//
// node --test tests/installerHooks.test.ts
//
// The Windows installer hooks cannot be compiled on a Mac (no makensis here,
// and installers are built in CI only), so this pins what they must keep
// doing: be wired into the Windows config, stop our own app and its tree
// before files are copied or removed, never kill an engine by its name
// (another VPN client's xray.exe or tun2socks.exe), and never fail the
// install when the app is not running.

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";

const TAURI_DIR = new URL("../src-tauri/", import.meta.url);

function windowsConfig(): { bundle: { windows: { nsis: { installerHooks?: string } } } } {
  return JSON.parse(readFileSync(new URL("tauri.windows.conf.json", TAURI_DIR), "utf8"));
}

function hooks(): string {
  const path = windowsConfig().bundle.windows.nsis.installerHooks;
  assert.ok(path, "bundle.windows.nsis.installerHooks is set");
  const file = new URL(path, TAURI_DIR);
  assert.ok(existsSync(file), `${path} exists next to tauri.windows.conf.json`);
  // A Windows checkout may carry CRLF; the macros are read line by line.
  return readFileSync(file, "utf8").replace(/\r\n/g, "\n");
}

/** The body of one `!macro NAME` … `!macroend`, without comment lines. */
function macro(text: string, name: string): string {
  const start = text.indexOf(`!macro ${name}\n`);
  assert.ok(start >= 0, `${name} is defined`);
  const end = text.indexOf("!macroend", start);
  return text
    .slice(start, end)
    .split("\n")
    .filter((line) => !line.trim().startsWith(";"))
    .join("\n");
}

test("both hooks Tauri runs before copying and before removing files stop the app", () => {
  const text = hooks();
  for (const name of ["NSIS_HOOK_PREINSTALL", "NSIS_HOOK_PREUNINSTALL"]) {
    assert.match(macro(text, name), /!insertmacro PVPN_STOP_RUNNING_APP/);
  }
});

test("only our own process tree is killed, by the template's own binary name", () => {
  const body = macro(hooks(), "PVPN_STOP_RUNNING_APP");
  assert.match(body, /taskkill\.exe" \/F \/T \/IM "\$\{MAINBINARYNAME\}\.exe"/);
  for (const engine of ["xray", "tun2socks", "hysteria"]) {
    assert.ok(!body.includes(`${engine}.exe`), `${engine}.exe is never killed by name`);
  }
});

test("an app that is not running never fails the install", () => {
  const body = macro(hooks(), "PVPN_STOP_RUNNING_APP");
  assert.ok(!/\bAbort\b/.test(body), "the hook never aborts");
  assert.ok(!/\bQuit\b/.test(body), "the hook never quits the installer");
  assert.ok(!/MessageBox/.test(body), "the hook asks nothing");
  // The wait for the process to go is bounded.
  assert.match(body, /\$\{If\} \$1 > \d+\s+\$\{ExitDo\}/);
});
