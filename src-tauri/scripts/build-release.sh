#!/usr/bin/env bash
# src-tauri/scripts/build-release.sh
# Full release build pipeline:
#   1. Sign sidecar binaries (xray, tun2socks)
#   2. cargo tauri build --bundles app  (Rust + .app + initial sign)
#   3. post-build-wrap.sh                (install launcher, re-sign)
#   4. hdiutil to package wrapped .app   (skip Tauri's dmg step which would
#      re-bundle from un-wrapped .app under some conditions)
#
# Run from project root:
#   bash src-tauri/scripts/build-release.sh

set -euo pipefail

cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
SRC_TAURI="$ROOT/src-tauri"

step()  { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
ok()    { printf '\033[1;32m    OK: %s\033[0m\n' "$*"; }
warn()  { printf '\033[1;33m    WARN: %s\033[0m\n' "$*"; }

step "Pre-flight"
[[ "$(uname)" == "Darwin" ]]                      || { echo "macOS only"; exit 1; }
[[ -f "$SRC_TAURI/tauri.conf.json" ]]             || { echo "tauri.conf.json missing"; exit 1; }
[[ -f "$SRC_TAURI/Entitlements.plist" ]]          || { echo "Entitlements.plist missing"; exit 1; }
[[ -f "$SRC_TAURI/scripts/launcher.sh" ]]         || { echo "launcher.sh missing"; exit 1; }
[[ -f "$SRC_TAURI/scripts/sign-binaries.sh" ]]    || { echo "sign-binaries.sh missing"; exit 1; }
[[ -f "$SRC_TAURI/scripts/post-build-wrap.sh" ]]  || { echo "post-build-wrap.sh missing"; exit 1; }
command -v codesign >/dev/null                     || { echo "codesign not in PATH"; exit 1; }
command -v hdiutil >/dev/null                      || { echo "hdiutil not in PATH"; exit 1; }
command -v npx >/dev/null                          || { echo "npx not in PATH"; exit 1; }
ok "all good"

step "Sign sidecar binaries"
bash "$SRC_TAURI/scripts/sign-binaries.sh"

step "Build .app  (cargo tauri build --bundles app)"
cd "$ROOT"
npx tauri build --bundles app

step "Wrap .app with launcher and re-sign"
bash "$SRC_TAURI/scripts/post-build-wrap.sh"

# ── Build .dmg from wrapped .app ────────────────────────────────────
APP="$SRC_TAURI/target/release/bundle/macos/ProxysVPN.app"
DMG_DIR="$SRC_TAURI/target/release/bundle/dmg"

VERSION="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$APP/Contents/Info.plist")"
case "$(uname -m)" in
    arm64|aarch64) ARCH="aarch64" ;;
    x86_64)        ARCH="x64" ;;
    *)             ARCH="$(uname -m)" ;;
esac
DMG_OUT="$DMG_DIR/ProxysVPN_${VERSION}_${ARCH}.dmg"

step "Package wrapped .app into .dmg  (hdiutil)"
mkdir -p "$DMG_DIR"
rm -f "$DMG_OUT"
hdiutil create \
    -volname "ProxysVPN" \
    -srcfolder "$APP" \
    -ov \
    -format UDZO \
    "$DMG_OUT" >/dev/null
ok "$(basename "$DMG_OUT")"

# ── Final report ───────────────────────────────────────────────────
SIZE="$(du -h "$DMG_OUT" | cut -f1)"
SHA="$(shasum -a 256 "$DMG_OUT" | cut -d' ' -f1)"

cat <<EOF


  Path:   $DMG_OUT
  Size:   $SIZE
  SHA256: $SHA

Distribution checklist:
  1. Upload .dmg to your CDN / GitHub Release
  2. Publish SHA256 next to the download link
  3. Link to INSTALL.md so users know to run xattr -cr after first download

Test locally:
  rm -rf /Applications/ProxysVPN.app
  open "$DMG_OUT"
  # drag to Applications
  xattr -cr /Applications/ProxysVPN.app
  open /Applications/ProxysVPN.app

  # → Should show password prompt → enter password → app launches as root
EOF
