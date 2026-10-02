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
// — on a PC that is not ARM64, before the installer does anything at all.
//
// Where that check can run depends on Tauri's own installer.nsi, which is not
// in this repository: it is compiled into the Tauri CLI. The last tests read
// it out of the CLI binary npm installed, so a Tauri upgrade that moves the
// hooks' include, takes MUI_CUSTOMFUNCTION_GUIINIT or changes the lines the
// hooks read fails here instead of shipping an installer that refuses too late.

import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

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

/** The `!if "${PVPN_ARCH}" == "arm64"` … `!endif` block, without comments. */
function arm64Block(text: string): string {
  const open = '!if "${PVPN_ARCH}" == "arm64"';
  const lines = text.split("\n").filter((line) => !line.trim().startsWith(";"));
  const first = lines.findIndex((line) => line.trim() === open);
  assert.ok(first >= 0, "the arm64-only block is there");
  let depth = 0;
  for (let i = first; i < lines.length; i += 1) {
    const trimmed = lines[i].trim();
    if (/^!if(n?def)?\b/.test(trimmed)) depth += 1;
    if (trimmed === "!endif") depth -= 1;
    if (depth === 0) return lines.slice(first, i + 1).join("\n");
  }
  assert.fail("the arm64-only block is closed");
}

/** The body of `Function NAME` … `FunctionEnd`. */
function nsisFunction(text: string, name: string): string {
  const start = text.indexOf(`Function ${name}\n`);
  assert.ok(start >= 0, `Function ${name} is defined`);
  return text.slice(start, text.indexOf("FunctionEnd", start));
}

/** The `!searchparse /file "installer.nsi"` lines: [start, symbol, end]. */
function searchparses(text: string): [string, string, string][] {
  return [...topLevel(text).matchAll(/^!searchparse \/file "installer\.nsi" `([^`]*)` (\w+) `([^`]*)`$/gm)].map(
    (m) => [m[1], m[2], m[3]],
  );
}

test("the CPU check reads the template's ARCH and VERSION, and is compiled into the arm64 installer only", () => {
  const text = hooks();
  const parsed = searchparses(text);
  assert.deepEqual(
    parsed.map(([, symbol]) => symbol),
    ["PVPN_ARCH", "PVPN_VERSION"],
  );
  assert.deepEqual(parsed[0], ['!define ARCH "', "PVPN_ARCH", '"']);
  assert.deepEqual(parsed[1], ['!define VERSION "', "PVPN_VERSION", '"']);
  // The template defines ARCH and VERSION after including this file: at the
  // top level they would still be unexpanded. Only macros, expanded later,
  // may read them.
  const top = topLevel(text);
  assert.ok(!top.includes("${ARCH}") && !top.includes("${VERSION}"), "no top-level use of the template's defines");

  const block = arm64Block(text);
  assert.match(block, /^\s*!define MUI_CUSTOMFUNCTION_GUIINIT PvpnRefuseForeignCpu$/m);
  assert.match(block, /^\s*Function PvpnRefuseForeignCpu$/m);
  // The silent install's first section, which runs only the same function.
  const section = block.match(/^\s*Section "(-[^"]+)"\n\s*Call PvpnRefuseForeignCpu\n\s*SectionEnd$/m);
  assert.ok(section, "a hidden section that calls the check and nothing else");
  // The x64 installer compiles none of it: nothing of the check outside.
  const outside = text.replace(block, "");
  assert.ok(!/PvpnRefuseForeignCpu|IsNativeARM64|MUI_CUSTOMFUNCTION_GUIINIT PvpnRefuseForeignCpu/.test(topLevel(outside)));
});

test("the CPU check runs before any page and any section, never inside the install or uninstall hook", () => {
  const text = hooks();
  for (const name of ["NSIS_HOOK_PREINSTALL", "NSIS_HOOK_PREUNINSTALL"]) {
    const body = macro(text, name);
    assert.ok(!/IsNativeARM64|PvpnRefuseForeignCpu|MessageBox/.test(body), `${name} refuses nothing`);
  }
  // What was read from installer.nsi is checked against the template's own
  // defines where they exist, before anything else the hook does.
  const pre = macro(text, "NSIS_HOOK_PREINSTALL");
  const check = pre.indexOf("!insertmacro PVPN_CHECK_TEMPLATE_DEFINES");
  assert.ok(check >= 0 && check < pre.indexOf("!insertmacro PVPN_STOP_RUNNING_APP"));
  const defines = macro(text, "PVPN_CHECK_TEMPLATE_DEFINES");
  assert.match(defines, /!if "\$\{PVPN_ARCH\}" != "\$\{ARCH\}"\s+!error /);
  assert.match(defines, /!if "\$\{PVPN_VERSION\}" != "\$\{VERSION\}"\s+!error /);
});

test("the refusal says what to download, in English and Russian, and ends the installer touching nothing", () => {
  const body = nsisFunction(arm64Block(hooks()), "PvpnRefuseForeignCpu");
  // The OS's own CPU, from x64.nsh, not the installer's (always x86).
  assert.match(body, /\$\{IfNot\} \$\{IsNativeARM64\}/);
  assert.ok(!/RunningX64|IsWow64/.test(body), "not a test of the installer's own bitness");
  const box = body.match(/MessageBox MB_ICONSTOP\|MB_OK "([^"]*)" \/SD IDOK/);
  assert.ok(box, "one stop box that a silent install answers by itself");
  const message = box[1];
  assert.match(message, /Windows on ARM \(arm64\)/);
  assert.match(message, /Windows на ARM \(arm64\)/);
  assert.equal(message.match(/ProxysVPN_\$\{PVPN_VERSION\}_x64-setup\.exe/g)?.length, 2, "the x64 installer, named in both");
  // A silent install says it on its console instead.
  assert.match(body, /\$\{If\} \$\{Silent\}[\s\S]*AttachConsole[\s\S]*FileWrite \$0 "[^"]*_x64-setup\.exe/);
  // Ended at once, as "aborted by script", from .onGUIInit or a section.
  assert.match(body, /SetErrorLevel 2\s+Quit/);
  assert.ok(!/\bAbort\b/.test(body), "Abort would not end the installer from a section");
  assert.ok(!/Delete\b|RMDir|SetOutPath|\bFile\b|taskkill|Exec/.test(body), "nothing is touched");
});

// ── Tauri's own template ─────────────────────────────────────────────────

/** The Tauri CLI's native binary npm installed for this machine, if any. */
function tauriCliBinary(): string | null {
  const scope = new URL("../node_modules/@tauri-apps/", import.meta.url);
  if (!existsSync(scope)) return null;
  for (const dir of readdirSync(scope).filter((name) => name.startsWith("cli-"))) {
    const file = readdirSync(new URL(`${dir}/`, scope)).find((name) => name.endsWith(".node"));
    if (file) return fileURLToPath(new URL(`${dir}/${file}`, scope));
  }
  return null;
}

/**
 * installer.nsi as the CLI embeds it (tauri-bundler's `include_str!`), up to
 * the first NUL: the template and the .nsh files the bundler stores after it.
 * CRLF in a binary built from a Windows checkout reads as LF.
 */
function tauriTemplate(binary: string): string {
  const bytes = readFileSync(binary);
  for (let at = bytes.indexOf("Unicode true"); at >= 0; at = bytes.indexOf("Unicode true", at + 1)) {
    const head = bytes.subarray(at, at + 64).toString("latin1");
    if (!/^Unicode true\r?\nManifestDPIAware true/.test(head)) continue;
    const nul = bytes.indexOf(0, at);
    const end = nul > at ? Math.min(nul, at + 400_000) : at + 400_000;
    return bytes.subarray(at, end).toString("utf8").replace(/\r\n/g, "\n");
  }
  assert.fail(`no installer.nsi template in ${binary}`);
}

const CLI = tauriCliBinary();
const NO_CLI = CLI ? false : "no Tauri CLI binary under node_modules/@tauri-apps (run npm ci)";

/** What `!searchparse /file` defines: the first line holding both ends. */
function searchparse(file: string, start: string, end: string): string | undefined {
  for (const line of file.split("\n")) {
    const from = line.indexOf(start);
    if (from < 0) continue;
    const rest = line.slice(from + start.length);
    const to = rest.indexOf(end);
    if (to >= 0) return rest.slice(0, to);
  }
  return undefined;
}

test("Tauri's template includes the hooks before every page, section and .onGUIInit", { skip: NO_CLI }, () => {
  const template = tauriTemplate(CLI as string);
  const at = (needle: string): number => {
    const index = template.indexOf(needle);
    assert.ok(index >= 0, `the template has ${JSON.stringify(needle)}`);
    return index;
  };
  const include = at('!include "{{installer_hooks}}"');
  // What the hooks use at their own top level is there already.
  assert.ok(at("!include MUI2.nsh") < include && at("!include x64.nsh") < include);
  // Before anything that can show, remove or install something.
  for (const later of [
    "!insertmacro MUI_PAGE_WELCOME",
    "Page custom PageReinstall PageLeaveReinstall",
    "!insertmacro MUI_LANGUAGE", // MUI_INSERT: where .onGUIInit is generated
    "Function .onInit",
    "Section EarlyChecks",
    "Section WebView2",
    "Section Install",
  ]) {
    assert.ok(include < at(later), `the hooks come before ${later}`);
  }
  // Why the check cannot live in NSIS_HOOK_PREINSTALL: that runs in Section
  // Install, after the reinstall page (which may run the old uninstaller)
  // and after Section WebView2.
  const preinstall = at("!insertmacro NSIS_HOOK_PREINSTALL");
  assert.ok(at("Section Install") < preinstall);
  assert.ok(at("Page custom PageReinstall") < preinstall && at("Section WebView2") < preinstall);
  // The template leaves the GUI-init hook to us; defining it twice would not compile.
  assert.ok(!template.includes("MUI_CUSTOMFUNCTION_GUIINIT"), "the template does not take MUI_CUSTOMFUNCTION_GUIINIT");
});

test("the hooks read ARCH and VERSION from the template as tauri-bundler renders it", { skip: NO_CLI }, () => {
  const template = tauriTemplate(CLI as string);
  const parsed = searchparses(hooks());
  assert.equal(parsed.length, 2);
  for (const arch of ["arm64", "x64"]) {
    const rendered = template.replaceAll("{{arch}}", arch).replaceAll("{{version}}", "9.9.9");
    const [archLine, versionLine] = parsed;
    assert.equal(searchparse(rendered, archLine[0], archLine[2]), arch);
    assert.equal(searchparse(rendered, versionLine[0], versionLine[2]), "9.9.9");
  }
  // One line each: the first match is the define itself.
  assert.equal(template.match(/^!define ARCH "\{\{arch\}\}"$/gm)?.length, 1);
  assert.equal(template.match(/^!define VERSION "\{\{version\}\}"$/gm)?.length, 1);
});
