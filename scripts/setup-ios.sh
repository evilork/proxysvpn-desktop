#!/usr/bin/env bash
# iOS build environment setup. Run on a Mac with Xcode installed: once on a
# fresh clone, and again after pulling changes to project.yml.
#
# Usage:
#   bash scripts/build-libxray.sh                # once, the tunnel engine
#   PVPN_TEAM_ID=<team id> bash scripts/setup-ios.sh
#
# PVPN_TEAM_ID is the paid Apple Developer team (developer.apple.com ->
# Membership details). It is written into both targets, which sign
# automatically with it. Without it the project builds for the simulator only.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

APPLE_DIR="src-tauri/gen/apple"

fail() { echo "error: $*" >&2; exit 1; }

echo "==> Checking Xcode"
if ! xcode-select -p 2>/dev/null | grep -q "Xcode.app"; then
    fail "full Xcode is required (App Store), then: sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
fi
# `sed -n 1p`, not `head -1`: head exits after one line, xcodebuild dies of
# SIGPIPE writing the second, and pipefail + set -e end the script (exit 141).
xcodebuild -version | sed -n 1p

echo "==> Checking Rust iOS targets"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

echo "==> Checking CocoaPods"
command -v pod >/dev/null 2>&1 || brew install cocoapods

echo "==> Checking xcodegen"
command -v xcodegen >/dev/null 2>&1 || brew install xcodegen

echo "==> Checking npm dependencies"
# `npm ci` installs exactly package-lock.json; `npm install` could rewrite it.
if [ -d node_modules ]; then
    echo "node_modules present (after package-lock.json changes run: npm ci)"
else
    npm ci
fi

echo "==> Checking the bundle id and version against the Tauri config"
# `tauri ios build` stamps the app target with the identifier and version from
# the Tauri config, while the tunnel extension keeps what project.yml says. A
# difference fails late (Xcode refuses an extension whose id is not prefixed
# by the app's; App Store Connect flags differing versions), so refuse here.
TAURI_INFO="$(node -e '
    const fs = require("fs");
    const read = (p) => (fs.existsSync(p) ? JSON.parse(fs.readFileSync(p, "utf8")) : {});
    const base = read("src-tauri/tauri.conf.json");
    const ios = read("src-tauri/tauri.ios.conf.json");
    const id = ios.identifier ?? base.identifier ?? "";
    let version = ios.version ?? base.version ?? "";
    // "version" may point at a package.json instead of holding the value.
    if (version.endsWith(".json")) version = read("src-tauri/" + version).version ?? "";
    // CFBundleShortVersionString has no pre-release or build part; Tauri strips it too.
    console.log(id || "-", version.replace(/[-+].*$/, "") || "-");
')" || fail "cannot read src-tauri/tauri.conf.json / tauri.ios.conf.json"
read -r TAURI_ID TAURI_VERSION <<<"$TAURI_INFO"
yml_value() {
    sed -n "s/^[[:space:]]*$1:[[:space:]]*\([^[:space:]#]*\).*/\1/p" "$APPLE_DIR/project.yml" | sed -n 1p
}
YML_ID="$(yml_value PVPN_BUNDLE_ID)"
YML_VERSION="$(yml_value MARKETING_VERSION)"
[ "$YML_ID" = "$TAURI_ID" ] || fail "bundle id differs: project.yml PVPN_BUNDLE_ID=$YML_ID, Tauri config identifier=$TAURI_ID. Set the same value in both (for iOS only: \"identifier\" in src-tauri/tauri.ios.conf.json)."
[ "$YML_VERSION" = "$TAURI_VERSION" ] || fail "version differs: project.yml MARKETING_VERSION=$YML_VERSION, tauri.conf.json version=$TAURI_VERSION. Bump MARKETING_VERSION in $APPLE_DIR/project.yml."
echo "bundle id $YML_ID, version $YML_VERSION"

echo "==> Generating the Xcode project from project.yml"
# xcodegen expands ${PVPN_TEAM_ID} in project.yml; exporting it even when
# empty keeps an unexpanded placeholder out of the project.
export PVPN_TEAM_ID="${PVPN_TEAM_ID:-}"
if [ -z "$PVPN_TEAM_ID" ]; then
    echo "warning: PVPN_TEAM_ID is not set - simulator builds only (see the header of this script)" >&2
fi
if [ ! -f "$APPLE_DIR/Sources/proxysvpn-desktop/main.mm" ]; then
    # Fresh clone: git holds only the hand-written parts of gen/apple. `tauri
    # ios init` adds Tauri's scaffolding (main.mm, bindings, launch screen,
    # asset catalog root) and never overwrites a file that exists, so
    # project.yml, the icon and our sources stay as they are.
    # `npm run` rather than npx: it uses the CLI from node_modules and never
    # fetches an unrelated package called "tauri" from the registry.
    echo "Tauri scaffolding missing, running tauri ios init"
    npm run tauri -- ios init --ci --skip-targets-install
fi
(cd "$APPLE_DIR" && xcodegen generate)

[ -f "$APPLE_DIR/Sources/proxysvpn-desktop/VpnBridge.swift" ] \
    || fail "$APPLE_DIR/Sources/proxysvpn-desktop/VpnBridge.swift is missing - the app cannot drive the tunnel without it. It is tracked in git: git checkout -- $APPLE_DIR/Sources/proxysvpn-desktop/VpnBridge.swift"
[ -d "$APPLE_DIR/Frameworks/LibXray.xcframework" ] \
    || fail "$APPLE_DIR/Frameworks/LibXray.xcframework is missing - the PacketTunnel extension cannot link without it. Build it first: bash scripts/build-libxray.sh"
if [ -d "$APPLE_DIR/Frameworks/Libbox.xcframework" ]; then
    # Left over from the sing-box engine (before 28.09.2026). Nothing links it
    # any more; it only takes space and invites linking a second Go runtime.
    echo "note: $APPLE_DIR/Frameworks/Libbox.xcframework is unused now and can be deleted" >&2
fi

cat <<'EOF'

Done. Next steps:

  Run on a connected iPhone/iPad (PVPN_TEAM_ID set above) or a simulator,
  picked from the list (the simulator has no Network Extension - UI only):
    npm run ios:dev
  App Store Connect upload: npm run ios:build with a new build number, then
  check the extension's entitlements in the IPA - both in docs/IOS.md.
EOF
