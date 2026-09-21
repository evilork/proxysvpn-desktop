#!/usr/bin/env bash
# One-shot iOS build environment setup. Run once on a Mac with Xcode installed.
#
# Usage: bash scripts/setup-ios.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

fail() { echo "error: $*" >&2; exit 1; }

echo "==> Checking Xcode"
if ! xcode-select -p 2>/dev/null | grep -q "Xcode.app"; then
    fail "full Xcode is required (App Store), then: sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
fi
xcodebuild -version | head -1

echo "==> Checking Rust iOS targets"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

echo "==> Checking CocoaPods"
command -v pod >/dev/null 2>&1 || brew install cocoapods

echo "==> Checking xcodegen"
command -v xcodegen >/dev/null 2>&1 || brew install xcodegen

echo "==> Installing npm dependencies"
npm install

echo "==> Regenerating Xcode project from project.yml"
(cd src-tauri/gen/apple && xcodegen generate)

if [ ! -d src-tauri/gen/apple/Frameworks/Libbox.xcframework ]; then
    echo "==> Building Libbox.xcframework (sing-box engine for the tunnel extension)"
    bash scripts/build-libbox.sh
else
    echo "==> Libbox.xcframework already present"
fi

cat <<'EOF'

Done. Remaining manual steps (once, in Xcode):

1. Open src-tauri/gen/apple/proxysvpn-desktop.xcodeproj
2. For BOTH targets (proxysvpn-desktop_iOS and PacketTunnel):
   Signing & Capabilities → set your Team (paid Apple Developer account —
   Network Extension does not work with a free account).
3. Connect an iPhone/iPad and run:
     npm run tauri ios dev
   or build a release IPA:
     npm run tauri ios build

Note: the VPN tunnel only works on a REAL device. The iOS Simulator does not
support Network Extensions — the UI runs there, but connect will fail.
EOF
