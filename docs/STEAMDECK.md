# ProxysVPN Desktop on a Steam Deck

Status (04.10.2026): **never run on a Deck; run in a simulation of one.**
There was no Steam Deck. The real AppImage, built by CI, has been started in a
SteamOS userspace under gamescope on a GitHub runner and driven by key
presses and a virtual gamepad.
What depends on Steam itself (Steam Input, its on-screen keyboard, the
environment it starts a program in) and on the Deck's hardware is still
reasoned from documented behaviour and public reports, and listed at the
bottom as not verified. The CI job "Linux x86_64 (AppImage)" also builds the
AppImage and checks what is inside it.

The Deck build is the Linux x86_64 **AppImage**,
`ProxysVPN_<version>_amd64.AppImage`. It is not a separate app: the same
binary, the same tunnel as the `.deb`. What differs is how the
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
root can change, and runs it from there afterwards.

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

4. **First connect, in Desktop Mode.** Double-click the AppImage. Right after
   the data notice the app asks on a screen of its own, **"Gaming Mode without
   a password?"**, with two answers:

   * **"Allow Gaming Mode without a password"**: the first connect asks for
     the `deck` password once, and from then on ProxysVPN can connect in
     Gaming Mode without one; the permission stays on the Deck until it is
     removed in **More → Remove system files**;
   * **"Only with a password"**: the password is asked when you connect after
     every start of the app, and Gaming Mode cannot connect.

   Nothing is granted without the first answer: a setup that runs before the
   screen was answered (a connect from the tray menu, say) copies the helper
   and records nothing, and the screen asks again before the next connect
   from the window. Answer, sign in with the pair code, press Connect. The
   system's permission dialog appears once. It is polkit's generic one — the
   AppImage cannot install a policy file of its own — and pkexec quotes the
   command it runs, cut to its first 38 and last 37 bytes, so after "allow"
   it reads "Authentication is needed to run `/bin/sh -c # ProxysVPN sets up
   its VPN ... so Gaming Mode then needs no password' as the super user"
   (after "only with a password", on another Linux, or once Gaming Mode was
   decided: "... helper as root and starts the tunnel."). The command's first
   line says it in full. Type the `deck` password. That one dialog:

   * copies `proxysvpn-helper` and `tun2socks` out of the AppImage into
     `/home/.proxysvpn/bin/`, checking that the copies are byte for byte the
     files in the AppImage (SHA-256, checked by root on its own copy);
   * writes the answer down, SteamOS only, unless one is on this Deck
     already: for "allow" that `deck` may start that helper without a
     password (`/home/.proxysvpn/gaming-mode-user`), for "only with a
     password" the "no" (`/home/.proxysvpn/gaming-mode-off`);
   * starts the helper, and the tunnel comes up.

   After "allow" the helper then writes
   `/etc/polkit-1/rules.d/49-proxysvpn.rules` from that record, and from then
   on **no password is asked again**, in Desktop Mode or in Gaming Mode, until
   the app is updated. If the copy is already current when the screen is
   answered, the next connect runs the setup anyway, once, to write the
   answer down.

## Gaming Mode

Add the AppImage to Steam once, in Desktop Mode:

1. Open Steam (the desktop app) → **Games** → **Add a Non-Steam Game to My
   Library…**
2. **Browse…**, set the file type to **All files**, pick the AppImage, then
   **Add Selected Programs**.
3. Optional: in the shortcut's **Properties** rename it "ProxysVPN". Leave
   **Compatibility** off: it is a native Linux program, not one for Proton.

Back in Gaming Mode it is under **Library → Non-Steam**. The window asks for
the whole screen there, draws itself larger for it, and is driven with the
controller: the next section says how to set that up and what each button
does. Steam + X opens the on-screen keyboard for the pair code.

The tunnel lives as long as the app runs, exactly as on the desktop. Opening a
game through the Steam button leaves ProxysVPN running in the background;
**Exit game** on ProxysVPN disconnects.

If Gaming Mode shows **"Connect once from Desktop Mode"**
(`STEAMOS_DESKTOP_MODE_NEEDED`), the one-time setup has not been made, or the
app was updated since: Gaming Mode runs no polkit agent, so no password window
can appear there, and the copy needs one. Do step 4 above in Desktop Mode, then
come back.

## Controller

On SteamOS, and only there, the window can be driven with no mouse and no
touch: by the arrow keys, Enter or Space, and Escape. Those are the keys
Steam Input is expected to send for the D-pad, A and B when the shortcut's
layout is a keyboard one, and the ones the Deck's own layout for Desktop Mode
is expected to send (none of it was seen on a Deck: the list at the bottom).
The app also reads the controller directly through the web view's Gamepad
API, so that a gamepad layout works too. In the simulation the AppImage's web
view has that API and reads a pad shaped like the one Steam Input shows an
application; what Steam Input really hands the web view on a Deck was not
seen, so the keyboard layout is still the one to count on.

**Pick the layout, once.** In Gaming Mode open ProxysVPN's page in the
Library → the controller icon (**Controller settings**) → the name of the
current layout → **Templates** → **Web Browser**. Then **Edit Layout** and
check three bindings; change the ones that differ (the button → **Keyboard** →
the key):

| On the Deck | Has to send | In the app |
| --- | --- | --- |
| D-pad | the arrow keys | moves the focus ring to the next control that way |
| A | Enter (Return); Space works too | presses the control the ring is on |
| B | Escape; Backspace and the browser's Back key work too | closes the screen or the sheet on top; on the main screen, nothing |

The left stick can be bound as a second D-pad (its **Behavior** → **Directional
Pad**, same four keys). A template that keeps the right trackpad as a mouse
and R2 as the click stays useful: the pointer and the touch screen work as
before, next to the controller.

If the ring moves with the default (gamepad) layout already, the web view has
the Gamepad API and nothing needs choosing: D-pad or left stick, A and B then
mean the same as in the table. The first button press after the app starts
does nothing, though: WebKit shows a page a pad only after the pad's first
input, and the app starts reading it then, so the second press is the first
that acts. A layout that sends a button both ways, as a key and as a pad
button, is harmless: the same press arriving twice within a moment counts
once.

**What the ring does.**

* It is a double ring drawn inside the control, the text's colour over the
  background's, and it shows while the controller is what was used last.
  After a tap or a click the window looks as it does everywhere else; the
  next press of the D-pad brings the ring back where it was.
* It starts on the screen's main action: "Turn on" on the main screen,
  "Continue" on the data notice, the chosen country in Countries, "Cancel" in
  a confirmation. Never on an action that removes something. Two screens
  start with no ring at all, and the first press there only shows it: the
  **Gaming Mode question**, because one of its two answers grants a standing
  permission, and the **recovery sheet**, because it comes up by itself and
  its one button turns protection off.
* Up and down go row by row and stop at the first and the last control: no
  wrapping around. Left and right move only between controls standing side by
  side (the two buttons at the bottom of the main screen, the options of a
  switch like "Language").
* A list scrolls under the ring. A long text (the data notice, a licence) is
  read with the same keys: where nothing is left to move to, Down and Up
  scroll it a step at a time. On the data notice the ring stays on "Continue"
  while Down reads the notice to its end.
* B closes one layer per press, and the ring returns to the control that
  opened it: back from Licences to the "Licences" row of More, from More to
  the gear.
* One press is one press: a button held down, pressed again within a third of
  a second, or pressed in the first third of a second of a new screen, does
  nothing the second time. "Turn on" cannot turn into "Cancel" under a
  bouncing thumb. A screen that nobody asked for (the recovery sheet, the
  failure screen that follows it) ignores A and B for a little longer,
  0.6 s: the press on its way was meant for what was there before.
* In a text field Left and Right move the caret, Up and Down leave the field
  (in a field of several lines, once the caret is at its first or last
  position). Typing needs the on-screen keyboard: **Steam + X** (the
  pair-code field says so). A focused field moves to the top of the screen,
  so that a keyboard covering the lower half does not cover it.

**How large it is.** In Gaming Mode the app makes its window the size of the
screen and sets the fullscreen state: gamescope shows a window as large as the window
makes itself, and the desktop's 480x720 would be a narrow, blurred column.
The text then follows the window: the base size is 3 % of the window's height, at most a
thirtieth of its width, never under the desktop's 16 px and never over 32. At
1280x800 that is 24 px, one and a half times the desktop size, in a 720 px
column; the smallest lines, 13 px on the desktop with lowercase letters under
7 px tall, come out with 10 px lowercase. Buttons, rows and icons grow with
the text, the gaps between them do not: 800 px at that size holds what a
533 px window holds, and where the main screen still does not fit, the emblem
shrinks to make room. In Desktop Mode the window is the desktop's 480x720 at
16 px, the same as on any Linux; made wider (up to 720 px) it grows the text
with it. (Seen in the simulation: under gamescope the window took 1280x800 on
a 1280x800 screen and 1920x1080 on a 1920x1080 one, with a root font of 24 px
and 32 px; told it is not in Gaming Mode it stayed 480x720 at 16 px.)

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
| `/home/.proxysvpn/gaming-mode-user` | root:root 0644 | SteamOS only, after "allow": the user name the rule is for |
| `/home/.proxysvpn/gaming-mode-off` | root:root 0644 | SteamOS only, after "only with a password" or if you switched Gaming Mode off: the "no" that keeps the record from coming back |
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
traffic — sealed with a key the caller names — to a listener on a loopback
port of the app's window (20000–29999) that belongs to root or to that same
user, pin a host route to one unicast IPv4 address, publish resolvers on our
device, and take it all down. It executes nothing but
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
the answer the screen gets next):

```bash
sudo sh -c 'touch /home/.proxysvpn/gaming-mode-off && rm -f /home/.proxysvpn/gaming-mode-user /etc/polkit-1/rules.d/49-proxysvpn.rules'
```

Every later connect then asks for the password, Gaming Mode cannot connect,
and an app update does not switch it back on. Only after `/home/.proxysvpn`
is removed — More → Remove system files, which also forgets the answer the
app holds for its current run — does the screen ask again, and only an
"allow" there writes the record again. The choice is kept in those two files
and, until a setup has written it, in the running app's memory, nowhere
else: a first setup that was cut short (the power went, the password window
was killed) has decided nothing, and the next one writes the answer as the
first would have.

## DNS on SteamOS

Public reports describe SteamOS as running systemd-resolved, with
NetworkManager handing it the network's servers
(`/etc/NetworkManager/conf.d` sets `dns=systemd-resolved`). The helper picks
its DNS backend at connect time: with
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
   confirmation, behind one password window ("Authentication is needed to
   run `/bin/sh -c # ProxysVPN: remove helper, ... and the Gaming Mode
   no-password rule.'"), the app removes `/home/.proxysvpn` and
   `/etc/polkit-1/rules.d/49-proxysvpn.rules`, and only then disconnects and
   stops its root helper, which ran on from the removed folder (Linux lets a
   running program's file go). A cancelled window leaves the tunnel as it
   was. A fixed script does the removal (`REMOVE_SCRIPT` in
   `helper/install.rs`): it removes those two paths and nothing else, refuses
   any other name, removes a link in their place instead of following it,
   never follows a link inside the folder, and leaves the folder alone if it
   is not root's alone (the setup never makes it so). In Gaming Mode there is
   no password window: the app says to remove the files from Desktop Mode
   and touches nothing, the tunnel included. The row exists in the AppImage
   only; the `.deb` is removed with `apt remove`. If you keep the AppImage,
   its next connect sets everything up again, with the screen and the
   password.
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
* **The controller does nothing in Gaming Mode**: the shortcut's layout sends
  gamepad buttons and the web view has no Gamepad API. Pick the keyboard
  layout as in "Controller" above. The right trackpad with R2, and the touch
  screen, work in any layout that has a mouse.
* **A small window in the middle of the screen in Gaming Mode**: the window
  did not get the whole screen. The log says "Gaming Mode: the window fills
  the screen" when the app asked for it; if the line is missing, the app did
  not recognise Gaming Mode (it looks for `XDG_CURRENT_DESKTOP=gamescope` or
  `GAMESCOPE_WAYLAND_DISPLAY`). The window still works as it is, at the
  desktop's size: under gamescope that is a column 533 px wide in the middle
  of the screen with black on both sides (the simulation's control run shows
  exactly that picture).
* **The on-screen keyboard does not come up with Steam + X**: a 2023 report
  about non-Steam programs says the Quick Access menu (the ⋯ button) opens it
  when the shortcut does not.

## Not verified on a real Deck

The AppImage has been run in a simulation only (04.10.2026): the userspace of
SteamOS 3.7, 3.8 and 3.9 under gamescope, without Steam and without a desktop
session, driven by key presses and a virtual gamepad. There it started, took
the whole screen in Gaming Mode, and the arrow keys, Enter, Escape and a pad
moved the focus and pressed on the screens that need no account. The release
notes say which release the simulation was last repeated for.

None of the following has run on a Deck. The simulation could not reach the
first group at all (it has no desktop session, no polkit agent, no Steam), and
the second only in part, as each item says:

* the AppImage's window in Desktop Mode (KWin, a panel, a title bar): the
  simulation has no desktop session;
* the KDE polkit agent in Desktop Mode with the generic `/bin/sh` action, with
  and without a `deck` password, and the exact text it shows (taken from
  pkexec's source, `cmdline_short`, and pinned by a unit test);
* that polkit on SteamOS loads the rule and that Gaming Mode's session counts
  as local and active, so the helper starts there without a prompt;
* that Gaming Mode without the rule makes pkexec say "No authentication agent
  found". Steam starts the app without a terminal, and every pkexec the app
  starts without one carries `--disable-internal-agent`, so that is the line
  pkexec's source prints when no agent is registered; were the option missing,
  it would try a password prompt on a terminal the app does not have and fail
  with "Error creating textual authentication agent", which the app reads as
  the same thing. Started from Konsole, the
  app leaves the option out and pkexec may ask on that terminal when no agent
  answers;
* that `/home` on SteamOS allows executing the copy (it is where AppImages
  usually run from, so it should);
* SteamOS's DNS backend (resolved, from public reports), and DNS after killing
  the helper;
* running ProxysVPN beside a game.

The controller. What the simulation settled is above; the rest is assumption
until a Deck says otherwise:

* **Steam Input.** Which templates Steam offers a non-Steam shortcut, what
  "Web Browser" binds the D-pad, A and B to, where its menus are and what
  they are called (written above from memory of the Deck's interface, not
  from a Deck in hand); that those bindings reach WebKitGTK as the keys
  `ArrowUp/Down/Left/Right`, `Enter` or a space, and `Escape`; that a held
  button arrives as a repeating key; that Desktop Mode's own layout sends the
  same keys;
* **the Gamepad API, on a Deck.** The simulation's pad has the ids the SDL
  controller database lists for Steam's virtual pad; that this is what the web
  view is shown on a Deck, with which layout, and whether the Deck's own
  controller is shown when no layout hides it, was not seen;
* **the window in Gaming Mode, with Steam.** gamescope was run without `-e`
  (Steam integration) and without Steam: there the window is told apart as a
  game by the id Steam gave it, and Steam can force windows to fullscreen
  itself; that a non-Steam shortcut's window is shown the same way as here
  is assumed. If the real session starts gamescope with `--expose-wayland`,
  GTK would take Wayland, not X11 (the AppImage's GTK hook does not force X11):
  the simulation does not start it that way, and what the window does then
  was not seen;
* **the on-screen keyboard.** That Steam + X opens it over a non-Steam window
  in Gaming Mode (a 2023 report says it opened the desktop one and could
  hang; the Quick Access menu was the way around), that it covers the lower
  half of the screen and does not move the window, and that what it types
  reaches the focused field. Nothing in the app opens the keyboard by itself;
  `steam://open/keyboard` is said to, and is not used: it could not be tried;
* **how it looks on the Deck.** Seen in the simulation, in the AppImage's own
  WebKitGTK, at 1280x800 and 1920x1080 and drawn in software; not looked at on
  a seven-inch screen at arm's length, not with a real GPU's drawing, and not
  by touch in Gaming Mode.

What the simulation cannot show, and only a real Deck can: Steam Input's
layouts and what they really send; Steam's on-screen keyboard (Steam + X) and
the input-method module Steam's session sets for GTK; touch; the seven-inch
screen; the password-less helper rule in the real Gaming Mode (polkit, the
session being local and active, no agent); the environment Steam starts a
program in; the hardware and the kernel; a link pressed in Gaming Mode (the
simulation has no browser, and the app showed no reaction to "Privacy Policy":
whether a Deck's session opens one was not seen).
