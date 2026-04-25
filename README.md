# ProxysVPN Desktop

> 🇷🇺 [Русская версия](README_RU.md)

Native macOS VPN client for the [ProxysVPN](https://proxysvpn.com) service. Routes **all** system traffic (browser, Telegram, Discord, games, App Store) through a VLESS Reality + ML-KEM tunnel — no manual proxy config required.

## Features

- **System-wide VPN** via TUN device (`utun225`) and `tun2socks` — every TCP/UDP socket on the machine routes through the tunnel
- **VLESS Reality** with post-quantum **ML-KEM** key exchange — censorship-resistant, indistinguishable from regular HTTPS
- **One-click installer** in the DMG — no Terminal needed
- **Single admin prompt** when starting the VPN (needed to install network routes)
- **System tray** with Show / Disconnect / Quit; closing the window keeps the VPN connected in background
- **Live ping** through the tunnel, smart reconnect on network changes
- **Built-in logs viewer** — bottom-right corner, one click for diagnostics
- **Light / dark theme** following macOS preferences

## Installation

1. Download **`ProxysVPN_0.1.0_aarch64.dmg`** from the [latest release](https://github.com/evilork/proxysvpn-desktop/releases/latest)
2. Open the `.dmg` (double-click)
3. Double-click **"ProxysVPN Installer"**
4. Click **"Install"** → enter admin password if prompted
5. Done — the app launches automatically

> On the first Installer launch, macOS may say "from an unidentified developer" — click **"Open"** or go to *System Settings → Privacy & Security → Open Anyway*. One-time action.

After launch, enter your subscription URL (get one at [proxysvpn.com](https://proxysvpn.com)) and click the power button.

## System requirements

- macOS 11 Big Sur or newer
- Apple Silicon (M1/M2/M3/M4). Intel Mac support planned for v0.2.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  ProxysVPN.app                                                  │
│                                                                 │
│  ┌─────────────────┐    spawns    ┌──────────────────────────┐  │
│  │ React UI        │ ───────────> │ xray (VLESS Reality)     │  │
│  │ (Tauri 2)       │              │ + ML-KEM post-quantum    │  │
│  └─────────────────┘              └──────────────────────────┘  │
│           │                                  ▲                  │
│           │                                  │ SOCKS5 :10808    │
│           ▼                                  │                  │
│  ┌─────────────────┐                ┌──────────────────────────┐│
│  │ Rust backend    │ ─── spawns ──> │ tun2socks → utun225      ││
│  │ (lib.rs)        │                │ + split-default routes   ││
│  └─────────────────┘                └──────────────────────────┘│
│                                              │                  │
└──────────────────────────────────────────────┼──────────────────┘
                                               ▼
                                       Whole-system traffic
                                       (Telegram, games, etc.)
```

- **Frontend:** React 19 + TypeScript + Tailwind CSS + neumorphic design
- **Backend:** Rust (Tauri 2) + tokio
- **Bundled binaries:** xray-core, tun2socks (downloaded by `scripts/fetch-binaries.sh` on first build)
- **Reality crypto:** ML-KEM-768 hybrid x25519 (post-quantum)

## Build from source

```bash
# Prerequisites: Rust 1.80+, Node.js 18+, npm
git clone https://github.com/evilork/proxysvpn-desktop.git
cd proxysvpn-desktop

# Fetch xray + tun2socks binaries (~25 MB)
bash src-tauri/scripts/fetch-binaries.sh

# Install JS deps
npm install

# Build a signed .app + .dmg with embedded Installer
bash src-tauri/scripts/build-release.sh
bash src-tauri/scripts/bundle-installer-into-dmg.sh
```

The signed `.dmg` will appear in `src-tauri/target/release/bundle/dmg/`.

## Privacy

- **Zero telemetry.** The app only talks to your subscription URL and the VLESS server it returns.
- **Logs stay local.** `~/Library/Logs/ProxysVPN/app.log` is never uploaded anywhere — share it manually with support if you need to.
- **No accounts in the desktop app.** Authentication is via your subscription URL (a long, unguessable token tied to your ProxysVPN profile).

## Troubleshooting

| Problem | Fix |
|---|---|
| Installer says "from an unidentified developer" | *System Settings → Privacy & Security → Open Anyway* |
| App freezes on launch / no admin prompt | Quit fully, then `sudo pkill -9 -x proxysvpn-desktop`, relaunch |
| Connected but Telegram/Instagram won't load | DPI fragmentation issue. Open the logs viewer (bottom-right corner) and send the export to support |
| VPN drops every few minutes | Click the (i) icon in the corner → Download .txt → send to support |

## License

MIT — see [LICENSE](LICENSE).

This project bundles several open-source components, each under its own license:
- [xray-core](https://github.com/XTLS/Xray-core) — MPL-2.0
- [tun2socks](https://github.com/xjasonlyu/tun2socks) — GPL-3.0
- [Tauri](https://github.com/tauri-apps/tauri) — MIT/Apache-2.0

## Links

- [proxysvpn.com](https://proxysvpn.com) — main service
- [@proxysvpn_bot](https://t.me/proxysvpn_bot) — Telegram bot for accounts and payments
- [Issues](https://github.com/evilork/proxysvpn-desktop/issues) — bug reports and feature requests
