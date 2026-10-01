# ProxysVPN Desktop on Linux

Status: built and packaged by CI (`.github/workflows/desktop-build.yml`,
ubuntu-22.04). The 0.3.1 `.deb` was installed on a Debian 13 live VM on
01.10.2026, redeemed a pair code and connected (VLESS, Germany). Not yet run
on real hardware, not yet tried on a Hysteria2 location, and 0.3.2 (the audit
fixes) has not been run live yet. What else is verified and what is not is
listed at the bottom.

## Build dependencies

Tauri v2 links the webview against **webkit2gtk-4.1** (not 4.0). On Debian and
Ubuntu:

```sh
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  librsvg2-dev \
  libayatana-appindicator3-dev \
  patchelf build-essential curl wget file unzip
```

Then, once:

```sh
./scripts/fetch-binaries.sh                 # sidecars for this host, sha256 pinned
npm ci
npm run tauri -- build --bundles deb
```

Build on **Ubuntu 22.04**, not 24.04: a binary linked against glibc 2.39 will not
start on 22.04, and 22.04 is still the most common desktop LTS. CI pins the
runner image for the same reason.

Runtime packages (what the `.deb` asks for): `libwebkit2gtk-4.1-0`, `libgtk-3-0`,
`iproute2`, `pkexec | policykit-1` and
`libayatana-appindicator3-1 | libappindicator3-1`.

`systemd-resolved` is deliberately not even recommended. apt installs
recommendations by default, and on a desktop that does not already run it
(Debian 13 live checked on 01.10.2026) installing it switches
`/etc/resolv.conf` to the stub at 127.0.0.53 while NetworkManager never hands
it a server: every name lookup on the machine fails until resolved is removed.
The app does not need it: with resolved running it sets per-link DNS through
`resolvectl`, without it it rewrites `/etc/resolv.conf` and restores it.

`pkexec` is a dependency with both package names as alternatives: it moved from
`policykit-1` (Ubuntu 22.04) into a package of its own (Debian 12, Ubuntu 23.04+),
and either satisfies apt. Without pkexec there is no way to start the helper;
if it is missing anyway, the window says so with its own error
(`ELEVATION_UNAVAILABLE`) instead of offering a Retry that cannot work. The tray
library is a dependency too: with neither of the two present, `libappindicator`
panics while the tray is built.

**No AppImage.** An AppImage runs from a FUSE mount that only the user who
mounted it may access — root included, since it is mounted without
`allow_other`. pkexec would ask for the password and then fail to execute the
helper from that mount (exit 127), so the tunnel could never come up. The app
recognises that case (statfs reports FUSE for its own executable) and shows
`ELEVATION_UNAVAILABLE` before any password dialog; CI builds only the `.deb`.

## How the tunnel is put together

The same core as macOS 0.3.1 (`src-tauri/src/lib.rs`, with the tunnel in
`src-tauri/src/tun_platform.rs`), so the same chain — xray is always the front,
and the split-routing rules apply on Hysteria2 locations too:

```
VLESS:  tun2socks(proxysvpn0) -> socks5 127.0.0.1:10808 (xray) --vless--> node
Hy2:    tun2socks(proxysvpn0) -> socks5 127.0.0.1:10808 (xray) --socks--> 127.0.0.1:10809 (hysteria) -> node
```

xray and hysteria run as the user, in the GUI; only tun2socks runs in the root
helper. xray binds its sockets to the physical interface (`sockopt.interface`,
`SO_BINDTODEVICE`, allowed unprivileged on an unbound socket since Linux 5.7),
read from `/proc/net/route` by the GUI itself. A location change sends the
helper `Retarget`, which pins the new node's address before it drops the old
one; the device, the split defaults and DNS stay up.

and the same routing shape:

* `198.18.0.1/15` on `proxysvpn0`, MTU 1500 (tun2socks terminates TCP locally, so
  there is no encapsulation overhead to subtract);
* `0.0.0.0/1` and `128.0.0.0/1` via `proxysvpn0` — two half-defaults that beat the
  real `0.0.0.0/0` on prefix length without deleting it, so the ISP's default and
  any other VPN's come back untouched;
* a `/32` host route for the node over the physical link, so the tunnel's own
  packets do not re-enter the tunnel.

The physical default route is read from `/proc/net/route`, a kernel ABI with no
localisation and no iproute2 version drift — and, unlike `ip route get`, not
answered by our own half-default once the tunnel is up.

Policy routing (`ip rule` + a private table, or tun2socks' `-fwmark`) is not used:
nothing else on a normal desktop claims `/1`, and keeping the same shape on all
three platforms is worth more than the extra isolation.

## Privileges: one prompt, and the GUI stays unprivileged

The GUI never runs as root. The same executable, started through `pkexec` with
`--helper`, becomes a small privileged helper that owns the device, the routes
and DNS, and speaks line-delimited JSON over stdin/stdout. It is spawned once per
app session and kept across connect/disconnect, so the user sees **one**
authorization dialog — the macOS launcher's feel, with a smaller root surface
(on macOS the whole process, webview included, is root).

Why not the alternatives:

* **GUI as root** — impossible on Wayland: a root process cannot talk to the
  user's compositor, so no window appears. On X11 it needs `xhost +si:localuser:root`.
* **File capabilities** (`setcap cap_net_admin+ep`) — not inherited by children,
  so tun2socks would still fail.

The polkit action is `com.proxysvpn.desktop.helper`
(`src-tauri/linux/com.proxysvpn.desktop.policy`, installed by the `.deb` to
`/usr/share/polkit-1/actions/`). It only makes the dialog readable and lets
`auth_admin_keep` cache the answer: when no action matches the program path,
`pkexec` falls back to the built-in `org.freedesktop.policykit.exec` action and
everything still works with a generic prompt.

The action's `exec.path` is `/usr/bin/proxysvpn-desktop`. Note that this is the
**Cargo package name**, not `productName`: Tauri uses productName only for the
`.desktop` entry and `/usr/lib/ProxysVPN`. Verified by unpacking the `.deb` from
the first CI build, so the dialog gets our wording rather than the generic
fallback.

The peer does not tell the helper which binary to run. Until 0.3.2 the `Up`
request carried a tun2socks path, which the helper checked and then exec'd by
name again later; only the last path component was protected against symlinks,
so a path through a directory the peer controlled could pass the check against
`/usr/bin` and resolve to another file at exec time — root code execution with no
dialog inside the `auth_admin_keep` window. Now the helper finds `tun2socks`
next to its own executable (`/proc/self/exe`, on the `.deb` the root-owned
`/usr/bin`), and a request that still names a path is refused by
`deny_unknown_fields`. It also refuses a sidecar that is group- or
world-writable, that is a symlink, or that is neither root-owned nor sitting in
its own directory. `PROXYSVPN_HELPER_DEV=1` adds the repo's `binaries`
directory (a compile-time path) and relaxes the ownership rule for `cargo run` —
**in a debug build only**: the flag that asks for it is an argument of the
helper, and the helper's argv comes from an unprivileged peer, so a release
build ignores it outright instead of trusting that no bundle sets the variable.
The invariant is "no easier to tamper with than the helper binary itself".

The same reasoning applies to the rest of the request: the SOCKS port is checked
against this app's own two engine ports, because it decides where every packet on
the machine goes once the half-defaults are installed, and a request line is
bounded at 64 KiB. polkit's `auth_admin_keep` means an authorization earned by
one connect is reusable without a password for the rest of the keep window, so
the helper assumes its peer may not be our GUI.

Checking the port number is not enough on its own. The 0.3.1 core restarts xray
with the tunnel up (a location change, a revived engine), and between the stop
and the start nobody listens on 127.0.0.1:10808 — any local user could bind it
and receive the whole machine's traffic from our root tun2socks. So the helper
also reads `/proc/net/tcp{,6}` on `Up` and on every `Ensure` tick: a listener
that a connection to 127.0.0.1:<port> could reach must belong to root or to
`PKEXEC_UID`. On `Up` a stranger is refused; on `Ensure` the helper lowers the
tunnel and says why, and the GUI then reports protection as dropped. An empty
port is fine: that is the restart itself. Windows does the same against its TCP
table and the pids of its own engines (`net/windows.rs`).

### Teardown

* normal quit — the GUI closes the pipe, the helper tears down and exits;
* GUI crash or SIGKILL — the pipe hits EOF, same path;
* helper killed — `purge_stale_sync()` on the next start removes the leftover
  device, the host route (remembered in `/run/proxysvpn/route-hint`) and the
  `/etc/resolv.conf` backup, and kills the tun2socks the previous helper
  recorded in the route hint (`engine_pid=`), only while that pid still runs
  the helper's own binary — never every root `tun2socks` by name, which would
  stop another VPN client's engine;
* reboot — the route hint is in `/run`, a tmpfs, and that is right: routes do
  not survive a reboot either, so a hint that did would name entries that no
  longer exist. The `/etc/resolv.conf` backup is the opposite case and lives in
  `/var/lib/proxysvpn`: our replacement file *is* on disk and does survive, so
  the copy of the original has to as well — with it in a tmpfs, a power cut left
  the machine pointed at our resolvers with nothing left to restore.
  `purge_stale_sync()` puts it back on the next start, which means the machine
  keeps our resolvers until the app is launched again; a systemd unit that
  restores it at boot instead is the proper fix and is not written yet. The
  backup only ever goes back over our own file: when NetworkManager (or
  resolvconf, or resolved) has rewritten `/etc/resolv.conf` since — at boot on
  another network, say — theirs is newer, so it is kept and only the stale
  backup is deleted. Disconnect and the .deb's `postrm` follow the same rule.

## DNS

This is the one step macOS does not need. On macOS the system resolver keeps
using the physical link's servers and those queries are simply proxied, because
their addresses fall under `0.0.0.0/1`. On Linux the usual upstream is the LAN
router (`192.168.x.1`), which stays on-link and therefore leaks, and
systemd-resolved can pin a query to the physical link regardless of routes.

So the helper publishes resolvers that are only reachable through the tunnel:

* **systemd-resolved present** — `resolvectl dns proxysvpn0 <servers>`,
  `resolvectl domain proxysvpn0 '~.'`, `resolvectl default-route proxysvpn0 yes`,
  caches flushed. Reverted with `resolvectl revert` and by the link disappearing.
* **otherwise** — `/etc/resolv.conf` is replaced, with the original remembered in
  `/var/lib/proxysvpn/` (a symlink target is stored as a target, a regular file
  is copied) and restored on teardown or at the next start after a crash. A second
  apply never overwrites the first backup, and the symlink is unlinked rather
  than written through, so resolved's own stub file is left alone.

The default is `198.18.0.2`, the in-tunnel resolver the 0.3.1 xray config answers
through `dns-out` — the same one macOS hands its system resolver (`sysdns.rs`) —
so the DNS settings of the "Туннель" screen apply on Linux too. Override with
`PROXYSVPN_DNS=9.9.9.9,149.112.112.112`.

## Layout of the platform code

```
src-tauri/crates/pvpn-platform/src/      (no tauri, no reqwest -> cross-compiles)
  paths.rs                      every path, per platform, incl. /run/proxysvpn
  privilege.rs                  is_elevated, pkexec checks, `which`
  helper/proto.rs               the JSON protocol + validation (compiled everywhere)
  helper/server.rs              the root helper's main loop            (linux)
  net/mod.rs                    THE contract: preflight/up/ensure/down/...
  net/plan.rs                   pure argv for all three platforms (compiled everywhere)
  net/linux_logic.rs            parsers and resolv.conf logic  (compiled everywhere)
  net/linux.rs                  GUI side: talks to the helper          (linux)
  net/linux_priv.rs             privileged ip/resolvectl steps         (linux)
  net/local.rs                  the shared sequence for macOS + Windows
  net/macos.rs, net/windows.rs  their own primitives + the contract
  net/engine.rs                 the tun2socks handle, where we own it

src-tauri/src/
  tun.rs                        portable: resolve, plan, the 5 s supervisor
```

Note the asymmetry, which is the whole design: on macOS and Windows the app is
privileged and `net/local.rs` runs the sequence in-process; on Linux the GUI is
unprivileged and the *same* sequence runs inside the helper, reached through
three coarse operations. The contract in `net/mod.rs` is coarse for exactly that
reason — see the comment at the top of that file.

`linux_logic.rs` and `helper/proto.rs` are compiled on macOS too, so their unit
tests run on the only machine available: `cargo test` covers `/proc/net/route`
decoding (little-endian hex), split-default detection, `ip route get` parsing,
the resolv.conf backup/restore dance including the symlink and crash cases,
request validation and the sidecar trust rule. It also covers the Linux argv
itself (`net::plan::linux`) and the shared up/ensure/down sequence with a fake
backend, including every rollback path. All green on macOS; the count is in the
`cargo test` output and deliberately not repeated here, because the two copies
of it this file used to carry had already drifted apart.

Beyond the tests, the whole Linux backend — the helper, the route code, the DNS
code — is **compiled** for `x86_64-unknown-linux-gnu` on the developer's Mac and
linted with `-D warnings`:

```
scripts/check-platform.sh
```

That is not a substitute for running it, but it does mean no Linux code path in
this branch is unseen by a compiler.

## Verified / not verified

Verified here:

* the whole workspace compiles and all tests pass on macOS (`cargo test`);
* the Linux code is **compiled for Linux** — not with the `cfg` gates lifted, but
  really, for `x86_64-unknown-linux-gnu`, and linted with `clippy -D warnings`.
  That is possible because the platform layer is its own crate with no
  tauri/reqwest/rustls, so it has no `ring` and needs no C compiler. Run it with
  `scripts/check-platform.sh`; the `cross-check` job runs that very script, so a
  target added to it cannot be missed by CI;
* `scripts/fetch-binaries.sh` really downloads and sha256-verifies the Linux
  x86_64 **and** aarch64 sidecars; `file` confirms the architectures;
* tun2socks flag names (`-device`, `-proxy`, `-mtu`, `-loglevel warn`) read from
  the binary's own `--help`.

**Not** verified, and not verifiable without a Linux machine:

* ~~that the crate links and the `.deb`/AppImage build~~ — done on the first green
  CI run of the 0.1.0 app: `ProxysVPN_0.1.0_amd64.deb` (41 MB) and
  `ProxysVPN_0.1.0_amd64.AppImage` (114 MB). Neither has been **launched**, which is the next line;
* that the app actually starts, shows a window and raises a tunnel. Nothing below
  this point has run on a Linux machine even once;
* that `tun2socks -device tun://proxysvpn0` creates the device under the name we
  then configure;
* `pkexec` behaviour in a real desktop session (Wayland and X11), with and
  without our policy file;
* ~~that the `.deb` installs the binary where the policy expects~~ — confirmed on
  the first CI build: `usr/bin/proxysvpn-desktop`, root:root 0755, and the
  sidecars beside it are root-owned too, so the helper's trust check passes;
* whether `resolvectl domain '~.'` is enough on distros where NetworkManager
  manages DNS itself, and whether the `/etc/resolv.conf` fallback survives a
  NetworkManager rewrite (the supervisor re-applies it, but only every 5 s);
* tray behaviour on desktops without a StatusNotifier host (stock GNOME): the
  close button minimises there instead of hiding into an icon nobody can see
  (`pvpn-platform` tray.rs asks the session bus), and a second launch shows the
  running window (single-instance plugin) instead of starting a second core;
* aarch64 Linux end to end; the matrix builds x86_64 only, since ARM runners are
  not reliably available;
* the AppImage refusal (`ELEVATION_UNAVAILABLE` from statfs on a FUSE mount) —
  reasoned from the kernel's FUSE access rule, never seen on a machine.
