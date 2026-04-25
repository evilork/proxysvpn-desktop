#!/usr/bin/env bash
# src-tauri/scripts/bundle-installer-into-dmg.sh
# Repackages the existing ProxysVPN.app (already built) into a NEW DMG that
# contains both the app and a one-click "ProxysVPN Installer.app".
#
# Run AFTER the regular build:
#   bash src-tauri/scripts/build-release.sh                      # makes the .app
#   bash src-tauri/scripts/bundle-installer-into-dmg.sh          # makes the .dmg
#
# Output: src-tauri/target/release/bundle/dmg/ProxysVPN_0.1.0_aarch64.dmg
#         (overwrites the auto-generated one from tauri build)

set -euo pipefail

# ── Paths ────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BUILD_DIR="$PROJECT_ROOT/src-tauri/target/release/bundle/macos"
DMG_DIR="$PROJECT_ROOT/src-tauri/target/release/bundle/dmg"
APP_PATH="$BUILD_DIR/ProxysVPN.app"
APP_ICON="$PROJECT_ROOT/src-tauri/icons/icon.icns"

# Get version from tauri.conf.json (best-effort; fallback to 0.1.0)
VERSION=$(grep -m1 '"version"' "$PROJECT_ROOT/src-tauri/tauri.conf.json" \
    | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/' \
    || echo "0.1.0")
DMG_NAME="ProxysVPN_${VERSION}_aarch64.dmg"
DMG_PATH="$DMG_DIR/$DMG_NAME"

# ── Pre-flight ───────────────────────────────────────────────────────
[[ -d "$APP_PATH" ]] || { echo "ERROR: $APP_PATH not found. Run build-release.sh first."; exit 1; }
mkdir -p "$DMG_DIR"

echo "═══════════════════════════════════════════════════════"
echo "  Bundling installer into DMG"
echo "═══════════════════════════════════════════════════════"
echo "  app:     $APP_PATH"
echo "  output:  $DMG_PATH"
echo "  version: $VERSION"

# ── Build the installer ──────────────────────────────────────────────
STAGING="$(mktemp -d /tmp/proxysvpn-dmg-staging.XXXXXX)"
trap 'rm -rf "$STAGING"' EXIT

echo ""
echo "==> Building installer..."
APP_ICON="$APP_ICON" bash "$SCRIPT_DIR/build-installer.sh" "$STAGING"

# ── Stage DMG contents ───────────────────────────────────────────────
echo ""
echo "==> Staging DMG contents..."
ditto "$APP_PATH" "$STAGING/ProxysVPN.app"
ln -s /Applications "$STAGING/Applications"

# Create a README for users who manually browse the DMG
cat > "$STAGING/Прочти меня.txt" <<'EOF'
УСТАНОВКА PROXYSVPN

Самый простой способ:

  1. Двойной клик на «ProxysVPN Installer»
  2. Нажать «Установить»
  3. Ввести пароль администратора если попросит
  4. Готово — приложение запустится автоматически

────────────────────────────────────────

INSTALLATION (English)

The simplest way:

  1. Double-click "ProxysVPN Installer"
  2. Click "Install"
  3. Enter your admin password when asked
  4. Done — the app launches automatically
EOF

# ── Build DMG ────────────────────────────────────────────────────────
echo ""
echo "==> Creating DMG..."
rm -f "$DMG_PATH"

# Use hdiutil with UDZO compression. Volume name shows in Finder titlebar.
VOL_NAME="ProxysVPN ${VERSION}"
hdiutil create \
    -volname "$VOL_NAME" \
    -srcfolder "$STAGING" \
    -ov \
    -format UDZO \
    -fs HFS+ \
    "$DMG_PATH"

# ── Verify ───────────────────────────────────────────────────────────
SIZE=$(du -h "$DMG_PATH" | awk '{print $1}')
SHA256=$(shasum -a 256 "$DMG_PATH" | awk '{print $1}')

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  ✓ DMG built"
echo "═══════════════════════════════════════════════════════"
echo "  path:    $DMG_PATH"
echo "  size:    $SIZE"
echo "  sha256:  $SHA256"
echo ""
echo "Test by mounting it:"
echo "  open \"$DMG_PATH\""
echo ""
