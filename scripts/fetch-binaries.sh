#!/usr/bin/env bash
# scripts/fetch-binaries.sh
#
# Downloads the sidecars Tauri bundles as externalBin — xray (+ geoip/geosite),
# tun2socks and hysteria — plus wintun.dll for Windows targets.
#
# Versions and SHA-256 sums come from scripts/sidecars.lock, so the same commit
# always produces the same bundle and a corrupted or substituted download fails
# instead of shipping.
#
# Usage:
#   scripts/fetch-binaries.sh                              # host platform
#   scripts/fetch-binaries.sh --target x86_64-pc-windows-msvc
#
# Runs on macOS, Linux and Git Bash on Windows.

set -euo pipefail
cd "$(dirname "$0")/.."

BIN_DIR="src-tauri/binaries"
LOCK="scripts/sidecars.lock"
mkdir -p "$BIN_DIR"

TARGET=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="${2:?--target needs a triple}"; shift 2 ;;
    --target=*) TARGET="${1#*=}"; shift ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------- host/target

detect_target() {
  local os arch
  case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    MINGW*|MSYS*|CYGWIN*) os="pc-windows-msvc" ;;
    *) echo "unsupported host OS: $(uname -s)" >&2; exit 1 ;;
  esac
  case "$(uname -m)" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64) arch="x86_64" ;;
    *) echo "unsupported host arch: $(uname -m)" >&2; exit 1 ;;
  esac
  echo "${arch}-${os}"
}

[ -n "$TARGET" ] || TARGET="$(detect_target)"

# Platform key used in the lock file, plus the executable suffix.
case "$TARGET" in
  aarch64-apple-darwin)       PLATFORM="darwin-arm64";  EXE="" ;;
  x86_64-apple-darwin)        PLATFORM="darwin-amd64";  EXE="" ;;
  x86_64-pc-windows-msvc)     PLATFORM="windows-amd64"; EXE=".exe" ;;
  aarch64-pc-windows-msvc)    PLATFORM="windows-arm64"; EXE=".exe" ;;
  x86_64-unknown-linux-gnu)   PLATFORM="linux-amd64";   EXE="" ;;
  aarch64-unknown-linux-gnu)  PLATFORM="linux-arm64";   EXE="" ;;
  *) echo "unsupported target: $TARGET" >&2; exit 1 ;;
esac

echo "Target:   $TARGET"
echo "Platform: $PLATFORM"

# ------------------------------------------------------------------- helpers

lock_field() { # <tool> <platform> <column 3|4|5>
  awk -v t="$1" -v p="$2" -v c="$3" '
    /^[[:space:]]*#/ { next }
    NF >= 5 && $1 == t && $2 == p { print $c; found = 1; exit }
    END { if (!found) exit 1 }
  ' "$LOCK"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "need sha256sum or shasum" >&2
    exit 1
  fi
}

verify() { # <file> <expected sha256> <label>
  local actual
  actual="$(sha256_of "$1")"
  if [ "$actual" != "$2" ]; then
    echo "SHA-256 mismatch for $3" >&2
    echo "  expected: $2" >&2
    echo "  actual:   $actual" >&2
    echo "Refusing to install. Update scripts/sidecars.lock only on purpose." >&2
    exit 1
  fi
  echo "  sha256 ok"
}

# unzip is missing from Git for Windows; bsdtar ships with Windows 10+ and
# macOS and reads zip archives. GNU tar does not, hence the ordering.
extract_zip() { # <absolute archive path> <dest dir>
  if command -v unzip >/dev/null 2>&1; then
    unzip -qo "$1" -d "$2"
  elif tar --version >/dev/null 2>&1; then
    ( cd "$2" && tar -xf "$1" )
  else
    echo "need unzip or tar to extract $1" >&2
    exit 1
  fi
}

download() { # <url> <dest>
  echo "  GET $1"
  curl -fsSL --retry 3 --retry-delay 2 -o "$2" "$1"
}

urlencode_tag() { printf '%s' "$1" | sed 's|/|%2F|g'; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------- xray

fetch_xray() {
  local tag asset sha
  tag="$(lock_field xray "$PLATFORM" 3)"
  asset="$(lock_field xray "$PLATFORM" 4)"
  sha="$(lock_field xray "$PLATFORM" 5)"
  echo "xray $tag ($asset)"
  download "https://github.com/XTLS/Xray-core/releases/download/${tag}/${asset}" "$TMP/xray.zip"
  verify "$TMP/xray.zip" "$sha" "$asset"

  mkdir -p "$TMP/xray"
  extract_zip "$TMP/xray.zip" "$TMP/xray"
  mv "$TMP/xray/xray${EXE}" "$BIN_DIR/xray-${TARGET}${EXE}"
  # geoip/geosite ship inside the same archive and go to resources, not bin.
  mv "$TMP/xray/geoip.dat" "$BIN_DIR/geoip.dat"
  mv "$TMP/xray/geosite.dat" "$BIN_DIR/geosite.dat"
}

# ----------------------------------------------------------------- tun2socks

fetch_tun2socks() {
  local tag asset sha inner
  tag="$(lock_field tun2socks "$PLATFORM" 3)"
  asset="$(lock_field tun2socks "$PLATFORM" 4)"
  sha="$(lock_field tun2socks "$PLATFORM" 5)"
  echo "tun2socks $tag ($asset)"
  download "https://github.com/xjasonlyu/tun2socks/releases/download/${tag}/${asset}" "$TMP/t2s.zip"
  verify "$TMP/t2s.zip" "$sha" "$asset"

  mkdir -p "$TMP/t2s"
  extract_zip "$TMP/t2s.zip" "$TMP/t2s"
  # The archive holds a single file named after the archive itself, with .exe
  # appended on Windows. Checked against v2.6.0 for darwin/linux/windows.
  inner="${asset%.zip}${EXE}"
  if [ ! -f "$TMP/t2s/$inner" ]; then
    echo "unexpected archive layout in $asset:" >&2
    ls -la "$TMP/t2s" >&2
    exit 1
  fi
  mv "$TMP/t2s/$inner" "$BIN_DIR/tun2socks-${TARGET}${EXE}"
}

# ------------------------------------------------------------------ hysteria

fetch_hysteria() {
  local tag asset sha
  tag="$(lock_field hysteria "$PLATFORM" 3)"
  asset="$(lock_field hysteria "$PLATFORM" 4)"
  sha="$(lock_field hysteria "$PLATFORM" 5)"
  echo "hysteria $tag ($asset)"
  # The hysteria tag contains a slash (app/v2.9.3) and must be escaped.
  download \
    "https://github.com/apernet/hysteria/releases/download/$(urlencode_tag "$tag")/${asset}" \
    "$TMP/hysteria"
  verify "$TMP/hysteria" "$sha" "$asset"
  mv "$TMP/hysteria" "$BIN_DIR/hysteria-${TARGET}${EXE}"
}

# -------------------------------------------------------------------- wintun

# Wintun is the TUN driver tun2socks uses on Windows. The DLL must sit next to
# tun2socks.exe (Windows searches the loading executable's directory first), so
# it is bundled through tauri.windows.conf.json as a resource mapped to the
# install root. LICENSE.txt travels with it: the prebuilt-binary licence permits
# redistribution alongside software that uses the published API — which is what
# tun2socks does — and forbids stripping the notices.
fetch_wintun() {
  local tag asset sha arch
  case "$PLATFORM" in
    windows-amd64) arch="amd64" ;;
    windows-arm64) arch="arm64" ;;
    *) return 0 ;;
  esac
  tag="$(lock_field wintun any 3)"
  asset="$(lock_field wintun any 4)"
  sha="$(lock_field wintun any 5)"
  echo "wintun $tag ($asset, $arch)"
  download "https://www.wintun.net/builds/${asset}" "$TMP/wintun.zip"
  verify "$TMP/wintun.zip" "$sha" "$asset"

  mkdir -p "$TMP/wintun"
  extract_zip "$TMP/wintun.zip" "$TMP/wintun"
  mv "$TMP/wintun/wintun/bin/${arch}/wintun.dll" "$BIN_DIR/wintun.dll"
  mv "$TMP/wintun/wintun/LICENSE.txt" "$BIN_DIR/wintun-LICENSE.txt"
}

# ---------------------------------------------------------------------- main

fetch_xray
fetch_tun2socks
fetch_hysteria
fetch_wintun

# Windows has no execute bit to set.
if [ -z "$EXE" ]; then
  chmod +x "$BIN_DIR/xray-${TARGET}" "$BIN_DIR/tun2socks-${TARGET}" "$BIN_DIR/hysteria-${TARGET}"
fi

echo
echo "Done:"
ls -la "$BIN_DIR"
