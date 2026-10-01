# ProxysVPN Desktop

> 🇷🇺 [Русская версия](README_RU.md)
>
> 📱 **iPhone / iPad:** the same codebase builds an iOS app with a Network
> Extension tunnel (Xray-core engine via libXray) — see [docs/IOS.md](docs/IOS.md).

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

1. Run `ProxysVPN_<version>_x64-setup.exe`. SmartScreen says the publisher is
   unknown: **More info → Run anyway**.
2. Allow the administrator prompt (UAC). The app asks for it at every launch:
   it creates the tunnel adapter and routes.
3. Pair with a code or a QR, or paste your link.

### Linux (x86_64, .deb)

```bash
sudo apt install ./ProxysVPN_<version>_amd64.deb
```

Start **ProxysVPN** from the applications menu. The first Connect shows the
polkit password dialog (your desktop needs a polkit agent: GNOME, KDE, Xfce,
Cinnamon and MATE have one). Remove with `sudo apt remove proxys-vpn`.

## System requirements

- **macOS** 12 Monterey or newer, Apple Silicon (Intel Macs are not supported).
- **Windows** 10 or 11, x64 (tested on Windows 11). WebView2 is installed by
  the setup if missing.
- **Linux** x86_64 with glibc 2.35+ (Debian 12+, Ubuntu 22.04+; tested on
  Debian 13), a desktop session with a polkit agent, `iproute2`.

## Known limitations

- **IPv6 does not go through the tunnel** on any desktop system yet. If your
  network offers IPv6, programs that connect to an IPv6 address on their own
  (WebRTC calls in the browser, apps with their own DNS) can go outside the
  VPN. The app never hands out IPv6 addresses for names it resolves.
- **Windows: DNS is not redirected yet.** Site names are resolved by your
  network's DNS (router or provider), and those lookups can go outside the
  VPN. The app's data notice says so.
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

## Architecture

```
tun2socks (TUN device) ──socks5 127.0.0.1:10808──> xray ──vless──> node
                                                     └──socks5 127.0.0.1:10809──> hysteria ──> node (Hysteria2)
```

- **Frontend:** React 19 + TypeScript (Tauri 2 webview).
- **Core:** Rust (Tauri 2 + tokio): subscription and pairing, the repair
  ladder, the supervisor (`src-tauri/src/lib.rs`).
- **Platform layer:** `src-tauri/crates/pvpn-platform` — routes, TUN device,
  DNS and elevation per OS. Linux runs only a small root helper through
  `pkexec`; the GUI stays unprivileged. See [docs/WINDOWS.md](docs/WINDOWS.md),
  [docs/LINUX.md](docs/LINUX.md), [docs/DESIGN.md](docs/DESIGN.md).
- **Engines:** Xray-core 26.3.27, Hysteria 2.9.3, tun2socks 2.6.0, pinned with
  SHA-256 in `scripts/sidecars.lock`.

## Build from source

```bash
# Prerequisites: Rust (stable), Node.js 22+, npm
git clone https://github.com/evilork/proxysvpn-desktop.git
cd proxysvpn-desktop

bash scripts/fetch-binaries.sh      # pinned engines + geo data, checksummed
npm ci

# macOS: the .dmg with the root launcher
bash src-tauri/scripts/build-release.sh
# Windows / Linux
npx tauri build --bundles nsis      # or: --bundles deb
```

More in [docs/BUILDING.md](docs/BUILDING.md).

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

MIT — see [LICENSE](LICENSE).

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

## Links

- [proxysvpn.com](https://proxysvpn.com) — the service
- [@proxysvpn_bot](https://t.me/proxysvpn_bot) — Telegram bot for accounts and payments
- [Issues](https://github.com/evilork/proxysvpn-desktop/issues) — bug reports and feature requests
