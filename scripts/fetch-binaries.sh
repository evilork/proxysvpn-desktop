#!/usr/bin/env bash
# scripts/fetch-binaries.sh
# Downloads xray-core + geoip/geosite assets for the host platform.
# Run once after cloning the repo, and after cleaning target/.

set -euo pipefail

cd "$(dirname "$0")/.."
BIN_DIR="src-tauri/binaries"
mkdir -p "$BIN_DIR"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS-$ARCH" in
  Darwin-arm64)   ZIP="Xray-macos-arm64-v8a.zip"; TRIPLE="aarch64-apple-darwin" ;;
  Darwin-x86_64)  ZIP="Xray-macos-64.zip";        TRIPLE="x86_64-apple-darwin" ;;
  Linux-x86_64)   ZIP="Xray-linux-64.zip";        TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64)  ZIP="Xray-linux-arm64-v8a.zip"; TRIPLE="aarch64-unknown-linux-gnu" ;;
  *) echo "Unsupported platform: $OS-$ARCH" >&2; exit 1 ;;
esac

URL="https://github.com/XTLS/Xray-core/releases/latest/download/$ZIP"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $URL ..."
curl -fsSL -o "$TMP/xray.zip" "$URL"

echo "Extracting..."
unzip -qo "$TMP/xray.zip" -d "$TMP"

mv "$TMP/xray" "$BIN_DIR/xray-$TRIPLE"
chmod +x "$BIN_DIR/xray-$TRIPLE"
mv "$TMP/geoip.dat"   "$BIN_DIR/geoip.dat"
mv "$TMP/geosite.dat" "$BIN_DIR/geosite.dat"

echo "Done:"
ls -la "$BIN_DIR"
