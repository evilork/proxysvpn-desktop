# ProxysVPN Desktop

> 🇷🇺 [Русская версия](README_RU.md)

Desktop VPN client for the [ProxysVPN](https://proxysvpn.com) service, for
**macOS, Windows and Linux**. It routes the system's IPv4 traffic through a
VLESS (Reality / XHTTP) or Hysteria2 node, with the service's split routing:
Russian sites and anything on your "always direct" list go straight out.

## Features

- **System-wide tunnel** through a TUN device and `tun2socks` (`utun225` on
  macOS, the Wintun adapter `ProxysVPN` on Windows, `proxysvpn0` on Linux).
  IPv4 only for now: see [Known limitations](#known-limitations).
- **VLESS Reality / XHTTP and Hysteria2.** xray is always the front of the
  chain, so the split-routing rules apply on every protocol.
- **Pairing without typing a link:** an 8-character code from the cabinet or
  the bot, or a QR code scanned with the phone.
- **Repairs itself:** a dead engine is restarted, a node that stops getting
  through is swapped for another, and the window says what happened.
- **System tray** with Show / Disconnect / Quit; closing the window keeps the
  VPN connected in the background.
- **Russian and English** interface.

## Download and install

Installers are on the [Releases page](https://github.com/evilork/proxysvpn-desktop/releases).
Nothing is code-signed yet (no Apple Developer ID, no Windows certificate),
so each system warns once. Compare the installer's SHA-256 with
`SHA256SUMS.txt` on the release page before you install:

```bash
shasum -a 256 ProxysVPN_*            # macOS
sha256sum ProxysVPN_*                # Linux
certutil -hashfile ProxysVPN_<version>_x64-setup.exe SHA256   # Windows
```

### macOS (Apple Silicon)

1. Open `ProxysVPN_<version>_aarch64.dmg` and drag **ProxysVPN** onto the
   **Applications** shortcut.
2. Remove the download quarantine once, in Terminal:
   `xattr -dr com.apple.quarantine /Applications/ProxysVPN.app`
   (or launch it, press **Done**, then System Settings → Privacy & Security →
   **Open Anyway**; on macOS 15 and later right-click → Open no longer works).
3. Launch ProxysVPN and enter your administrator password: the tunnel needs it.
4. Pair with a code or a QR from the cabinet / bot, or paste your link.

Details and troubleshooting: [INSTALL.md](INSTALL.md) (Russian).

### Windows 10 / 11 (x64)

The installer is not signed with a publisher certificate yet, so Windows
warns twice: at the download and at the launch. Compare the file's SHA-256
with `SHA256SUMS.txt` on the release page if you want to be sure it is ours.

1. Download `ProxysVPN_<version>_x64-setup.exe`. Microsoft Edge may hold the
   download with "Make sure you trust the file before you open it" and show a
   "Delete" button. The way to keep the file is hidden next to it:
   **the arrow to the right of "Delete" → "Keep anyway"**.
2. Run the file. SmartScreen says the publisher is unknown:
   **More info → Run anyway**.
3. Allow the administrator prompt (UAC). The app asks for it at every launch:
   it creates the tunnel adapter and routes.
4. Pair with a code or a QR, or paste your link.

**Windows on ARM** (Snapdragon, Surface Pro X): take
`ProxysVPN_<version>_arm64-setup.exe`, the steps are the same. The x64 file
works there too, under emulation. This build has not been run on a real ARM
PC yet: it is built and checked on the build server's ARM machine.

### Linux (x86_64, .deb)

```bash
sudo apt install ./ProxysVPN_<version>_amd64.deb
```

Start **ProxysVPN** from the applications menu. The first Connect shows the
polkit password dialog (your desktop needs a polkit agent: GNOME, KDE, Xfce,
Cinnamon and MATE have one). Remove with `sudo apt remove proxys-vpn`.

### Steam Deck (AppImage)

The file is `ProxysVPN_<version>_amd64.AppImage`. Make it executable
(`chmod +x`); the first start and the first connect happen in Desktop Mode.
After that the app can be used in Gaming Mode, with the controller. Every
step, the controller layout and the list of what is not verified without the
device itself are in [docs/STEAMDECK.md](docs/STEAMDECK.md). The build has
not been run on a real Steam Deck yet.

## System requirements

- **macOS** 12 Monterey or newer, Apple Silicon (Intel Macs are not supported).
- **Windows** 10 or 11, x64 (tested on Windows 11). WebView2 is installed by
  the setup if missing.
- **Linux** x86_64 with glibc 2.35+ (Debian 12+, Ubuntu 22.04+; tested on
  Debian 13), a desktop session with a polkit agent, `iproute2`.

## Known limitations

- **IPv6 goes into the tunnel, but no server has an IPv6 exit.** Sites open
  over IPv4 through the VPN server as before. A program that connects to an
  IPv6 address it found on its own, with anything but HTTPS or HTTP (some
  games and calls), gets no connection over IPv6 while the VPN is on and has
  to fall back to IPv4 by itself. The app never hands out IPv6 addresses for
  names it resolves.
- **The IPv6 guard does not stop the connection when the system refuses it.**
  The "No leaks" row of the Check screen says whether IPv6 goes past the VPN.
  The app tries a refused guard again only in the first moments after a
  connect, and only on Windows. A guard that stayed refused is not put in
  later in that session, even when your network starts handing out IPv6 then:
  the row shows it, and reconnecting tries again.
  A more specific IPv6 route that your network announced itself wins over the
  app's two routes: the row sees the usual case (a route for all public
  addresses) and cannot see a route for a narrower range; the app repairs
  neither.
- **Windows: lookups can still go past the VPN in three cases.** While the VPN
  is on, the app refuses ordinary DNS (ports 53 and 853) on every network
  adapter but its own. Not covered: the "System" resolver chosen in Tunnel
  settings (then nothing is refused), DNS over HTTPS switched on in Windows
  itself, and name lookups on the local link (LLMNR, mDNS, NetBIOS).
- **macOS and Linux: the Check screen does not test DNS yet.** The system
  resolver points into the tunnel there; the "No leaks" row checks IPv6 only
  and says so, which is why it is never green on these two systems.
- **No code signing and no automatic updates.** Install new versions from the
  Releases page by hand.
- **macOS:** after a crash or Force Quit the engines may keep running until
  ProxysVPN is opened again; opening it cleans them up.
- **macOS:** the whole app runs as root (the price of not having a signed
  privileged helper yet). The engines it starts (xray, tun2socks, hysteria)
  and their geo files are copied to a root-owned folder at launch and run only
  from there, but the app itself (`Contents/MacOS/ProxysVPN-bin`) and its
  launcher still run as root straight from the app bundle, which a drag
  install leaves owned by your account. A program running as your user can
  therefore replace them, and the replacement runs as root at the next launch
  when you type your password. The planned fix is a `.pkg` installer with a
  small privileged helper, so that the window no longer runs as root.

## Privacy

- **No telemetry, no analytics.** The app talks to:
  - your subscription link's site and, if it is one of ours and does not
    answer, the reserve sites `proxysvpn.com`, `proxysvnovich.vercel.app`,
    `proksya.com`, `proksya.xyz` (subscription, signed server list, pairing).
    These requests carry your link's token and a random device id
    (`x-hwid`), which is how the service enforces "one link, one device";
  - the VPN nodes the subscription lists;
  - while connected, a check that the tunnel carries traffic:
    `proxysvpn.store/api/exit-ip` and the `/api/tools/whoami` page of the
    reserve sites, through the tunnel;
  - DNS: Cloudflare (DNS-over-HTTPS) and Google, asked through the VPN node
    (or the resolver you choose in Tunnel settings).
- **Logs stay on the device:** `~/Library/Logs/ProxysVPN/app.log` (macOS),
  `%LOCALAPPDATA%\ProxysVPN\logs\app.log` (Windows),
  `~/.local/state/ProxysVPN/app.log` (Linux). Node addresses, tokens and the
  names of sites you open are masked before a line is written; log files an
  older version left behind are masked the same way once, on the first start
  of a version whose masking rules changed. The "Report for support" leaves
  the device only if you send it yourself.
- **What the app remembers about a network stays on the device:** which
  location got through there and, when the signed server list says it, the
  country and the provider network (AS number) the service saw your request
  come from, so that the first server tried is one that works on such a
  network. The network itself is kept as a salted hash, never by name or
  address, and none of this is sent anywhere.
- **No accounts in the desktop app.** It is tied to your subscription link.

## Uninstall

- **macOS:** move ProxysVPN to the Bin, then
  `sudo rm -rf "/Library/Application Support/ProxysVPN"` and remove
  `~/Library/Application Support/com.proxysvpn.desktop` and
  `~/Library/Logs/ProxysVPN`.
- **Windows:** Settings → Apps → ProxysVPN → Uninstall, then delete
  `%LOCALAPPDATA%\ProxysVPN` (the uninstaller does not remove it yet: it holds
  your link, the device id and the log).
- **Linux:** `sudo apt purge proxys-vpn`, then remove
  `~/.local/share/ProxysVPN` and `~/.local/state/ProxysVPN`.

## License

This repository holds the installers, the documents for them and the notices
of the components they carry. The app's own source code is not published
here.

The installers bundle third-party components under their own licences:
Xray-core (MPL-2.0), Hysteria (MIT), tun2socks (MIT), Wintun on Windows
(WireGuard LLC prebuilt-binaries licence), Tauri (MIT/Apache-2.0) and others.
The stock Xray-core and Hysteria binaries also contain GPL-3.0-or-later Go
modules. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) and the app's
"Open-source licences" screen list, with versions and sources, the engines,
every Rust crate and npm package the app links, and those GPL modules. The
other Go modules compiled into the desktop engines (under MIT, BSD,
Apache-2.0 and similar licences) are not listed one by one yet;
`go version -m <engine binary>` prints them.

The source of our own changes to Xray-core (MPL-2.0), for the builds where we
compile the engine ourselves, is in [scripts/libxray](scripts/libxray).

## Links

- [proxysvpn.com](https://proxysvpn.com) — the service
- [@proxysvpn_bot](https://t.me/proxysvpn_bot) — Telegram bot for accounts and payments
- [Issues](https://github.com/evilork/proxysvpn-desktop/issues) — bug reports and feature requests
