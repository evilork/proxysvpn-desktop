# ProxysVPN Desktop on Windows

Status: built and packaged by CI (`windows-latest`). Run live only on a
Windows 11 ARM VM (x64 emulation): 0.3.1 connected on 01.10.2026; 0.3.2 failed
to configure the adapter ("Element not found"), and the build that became 0.3.3
connected and reconnected (VLESS, Germany) on 02.10.2026. On that VM a connect
takes 30 s to 2 min because Wintun under emulation cannot remove the previous
run's adapter; a native arm64 build would avoid it. Not yet run on real x64
hardware; everything below marked "unverified" still is.

## How the tunnel works

The same core as macOS 0.3.1 (`src-tauri/src/lib.rs`): same engines, same chain,
same split routing, same supervisor and repair ladder. xray is always the front
of the chain, so the split-routing rules apply on Hysteria2 locations too:

```
VLESS:  tun2socks(Wintun "ProxysVPN") -> socks5 127.0.0.1:10808 (xray) --vless--> node
Hy2:    tun2socks(Wintun "ProxysVPN") -> socks5 127.0.0.1:10808 (xray) --socks--> 127.0.0.1:10809 (hysteria) -> node
```

xray binds its sockets to the physical adapter (`sockopt.interface`, the adapter
alias from `net::physical_route`), which keeps its `direct` outbound from looping
back into the tunnel. The Windows side of the core is `src-tauri/src/tun_platform.rs`,
shared with Linux; a location change moves only the node's host route
(`net::retarget`), the adapter and the split defaults stay up.

Startup order (`crates/pvpn-platform/src/net/local.rs`, shared with macOS):

1. host route `<node>/32` through the physical adapter, so the engines keep
   reaching the node once the default points into the tunnel;
2. start tun2socks, which creates the Wintun adapter itself;
3. address the adapter (198.18.0.1/24), set interface metric 1, and explicitly
   leave it **without** DNS servers;
4. install `0.0.0.0/1` and `128.0.0.0/1` on the adapter — more specific than the
   physical default, which stays in the table untouched.

Teardown runs in the fixed order pinned by `net::TEARDOWN_ORDER`
(`crates/pvpn-platform/src/net/mod.rs`). Both the startup order and every
rollback path are unit-tested in `net/local.rs` against a fake backend that
records the calls, so a reordering or a forgotten undo fails the build rather
than leaking a route on a user's machine.

## Platform specifics

**Reads use IP Helper, writes use netsh.** `route print` and `netsh … show …`
are localised — on a Russian Windows the headers are Russian — so nothing parses
their output. Route and adapter facts come from `GetIpForwardTable2`,
`GetBestRoute2`, `ConvertInterfaceAliasToLuid` and `ConvertInterfaceLuidToIndex`.
Writes go through `netsh`, whose exit code is all we read, which keeps the unsafe
FFI surface to four read-only calls.

**Every route write is `store=active`.** Nothing we install survives a reboot,
which makes a crash cheap: the Wintun adapter dies with tun2socks and takes its
addresses and routes with it, and the only thing that can outlive the process is
the `/32` host route — recorded in the route hint
(`%LOCALAPPDATA%\ProxysVPN\route-hint`) and removed on the next start.
`netsh interface ipv4 delete route` needs the interface as well as the prefix
(without it netsh exits with "essential parameters were not entered" and the
route stays), so every delete is built from the row IP Helper reports for that
`/32`: its interface index and next hop. A leftover that is exactly the route a
connect wants is reused rather than re-added, because the add would fail with
"object already exists".

**Elevation: one UAC prompt at launch.** The exe carries
`requestedExecutionLevel="requireAdministrator"`
(`src-tauri/windows-app.manifest`, attached in `build.rs`). That mirrors the
macOS model — one prompt, one elevated process tree, no IPC — at the cost of
running the WebView elevated too. A split helper (unprivileged GUI + small
elevated service) is the right next step for both Windows and Linux; the `net`
contract is narrow enough to move behind IPC without touching callers.

Links do not inherit that. The opener plugin would call ShellExecute from the
elevated process, and a browser or Telegram that is not yet running would start
as administrator. On Windows the window hands links to the core's
`open_external_unelevated` instead, which gives them to the person's own
Explorer (`%SystemRoot%\explorer.exe <url>`, pvpn-platform `shell.rs`): the
running shell starts the default handler with normal rights. Only the opener
plugin's four schemes pass, commas are encoded, and anything Explorer would
read as a switch or a path is refused. Unverified on a real machine: that the
hand-off to the running shell happens for URLs on every supported build.

**DNS is deliberately not redirected (yet).** The 0.3.1 xray config does answer
DNS inside the tunnel now (`198.18.0.2`, routed to `dns-out`; macOS points its
system resolver there, Linux publishes it on the device), but Windows still
leaves the adapter alone until that is tried on a real machine. The adapter is
therefore left with no DNS server at all, which also stops the Windows resolver
from querying *into* the tunnel and taking whatever comes back first. Names keep
resolving through the physical adapter's servers, which is a leak; closing it
means publishing `198.18.0.2` on the adapter plus NRPT rules.

**Engines die with the app.** Each engine joins a kill-on-close job object at
spawn (`pvpn_platform::process::tie_to_app`); the app holds its only handle, so a
crash or "End task" takes xray, hysteria and tun2socks along. There is
deliberately no `taskkill /IM` sweep: it would also stop another VPN client's
`tun2socks.exe` or `xray.exe`.

**The installer stops the running app itself.** Tauri's own check before files
are copied or removed ("ProxysVPN is running, press OK to close it") failed to
close the elevated tray app once in three upgrades on a Windows 11 VM and
aborted the install. `src-tauri/nsis/installer-hooks.nsh` (wired in through
`bundle.windows.nsis.installerHooks`) runs first, in both
`NSIS_HOOK_PREINSTALL` and `NSIS_HOOK_PREUNINSTALL`: `taskkill /F /T` on our own
main binary only — the engines are its children and in its job object — then a
bounded wait until the process is gone. An app that is not running is the
normal case and never fails the install. The uninstallers of 0.3.2 and earlier
carry no hook, so the very first upgrade from them still goes through the old
check once.

**Paths.** State lives in `%LOCALAPPDATA%\ProxysVPN` (hysteria config, route
hint, the subscription link, device id and preferences) and logs in
`%LOCALAPPDATA%\ProxysVPN\logs\app.log`. Not `%PROGRAMDATA%`:
the hysteria config holds the node password and the default ACL on ProgramData
grants every local user read access.

**No console windows.** Every child process is spawned with `CREATE_NO_WINDOW`
(`pvpn_platform::process::no_window`); without it each engine start would flash
or keep a console window.

## Known gaps

Not fixed in this branch, each for the reason given; none of them is in the
0.3.1 macOS build.

* **The elevated app writes into a folder the person's unelevated processes
  control.** `%LOCALAPPDATA%\ProxysVPN` can be renamed away by any process of
  the same user and replaced with a junction (to `\RPC Control`, plus an object
  manager link per file name). The elevated app then deletes and overwrites
  through it: the route hint and the hysteria config (`write_private_file`
  unlinks first), and since the 0.3.1 merge every plain `fs::write` of the
  network memory, the tunnel and notification settings, the subscription link,
  the manifest cache and the device id. That is an unelevated-to-administrator
  file write and delete, the same class PR #3 closed for `netsh.exe`. The fix is
  a folder only administrators can change — `%ProgramData%\ProxysVPN` created
  by the elevated process with a protected DACL for SYSTEM and Administrators,
  checking ownership and reparse points of a folder that already exists (Users
  may create folders in ProgramData) — or every write opened with
  `FILE_FLAG_OPEN_REPARSE_POINT` and checked with `GetFinalPathNameByHandle`.
  Either needs a Windows machine to get right; nothing of the Windows build has
  shipped, so moving the folder needs no migration. Until then the honest
  statement is: on Windows, malware already running as the person can use the
  app to write as administrator.
* **DNS and IPv6 go past the tunnel** (see above). The data notice and the
  Tunnel screen say so on Windows (`src/platformCopy.ts`).

## Building

```sh
npm ci
scripts/fetch-binaries.sh                 # host platform
npm run tauri build                       # NSIS installer, per-machine
```

On Windows the fetch step also downloads `wintun.dll` and its licence; both are
bundled into the install root next to `tun2socks.exe`, because Windows searches
the loading executable's own directory first. See `THIRD-PARTY-NOTICES.md`.

The installer is NSIS, `perMachine` (the app needs administrator rights
anyway), with the WebView2 evergreen bootstrapper for Windows 10 machines that
lack the runtime. **There is no code signing**, so SmartScreen will warn on
first run until an EV certificate is in place — that is an owner decision.

## What can be checked without a Windows machine

```sh
scripts/check-platform.sh
```

It builds and tests everything on the host and then runs
`cargo clippy -p pvpn-platform --all-targets --target x86_64-pc-windows-msvc
-- -D warnings`, which type-checks *and* lints the whole Windows platform layer
including its `windows`-crate FFI. Clippy rather than `cargo check` on purpose:
the lints are where the FFI mistakes show up, and this target gets no other
review on this machine. CI runs the same script, so the two cannot drift.

`cargo check` for the **whole app** with that target cannot work here: `reqwest`
pulls in `ring`, whose build script compiles C and fails with
`fatal error: 'assert.h' file not found` without the MSVC toolchain and Windows
SDK. That is exactly why the platform layer is a separate crate.

## Unverified, needs CI or hardware

* that `-device ProxysVPN` makes tun2socks create a Wintun adapter whose
  interface alias is exactly `ProxysVPN` (`device_index` finds the adapter by
  that alias, and `wait_for_device` fails after 5s if it does not);
* that `wintun.dll` placed in the install root is found by `tun2socks.exe`;
* the exact `netsh` argument spelling on every supported build;
* that the NSIS bundle actually puts resources and sidecars in the same
  directory;
* behaviour on logoff/shutdown: there is no reliable notification for a GUI
  process, so the design relies on `store=active` plus route-hint cleanup rather
  than on a teardown handler.

## Content security policy

`tauri.conf.json` carries a policy now (it used to be `"csp": null`), which
matters more here than on Linux because the WebView runs elevated:

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';
img-src 'self' data:; font-src 'self' data:;
connect-src 'self' ipc: http://ipc.localhost;
object-src 'none'; frame-src 'none'; base-uri 'self'; form-action 'none'
```

`'unsafe-inline'` for styles because the UI uses a few `style=` attributes, and
`data:` for images because the pairing QR code is a data URL. Everything the app
fetches over the network is fetched by Rust, so the WebView itself needs no
outside origin — `connect-src` only has to cover Tauri's own IPC, which is
`ipc:` on macOS and `http://ipc.localhost` on Windows and Linux.

Verified against the real production bundle (`dist/`) by serving it with this
exact policy and loading it in a browser: the bundle executes, React mounts, the
stylesheet applies, the `style=` attributes take effect and a `data:` image
loads, with no CSP violation reported. What that does **not** cover is Tauri's
own IPC under the policy, because the test page had no Tauri runtime — if a
command call ever fails with a CSP error in the console, `connect-src` is where
to look.
