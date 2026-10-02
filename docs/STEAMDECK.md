# ProxysVPN Desktop on a Steam Deck

Status (02.10.2026): **compiled and unit-tested, never run on a Deck.** There
was no Steam Deck, no SteamOS and no Linux machine to try it on. The CI job
"Linux x86_64 (AppImage)" builds the AppImage and checks what is inside it;
everything else on this page that depends on SteamOS itself is reasoned from
its documented behaviour and public reports, and listed at the bottom as not
verified.

The Deck build is the Linux x86_64 **AppImage**,
`ProxysVPN_<version>_amd64.AppImage`. It is not a separate app: the same
binary, the same tunnel as the `.deb` (docs/LINUX.md). What differs is how the
root helper is started, because neither of the usual ways works on SteamOS:

* **No .deb.** SteamOS's `/usr` is a read-only image that every OS update
  replaces, so nothing can be installed into `/usr/bin` or
  `/usr/share/polkit-1/actions` the way the `.deb` does.
* **No Flatpak.** The Flatpak sandbox has no `CAP_NET_ADMIN`, so no TUN device
  and no routes.
* **The AppImage alone is not enough either.** It runs from a FUSE mount
  (`/tmp/.mount_*`) that root may not enter, so pkexec cannot run the helper
  from it — which is why 0.3.3 refused to connect from an AppImage at all.

So on the first connect the AppImage copies its root helper into a folder only
root can change, and runs it from there afterwards. The code and the full
reasoning are in `src-tauri/crates/pvpn-platform/src/helper/install.rs`.

## Install

Everything here happens once, in **Desktop Mode** (Steam button → Power →
Switch to Desktop).

1. **Download** `ProxysVPN_<version>_amd64.AppImage`, for example into
   `~/Applications`. Check it against the release's sums in Konsole:

   ```bash
   cd ~/Applications && sha256sum ProxysVPN_*_amd64.AppImage
   ```

2. **Make it executable.** A download never keeps the executable bit:

   ```bash
   chmod +x ~/Applications/ProxysVPN_*_amd64.AppImage
   ```

   (or in Dolphin: right click → Properties → Permissions → "Is executable").

3. **Give the `deck` user a password**, if it has none yet. SteamOS ships the
   `deck` user without one, and the system's permission dialog cannot accept
   "nothing". In Konsole:

   ```bash
   passwd
   ```

   and type the new password twice. If you already set one (for `sudo`, for
   example), skip this. If you forget, the app says so: a dismissed or refused
   dialog on SteamOS shows "The deck user needs a password" with these same
   steps (`STEAMOS_PASSWORD_NEEDED`).

4. **First connect, in Desktop Mode.** Double-click the AppImage. Before
   anything else the app shows its own screen, **"Gaming Mode without a
   password"**: the first connect will ask for the `deck` password once, and
   from then on ProxysVPN can connect in Gaming Mode without one; the
   permission stays on the Deck until it is removed in **More → Remove system
   files**. Press "Got it, continue", sign in with the pair code, press
   Connect. The system's permission dialog appears once. It is polkit's
   generic one — the AppImage cannot install a policy file of its own — and
   pkexec quotes the command it runs, cut to its first 38 and last 37 bytes,
   so it reads "Authentication is needed to run `/bin/sh -c # ProxysVPN sets
   up its VPN ... so Gaming Mode then needs no password' as the super user"
   (on another Linux, or once Gaming Mode was decided: "... helper as root
   and starts the tunnel."). The command's first line says it in full.
   Type the `deck` password. That one dialog:

   * copies `proxysvpn-helper` and `tun2socks` out of the AppImage into
     `/home/.proxysvpn/bin/`, checking that the copies are byte for byte the
     files in the AppImage (SHA-256, checked by root on its own copy);
   * records that `deck` may start that helper without a password
     (`/home/.proxysvpn/gaming-mode-user`, SteamOS only);
   * starts the helper, and the tunnel comes up.

   The helper then writes `/etc/polkit-1/rules.d/49-proxysvpn.rules` from that
   record. From then on **no password is asked again**, in Desktop Mode or in
   Gaming Mode, until the app is updated.

## Gaming Mode

Add the AppImage to Steam once, in Desktop Mode:

1. Open Steam (the desktop app) → **Games** → **Add a Non-Steam Game to My
   Library…**
2. **Browse…**, set the file type to **All files**, pick the AppImage, then
   **Add Selected Programs**.
3. Optional: in the shortcut's **Properties** rename it "ProxysVPN". Leave
   **Compatibility** off: it is a native Linux program, not one for Proton.

Back in Gaming Mode it is under **Library → Non-Steam**. The app has no
gamepad UI: drive it with the right trackpad as a mouse (press the trackpad to
click). If the shortcut's controller layout does not move the pointer, pick a
template with a mouse in its controller settings. Steam + X opens the
on-screen keyboard for the pair code. The window is the desktop one, and
gamescope scales it to the screen.

The tunnel lives as long as the app runs, exactly as on the desktop. Opening a
game through the Steam button leaves ProxysVPN running in the background;
**Exit game** on ProxysVPN disconnects.

If Gaming Mode shows **"Connect once from Desktop Mode"**
(`STEAMOS_DESKTOP_MODE_NEEDED`), the one-time setup has not been made, or the
app was updated since: Gaming Mode runs no polkit agent, so no password window
can appear there, and the copy needs one. Do step 4 above in Desktop Mode, then
come back.

## Updating

Replace the AppImage file (keep the Steam shortcut pointing at the same path,
or edit its target). The next connect finds that the copy in
`/home/.proxysvpn` is from another release and copies again — which needs the
password dialog once, so the first connect after an update has to be in Desktop
Mode. The Gaming Mode record is kept.

## What the setup leaves on the device

| Path | Owner, mode | What |
| --- | --- | --- |
| `/home/.proxysvpn/` | root:root 0755 | the folder; `/home` survives SteamOS updates |
| `/home/.proxysvpn/bin/proxysvpn-helper` | root:root 0755 | the root helper, a copy from the AppImage |
| `/home/.proxysvpn/bin/tun2socks` | root:root 0755 | the one engine root runs |
| `/home/.proxysvpn/gaming-mode-user` | root:root 0644 | SteamOS only: the user name the rule is for |
| `/home/.proxysvpn/gaming-mode-off` | root:root | only if you switched Gaming Mode off: the "no" that keeps the record from coming back |
| `/etc/polkit-1/rules.d/49-proxysvpn.rules` | root:root 0644 | SteamOS only: the Gaming Mode rule |
| `/run/proxysvpn/`, `/var/lib/proxysvpn/` | root, 0700 | the helper's crash-recovery notes, as with the `.deb` |

Plus the app's own data in `~/.local/share/ProxysVPN` and its log in
`~/.local/state/ProxysVPN/app.log`, as on any Linux.

**The privilege this leaves, precisely.** The rule lets the user it names,
logged in at the device and in the active session, run exactly
`/home/.proxysvpn/bin/proxysvpn-helper --helper` as root without a password —
that program, that argument, nothing else. So any program running as `deck`
can start the helper. The helper only does what its protocol allows
(`helper/proto.rs`): raise our TUN device `proxysvpn0`, send the machine's
traffic to a SOCKS listener on `127.0.0.1:10808` or `10809` that belongs to
root or to that same user, pin a host route to one unicast IPv4 address,
publish resolvers on our device, and take it all down. It executes nothing but
the `tun2socks` beside it. What a program of that user gains is one thing it
could not do before: route the whole Deck through a proxy it runs itself. Not
root code execution, not any file, nothing that outlasts the helper.

Why that is acceptable: it is the same power a `.deb` install hands out for
polkit's `auth_admin_keep` minutes after every connect, here without the time
limit — the price of a Gaming Mode that cannot ask. No file that carries
privilege sits where a normal user can change it: no setuid bit, no file
capability, and every folder on the rule's path is root's alone (the helper
withdraws the rule instead of writing it if one is not, or if the record is not
a root-only file holding one user name). Rejected alternatives: `setcap` on a
binary (a capability-carrying file, and the capability does not reach
tun2socks anyway), a sudoers drop-in (the same grant through a second
mechanism), and a root service running all the time (more surface, and a design
of its own).

To switch the Gaming Mode grant off but keep the app, record the "no" and
remove the record and the rule (the helper rewrites the rule from the record
at every start, and a setup that finds neither the record nor the "no" writes
the record again):

```bash
sudo sh -c 'touch /home/.proxysvpn/gaming-mode-off && rm -f /home/.proxysvpn/gaming-mode-user /etc/polkit-1/rules.d/49-proxysvpn.rules'
```

Every later connect then asks for the password, Gaming Mode cannot connect,
and an app update does not switch it back on. Only a setup after
`/home/.proxysvpn` is removed writes the record again. The choice is kept in
those two files and nowhere else: a first setup that was cut short (the
power went, the password window was killed) has decided nothing, and the next
one records the user as the first would have.

## DNS on SteamOS

Public reports describe SteamOS as running systemd-resolved, with
NetworkManager handing it the network's servers
(`/etc/NetworkManager/conf.d` sets `dns=systemd-resolved`). The helper picks
its DNS backend at connect time (`net/linux_logic.rs`, `dns_backend`): with
`resolvectl` present and `/run/systemd/resolve` there, it sets the tunnel's
resolver **per link**, on `proxysvpn0` only. Nothing is written to `/etc`. That
setting belongs to the device: if the app or the helper crashes, the helper's
tun2socks dies with it (`PR_SET_PDEATHSIG`), the TUN device disappears, and
resolved falls back to the Wi-Fi's servers by itself; after a reboot there is
no device at all. A crash or a power cut cannot leave the Deck without DNS on
that path.

Should a Deck ever run without resolved, the helper falls back to rewriting
`/etc/resolv.conf`, with the original kept in `/var/lib/proxysvpn` and put
back on disconnect or at the next helper start — the `.deb`'s fallback. A
crash there leaves the tunnel's resolver in the file until the next connect or
until NetworkManager rewrites it, which it does for a regular file it manages;
that is the one case to check on a real Deck (`resolvectl status` while
connected says which backend is in use).

## Removing it

1. **In the app, in Desktop Mode: More → Remove system files.** After a
   confirmation the app disconnects, stops its root helper and, behind one
   password window ("Authentication is needed to run `/bin/sh -c # ProxysVPN:
   remove helper, ... and the Gaming Mode no-password rule.'"), removes
   `/home/.proxysvpn` and `/etc/polkit-1/rules.d/49-proxysvpn.rules`. A fixed
   script does it (`REMOVE_SCRIPT` in `helper/install.rs`): it removes those
   two paths and nothing else, refuses any other name, removes a link in
   their place instead of following it, never follows a link inside the
   folder, and leaves the folder alone if it is not root's alone (the setup
   never makes it so). In Gaming Mode there is no password window, and the
   app says to use Desktop Mode. The row exists in the AppImage only; the
   `.deb` is removed with `apt remove`. If you keep the AppImage, its next
   connect sets everything up again, with the screen and the password.
2. Delete the AppImage, and the shortcut in Steam (right click → Manage →
   Remove non-Steam game from your library).
3. What the helper keeps for crash recovery is left by step 1, because
   `/var/lib/proxysvpn` may hold the `resolv.conf` to put back. Once the app
   is gone, in Konsole:

   ```bash
   sudo rm -rf /var/lib/proxysvpn /run/proxysvpn
   ```

   Without the app (or if step 1 was not possible), remove everything by
   hand:

   ```bash
   sudo rm -rf /home/.proxysvpn /var/lib/proxysvpn /run/proxysvpn
   sudo rm -f /etc/polkit-1/rules.d/49-proxysvpn.rules
   ```

4. The app's own data (the pair, the settings, the log):

   ```bash
   rm -rf ~/.local/share/ProxysVPN ~/.local/state/ProxysVPN
   ```

The `deck` password stays as you set it.

## When something does not work

* The log: `~/.local/state/ProxysVPN/app.log`, the same lines the app's
  support report carries. The helper's lines have the source `helper`; the
  setup's own lines start with `proxysvpn-setup:`.
* **"The app could not prepare its files"** (`ENGINE_STAGE_FAILED`): the copy
  into `/home/.proxysvpn` failed; the log names the reason (a folder that is
  not root's alone, a full disk, a file that did not match its checksum).
* **A blank window in Gaming Mode**: the app already switches WebKitGTK's
  DMA-BUF renderer off under gamescope (`WEBKIT_DISABLE_DMABUF_RENDERER=1`,
  logged as "gamescope session"). If it is still blank, add
  `WEBKIT_DISABLE_COMPOSITING_MODE=1 %command%` to the shortcut's launch
  options.

## How it is built

The CI job "Linux x86_64 (AppImage)" (ubuntu-22.04, so glibc 2.35+) runs
`tauri build --bundles appimage`. `tauri.linux.conf.json` builds the standalone
helper first (`build.beforeBundleCommand`: `cargo build --release -p
pvpn-platform --bin proxysvpn-helper`) and puts it into the image as
`usr/bin/proxysvpn-helper` (`bundle.linux.appimage.files`); the `.deb` does
not carry it. The step "AppImage carries the helper" extracts the image, fails
unless the helper, tun2socks, xray and hysteria are in `usr/bin`, and prints
the helper's `NEEDED` and `RUNPATH` entries. Expect only the C library and
`libgcc_s` there, and, if linuxdeploy set one, a RUNPATH of `$ORIGIN/../lib`:
from `/home/.proxysvpn/bin` that is `/home/.proxysvpn/lib`, inside the
root-owned tree, which is why the helper sits one level down.

## Trying it without a Deck

Any x86_64 Linux VM with a desktop runs the same code. `ID=steamos` in
`/etc/os-release` is the only thing that switches the SteamOS behaviour on
(the Gaming Mode record and rule, the two SteamOS screens), so an Arch VM with
that one line changed (`sudo sed -i.bak 's/^ID=.*/ID=steamos/' /etc/os-release`,
put the `.bak` back afterwards) walks the Deck's path, and stopping the
desktop's polkit agent (`pkill -f polkit-kde-authentication-agent-1`, or the
GNOME / LXQt one) stands in for Gaming Mode. On Debian, leave os-release alone:
that is the plain AppImage path, one generic dialog per app start.

## Verified / not verified

Verified (02.10.2026, on the developer's Mac and by the CI that runs the same
tests on Linux):

* the whole Linux platform layer, the standalone helper binary, the copy and
  the rule code included, **compiles** for `x86_64-unknown-linux-gnu` and is
  clean under `clippy -D warnings` (`scripts/check-platform.sh`);
* unit tests: recognising an AppImage (and not the `.deb` started from one);
  **the removal script under `/bin/sh`**: it removes the whole folder and the
  rule, leaves the rule's neighbours and whatever links point at, refuses
  other names and a folder others can change;
  deciding between "run the copy", "copy again" and "refuse" for every
  ownership, mode and checksum case; **the setup script itself, run under
  `/bin/sh`** as pkexec would run it minus root — it installs both files, hands
  the waiting request to the helper, refuses a swapped or linked staged file
  without leaving it behind, refuses a FIFO or a link to a device in a staged
  file's place without waiting on it (root opens each staged file once, with
  dd's `iflag=nofollow,nonblock`), refuses a staging folder that is not the
  person's alone, refuses folders others can change, records the
  Gaming Mode user on SteamOS while nothing was decided yet (a first setup cut
  short included) and keeps an explicit "no"; the polkit rule's text and its
  quoting; the rule kept in step with the record, repaired after deletion,
  withdrawn for an unsafe path or record or for the "no"; SteamOS detection;
  the error codes;
* `tauri.linux.conf.json` merged over `tauri.conf.json` is accepted by Tauri's
  own config types.

**Not** verified — nothing below has run on a Deck or on any Linux machine:

* that the CI builds the AppImage with the helper in it (the 0.1.0 build made
  an AppImage, without the helper; the first run of this job will tell);
* that the AppImage starts on SteamOS at all, and its window in Desktop Mode
  and under gamescope;
* the KDE polkit agent in Desktop Mode with the generic `/bin/sh` action, with
  and without a `deck` password, and the exact text it shows (taken from
  pkexec's source, `cmdline_short`, and pinned by a unit test);
* that polkit on SteamOS loads the rule and that Gaming Mode's session counts
  as local and active, so the helper starts there without a prompt;
* that Gaming Mode without the rule makes pkexec say "No authentication agent
  found". Every pkexec the app starts carries `--disable-internal-agent`, so
  that is the line pkexec's source prints when no agent is registered; without
  the option it would try a password prompt on a terminal the app does not
  have and fail with "Error creating textual authentication agent", which the
  app reads as the same thing (`privilege::pkexec_unavailable`);
* that `/home` on SteamOS allows executing the copy (it is where AppImages
  usually run from, so it should);
* SteamOS's DNS backend (resolved, from public reports), and DNS after killing
  the helper;
* Steam Input, the on-screen keyboard and running ProxysVPN beside a game.
