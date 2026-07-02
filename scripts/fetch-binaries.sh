#!/usr/bin/env bash
# scripts/fetch-binaries.sh
# Downloads xray-core (+geo), hysteria2 and tun2socks for the host platform.
# Run once after cloning the repo, and after cleaning target/.

set -euo pipefail
cd "$(dirname "$0")/.."
BIN_DIR="src-tauri/binaries"
mkdir -p "$BIN_DIR"

OS="$(uname -s)"; ARCH="$(uname -m)"
case "$OS-$ARCH" in
  Darwin-arm64)   XRAY_ZIP="Xray-macos-arm64-v8a.zip"; HY="hysteria-darwin-arm64";  T2S="tun2socks-darwin-arm64.zip";  TRIPLE="aarch64-apple-darwin" ;;
  Darwin-x86_64)  XRAY_ZIP="Xray-macos-64.zip";        HY="hysteria-darwin-amd64";  T2S="tun2socks-darwin-amd64.zip";  TRIPLE="x86_64-apple-darwin" ;;
  Linux-x86_64)   XRAY_ZIP="Xray-linux-64.zip";        HY="hysteria-linux-amd64";   T2S="tun2socks-linux-amd64.zip";   TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  XRAY_ZIP="Xray-linux-arm64-v8a.zip"; HY="hysteria-linux-arm64";   T2S="tun2socks-linux-arm64.zip";   TRIPLE="aarch64-unknown-linux-gnu" ;;
  *) echo "Unsupported platform: $OS-$ARCH" >&2; exit 1 ;;
esac

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

echo "Downloading xray ($XRAY_ZIP)..."
curl -fsSL -o "$TMP/xray.zip" "https://github.com/XTLS/Xray-core/releases/latest/download/$XRAY_ZIP"
unzip -qo "$TMP/xray.zip" -d "$TMP"
mv "$TMP/xray" "$BIN_DIR/xray-$TRIPLE"
mv "$TMP/geoip.dat" "$BIN_DIR/geoip.dat"
mv "$TMP/geosite.dat" "$BIN_DIR/geosite.dat"

echo "Downloading hysteria ($HY)..."
curl -fsSL -o "$BIN_DIR/hysteria-$TRIPLE" "https://github.com/apernet/hysteria/releases/latest/download/$HY"

echo "Downloading tun2socks ($T2S)..."
curl -fsSL -o "$TMP/t2s.zip" "https://github.com/xjasonlyu/tun2socks/releases/latest/download/$T2S"
unzip -qo "$TMP/t2s.zip" -d "$TMP"
mv "$TMP/${T2S%.zip}" "$BIN_DIR/tun2socks-$TRIPLE"

chmod +x "$BIN_DIR/xray-$TRIPLE" "$BIN_DIR/hysteria-$TRIPLE" "$BIN_DIR/tun2socks-$TRIPLE"
echo "Done:"; ls -la "$BIN_DIR"
