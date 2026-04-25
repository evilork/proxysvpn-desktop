#!/usr/bin/env bash
# src-tauri/scripts/post-build-wrap.sh
# Run AFTER `cargo tauri build --bundles app`.
#
# What it does:
#   1. Detects the main executable name from Info.plist (CFBundleExecutable)
#   2. Renames Contents/MacOS/<EXEC>  ->  Contents/MacOS/<EXEC>-bin  (Rust binary)
#   3. Installs scripts/launcher.sh as Contents/MacOS/<EXEC>          (bash wrapper)
#   4. Re-signs all Mach-O binaries with our entitlements (ad-hoc)
#   5. Re-signs the .app as a whole (deep)
#
# Idempotent: re-running on an already-wrapped .app refreshes the launcher
# and re-signs without breaking anything.

set -euo pipefail

cd "$(dirname "$0")/.."
SRC_TAURI="$(pwd)"

APP="$SRC_TAURI/target/release/bundle/macos/ProxysVPN.app"
LAUNCHER_SRC="$SRC_TAURI/scripts/launcher.sh"
ENTITLEMENTS="$SRC_TAURI/Entitlements.plist"

[[ -d "$APP" ]]            || { echo "ERROR: $APP not found — run cargo tauri build --bundles app first"; exit 1; }
[[ -f "$LAUNCHER_SRC" ]]   || { echo "ERROR: $LAUNCHER_SRC not found"; exit 1; }
[[ -f "$ENTITLEMENTS" ]]   || { echo "ERROR: $ENTITLEMENTS not found"; exit 1; }

MACOS="$APP/Contents/MacOS"
EXEC_NAME="$(/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$APP/Contents/Info.plist")"
REAL="$MACOS/$EXEC_NAME"
WRAPPED="$MACOS/${EXEC_NAME}-bin"

echo "==> Wrap .app  ($EXEC_NAME -> ${EXEC_NAME}-bin + launcher.sh)"

# ── Idempotent rename ──────────────────────────────────────────────
if [[ -e "$REAL" && ! -e "$WRAPPED" ]]; then
    if file "$REAL" | grep -q 'Mach-O'; then
        echo "    rename: $EXEC_NAME -> ${EXEC_NAME}-bin"
        mv "$REAL" "$WRAPPED"
    else
        echo "    note: $EXEC_NAME is not Mach-O (already a launcher script?)"
        rm -f "$REAL"
    fi
elif [[ -e "$WRAPPED" ]]; then
    echo "    note: ${EXEC_NAME}-bin already present — re-applying launcher"
    [[ -e "$REAL" ]] && rm -f "$REAL"
fi

# ── Install launcher ────────────────────────────────────────────────
echo "    install launcher"
cp "$LAUNCHER_SRC" "$REAL"
chmod +x "$REAL"

# ── Re-sign each Mach-O binary explicitly with our entitlements ────
echo "==> Re-sign Mach-O binaries"
shopt -s nullglob
for bin in "$MACOS"/xray* "$MACOS"/tun2socks* "$WRAPPED"; do
    [[ -f "$bin" ]] || continue
    if file "$bin" 2>/dev/null | grep -q 'Mach-O'; then
        echo "    sign: $(basename "$bin")"
        codesign --remove-signature "$bin" 2>/dev/null || true
        codesign --force --sign - \
            --entitlements "$ENTITLEMENTS" \
            --options runtime \
            --timestamp=none \
            "$bin"
    fi
done
shopt -u nullglob

# ── Re-sign the .app as a whole (covers Resources, launcher script, etc) ──
echo "==> Re-sign .app (deep)"
codesign --remove-signature "$APP" 2>/dev/null || true
codesign --force --deep --sign - \
    --entitlements "$ENTITLEMENTS" \
    --options runtime \
    --timestamp=none \
    "$APP"

echo "==> Verify"
if codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | sed 's/^/    /'; then
    echo "    OK"
else
    echo "    WARN: verify reported issues (review above)"
fi

echo "Wrap done."
