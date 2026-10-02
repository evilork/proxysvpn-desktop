// tests/installerHooks.test.ts
//
// node --test tests/installerHooks.test.ts
//
// The Windows installer hooks cannot be compiled on a Mac (no makensis here,
// and installers are built in CI only), so this pins what they must keep
// doing: be wired into the Windows config, stop our own app and its tree
// before files are copied or removed, never kill an engine by its name
// (another VPN client's xray.exe or tun2socks.exe), never fail the install
// when the app is not running, and stop the arm64 installer — only that one
// — on a PC that is not ARM64.

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

/** `text` without comment lines and outside every `!macro` … `!macroend`. */
function topLevel(text: string): string {
  const out: string[] = [];
  let inMacro = false;
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.startsWith(";")) continue;
    if (trimmed.startsWith("!macro ")) inMacro = true;
    else if (trimmed === "!macroend") inMacro = false;
    else if (!inMacro) out.push(line);
  }
  return out.join("\n");
}

test("the CPU check runs first before install, and never before uninstall", () => {
  const text = hooks();
  const pre = macro(text, "NSIS_HOOK_PREINSTALL");
  const refuse = pre.indexOf("!insertmacro PVPN_REFUSE_FOREIGN_CPU");
  const stop = pre.indexOf("!insertmacro PVPN_STOP_RUNNING_APP");
  assert.ok(refuse >= 0, "the preinstall hook checks the CPU");
  assert.ok(refuse < stop, "before a running ProxysVPN is stopped");
  assert.ok(!macro(text, "NSIS_HOOK_PREUNINSTALL").includes("PVPN_REFUSE_FOREIGN_CPU"));
});

test("only the arm64 installer refuses, and only a PC that is not ARM64", () => {
  const text = hooks();
  const body = macro(text, "PVPN_REFUSE_FOREIGN_CPU");
  const lines = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("!macro"));
  // Everything is inside one compile-time branch on the template's ARCH, so
  // the x64 installer compiles an empty macro.
  assert.equal(lines[0], '!if "${ARCH}" == "arm64"');
  assert.equal(lines[lines.length - 1], "!endif");
  assert.equal(body.match(/!if\b/g)?.length, 1);
  assert.equal(body.match(/!endif\b/g)?.length, 1);
  // ARCH is defined by installer.nsi after this file is included: it may
  // only be read inside a macro, which is expanded where it is inserted.
  assert.ok(!topLevel(text).includes("${ARCH}"), "no top-level use of ARCH");
  // The OS's own CPU, from x64.nsh, not the installer's (always x86).
  assert.match(body, /\$\{IfNot\} \$\{IsNativeARM64\}/);
  assert.ok(!/RunningX64|IsWow64/.test(body), "not a test of the installer's own bitness");
});

test("the refusal says what to download, in English and Russian, and stops", () => {
  const body = macro(hooks(), "PVPN_REFUSE_FOREIGN_CPU");
  const box = body.match(/MessageBox MB_ICONSTOP\|MB_OK "([^"]*)" \/SD IDOK/);
  assert.ok(box, "one stop box that a silent install answers by itself");
  const message = box[1];
  assert.match(message, /Windows on ARM \(arm64\)/);
  assert.match(message, /Windows на ARM \(arm64\)/);
  assert.equal(message.match(/ProxysVPN_\$\{VERSION\}_x64-setup\.exe/g)?.length, 2, "the x64 installer, named in both");
  assert.match(body, /\bAbort\b/);
  // Only an empty folder SetOutPath made is removed, never an installation.
  assert.match(body, /RMDir "\$INSTDIR"/);
  assert.ok(!/RMDir \/r/.test(body), "never recursive");
  assert.ok(!/Delete\b|taskkill/.test(body), "nothing else is touched");
});
