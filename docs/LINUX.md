# ProxysVPN Desktop on Linux

Status: the Linux target is implemented and type-checked, but **never built or
run on a Linux machine** — the development box is an Apple Silicon Mac with no
Docker, no Windows and no Linux. The GitHub Actions workflow
`.github/workflows/build-linux.yml` is the first place this code is really
compiled and packaged. What is verified and what is not is listed at the bottom.

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
npm run tauri -- build --bundles deb,appimage
```

Build on **Ubuntu 22.04**, not 24.04: a binary linked against glibc 2.39 will not
start on 22.04, and 22.04 is still the most common desktop LTS. CI pins the
runner image for the same reason.

Runtime packages (what the `.deb` asks for): `libwebkit2gtk-4.1-0`, `libgtk-3-0`,
`iproute2`; recommended `libayatana-appindicator3-1` (tray icon) and
`systemd-resolved` (clean per-link DNS).

`pkexec` (polkit) is **not** in `Depends` on purpose — the package name moved
between Ubuntu releases, and an unsatisfiable dependency breaks installation for
everyone. Without it the app starts and says so instead of failing silently.

## How the tunnel is put together

Same chain as macOS:

```
tun2socks(proxysvpn0) -> socks5 127.0.0.1:10808 (xray) -> node
                                 \-> 127.0.0.1:10809 (hysteria) -> node   (hy2)
```

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
  so tun2socks would still fail; and capabilities do not survive an AppImage
  mount, which would mean two different privilege models for `.deb` and AppImage.

The polkit action is `com.proxysvpn.desktop.helper`
(`src-tauri/linux/com.proxysvpn.desktop.policy`, installed by the `.deb` to
`/usr/share/polkit-1/actions/`). It only makes the dialog readable and lets
`auth_admin_keep` cache the answer: when no action matches the program path,
`pkexec` falls back to the built-in `org.freedesktop.policykit.exec` action and
everything still works with a generic prompt. That is what happens in the
AppImage, which cannot install a policy file.

The action's `exec.path` is `/usr/bin/ProxysVPN`, which is where Tauri should put
a binary named after `productName`. Confirm on the first CI run — the workflow
prints `dpkg --contents` — and fix that one line if the bundler names it
differently.

The helper refuses to exec a sidecar that is group- or world-writable, or that is
neither root-owned nor sitting in its own directory; `PROXYSVPN_HELPER_DEV=1`
relaxes the last rule for `cargo run`. The invariant is "no easier to tamper with
than the helper binary itself".

Prefer the `.deb` for the strongest posture: in an AppImage the executable that
pkexec runs lives in a user-writable file.

### Teardown

* normal quit — the GUI closes the pipe, the helper tears down and exits;
* GUI crash or SIGKILL — the pipe hits EOF, same path;
* helper killed — `purge_stale_sync()` on the next start removes the leftover
  device, the host route (remembered in `/run/proxysvpn/route-hint`) and the
  `/etc/resolv.conf` backup, and kills orphaned tun2socks processes;
* reboot — `/run` is a tmpfs, so nothing survives it anyway.

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
  `/run/proxysvpn/` (a symlink target is stored as a target, a regular file is
  copied) and restored on teardown or at the next start after a crash. A second
  apply never overwrites the first backup, and the symlink is unlinked rather
  than written through, so resolved's own stub file is left alone.

Defaults are `1.1.1.1` and `1.0.0.1`; override with
`PROXYSVPN_DNS=9.9.9.9,149.112.112.112`. When `tunnel_prefs` lands from
`feat/app-redesign`, that is what should feed this list.

## Layout of the platform code

```
src-tauri/src/
  paths.rs                      triples and every path, per platform
  privilege.rs                  is_elevated, leftover-engine cleanup, pkexec checks
  helper/proto.rs               the JSON protocol + validation (compiled everywhere)
  helper/server.rs              the root helper's main loop            (linux)
  tun/mod.rs                    portable: state, resolve, supervisor, crash hint
  tun/sys/contract.rs           what every backend must implement
  tun/sys/macos.rs              route/ifconfig/netstat                 (macos)
  tun/sys/linux/mod.rs          GUI side: talks to the helper          (linux)
  tun/sys/linux/net.rs          privileged ip/resolvectl primitives    (linux)
  tun/sys/linux_logic.rs        parsers and resolv.conf logic (compiled everywhere)
```

`linux_logic.rs` and `helper/proto.rs` are compiled on macOS too, so their unit
tests run on the only machine available: `cargo test` covers `/proc/net/route`
decoding (little-endian hex), split-default detection, `ip route get` parsing,
the resolv.conf backup/restore dance including the symlink and crash cases,
request validation and the sidecar trust rule. 50 tests, green on macOS.

## Verified / not verified

Verified here:

* the whole crate compiles and all tests pass for macOS (`cargo test`, 50 tests);
* the Linux modules type-check — they were compiled once with the `cfg` gates
  lifted, zero errors (they use only `std` and `libc` items that exist on both);
* `scripts/fetch-binaries.sh` really downloads and sha256-verifies the Linux
  x86_64 **and** aarch64 sidecars; `file` confirms the architectures;
* tun2socks flag names (`-device`, `-proxy`, `-mtu`, `-loglevel warn`) read from
  the binary's own `--help`.

**Not** verified, and not verifiable without a Linux machine:

* that the crate links and the `.deb`/AppImage actually build — CI decides;
* that `tun2socks -device tun://proxysvpn0` creates the device under the name we
  then configure;
* `pkexec` behaviour in a real desktop session (Wayland and X11), with and
  without our policy file, and inside an AppImage;
* that `/usr/bin/ProxysVPN` is the path the `.deb` really installs;
* whether `resolvectl domain '~.'` is enough on distros where NetworkManager
  manages DNS itself, and whether the `/etc/resolv.conf` fallback survives a
  NetworkManager rewrite (the supervisor re-applies it, but only every 5 s);
* tray behaviour on desktops without an AppIndicator host (the window close is
  allowed to quit there, instead of hiding into a tray that does not exist);
* aarch64 Linux end to end — the CI job for it is behind a `workflow_dispatch`
  input because ARM runners are not always available.
