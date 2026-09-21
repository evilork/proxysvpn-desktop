#!/usr/bin/env bash
# Builds Libbox.xcframework (sing-box mobile library) for the iOS
# PacketTunnel extension and installs it into src-tauri/gen/apple/Frameworks/.
#
# Requirements:
#   • Xcode (full install, not just Command Line Tools)
#   • Go 1.23+
#
# Usage:
#   bash scripts/build-libbox.sh
#   SING_BOX_VERSION=v1.12.4 bash scripts/build-libbox.sh

set -euo pipefail

SING_BOX_VERSION="${SING_BOX_VERSION:-v1.12.4}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="$REPO_ROOT/src-tauri/gen/apple/Frameworks"

if ! command -v go >/dev/null 2>&1; then
    echo "error: Go is required (brew install go)" >&2
    exit 1
fi
if ! xcode-select -p 2>/dev/null | grep -q "Xcode.app"; then
    echo "error: full Xcode is required to build an iOS xcframework." >&2
    echo "       Install Xcode from the App Store, then run:" >&2
    echo "       sudo xcode-select -s /Applications/Xcode.app/Contents/Developer" >&2
    exit 1
fi

WORK_DIR="$(mktemp -d /tmp/libbox-build.XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "==> Cloning sing-box $SING_BOX_VERSION"
git clone --depth 1 --branch "$SING_BOX_VERSION" \
    https://github.com/SagerNet/sing-box.git "$WORK_DIR/sing-box"
cd "$WORK_DIR/sing-box"

echo "==> Installing gomobile toolchain"
make lib_install

export PATH="$PATH:$(go env GOPATH)/bin"

echo "==> Building Libbox.xcframework (this takes a while)"
# Newer sing-box exposes -platform to build iOS only; fall back to the
# all-Apple-platforms build if the flag is not recognized.
if ! go run ./cmd/internal/build_libbox -target apple -platform ios; then
    echo "==> '-platform ios' not supported by this version, building all apple targets"
    go run ./cmd/internal/build_libbox -target apple
fi

if [ ! -d Libbox.xcframework ]; then
    echo "error: Libbox.xcframework was not produced" >&2
    exit 1
fi

mkdir -p "$DEST_DIR"
rm -rf "$DEST_DIR/Libbox.xcframework"
cp -R Libbox.xcframework "$DEST_DIR/"

echo "==> Installed: $DEST_DIR/Libbox.xcframework"
