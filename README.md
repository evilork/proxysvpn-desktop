# ProxysVPN Desktop

Secure VPN client for VLESS Reality. Built with Tauri 2 + React.

## Prerequisites

- Rust 1.70+ (`rustup`)
- Node.js 18+ (`npm`)

## First-time setup

```bash
npm install
./scripts/fetch-binaries.sh   # downloads xray-core + geoip/geosite assets
```

## Development

```bash
npm run tauri dev
```

## Build for release

```bash
npm run tauri build
```

Artifacts land in `src-tauri/target/release/bundle/`.

## Project layout

- `src/` — React UI (TypeScript + Tailwind)
- `src-tauri/src/` — Rust backend
  - `subscription.rs` — parses VLESS Reality URLs from subscription
  - `xray_manager.rs` — lifecycle of the bundled xray-core process
  - `system_proxy.rs` — toggles macOS system SOCKS proxy
  - `ping.rs` — TCP latency to the VPN server
- `src-tauri/binaries/` — xray-core + geo assets (fetched, not committed)
