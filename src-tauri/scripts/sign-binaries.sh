#!/usr/bin/env bash
# src-tauri/scripts/sign-binaries.sh
# Strips existing signatures from sidecar binaries (xray, tun2socks)
# and re-signs them ad-hoc with our entitlements.
#
# Run this BEFORE `cargo tauri build`. Tauri then bundles the signed
# binaries into the .app and re-signs the .app as a whole.

set -euo pipefail

# Run from src-tauri/ regardless of cwd
cd "$(dirname "$0")/.."

BIN_DIR="binaries"
ENTITLEMENTS="Entitlements.plist"

if [[ ! -d "$BIN_DIR" ]]; then
    echo "ERROR: $BIN_DIR not found. Run from src-tauri/ or via build-release.sh"
    exit 1
fi
if [[ ! -f "$ENTITLEMENTS" ]]; then
    echo "ERROR: $ENTITLEMENTS not found in $(pwd)"
    exit 1
fi

echo "==> Signing sidecar binaries (ad-hoc)"

shopt -s nullglob
binaries=("$BIN_DIR"/xray* "$BIN_DIR"/tun2socks*)
shopt -u nullglob

if (( ${#binaries[@]} == 0 )); then
    echo "WARN: no xray/tun2socks binaries found in $BIN_DIR/"
    exit 0
fi

for bin in "${binaries[@]}"; do
    # Skip data files and non-executables
    case "$bin" in
        *.dat|*.json|*.md|*.txt) continue ;;
    esac
    [[ -f "$bin" ]] || continue
    [[ -x "$bin" ]] || continue

    # Skip files that are not Mach-O (e.g. accidentally placed scripts)
    if ! file "$bin" 2>/dev/null | grep -q 'Mach-O'; then
        echo "  skip (not Mach-O): $bin"
        continue
    fi

    echo "  sign: $bin"
    codesign --remove-signature "$bin" 2>/dev/null || true
    codesign --force \
        --sign - \
        --entitlements "$ENTITLEMENTS" \
        --options runtime \
        --timestamp=none \
        "$bin"
    codesign --verify --verbose=2 "$bin" 2>&1 | sed 's/^/    /'
done

echo "Done."
