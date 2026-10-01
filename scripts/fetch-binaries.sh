#!/usr/bin/env bash
# scripts/fetch-binaries.sh
# Downloads the xray-core (+ geo data), hysteria2 and tun2socks sidecars that
# Tauri bundles as externalBin, for one target triple.
#
# Versions and checksums come from scripts/sidecars.lock — pinned, and verified
# after every download. Run once after cloning, and after cleaning target/.
#
#   scripts/fetch-binaries.sh                      # for this host
#   scripts/fetch-binaries.sh --target x86_64-unknown-linux-gnu
#   scripts/fetch-binaries.sh --refresh-lock       # print new sha256 lines
#
# Windows targets are handled on the Windows branch (they also need wintun.dll).

set -euo pipefail
cd "$(dirname "$0")/.."

LOCK="scripts/sidecars.lock"
BIN_DIR="src-tauri/binaries"
TARGET=""
REFRESH=0

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --target) [ $# -ge 2 ] || die "--target needs a triple"; TARGET="$2"; shift 2 ;;
    --refresh-lock) REFRESH=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

host_triple() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)    echo aarch64-apple-darwin ;;
    Darwin-x86_64)   echo x86_64-apple-darwin ;;
    Linux-x86_64)    echo x86_64-unknown-linux-gnu ;;
    Linux-aarch64)   echo aarch64-unknown-linux-gnu ;;
    Linux-arm64)     echo aarch64-unknown-linux-gnu ;;
    *) die "unsupported host $(uname -s)-$(uname -m); pass --target explicitly" ;;
  esac
}

[ -n "$TARGET" ] || TARGET="$(host_triple)"

# Release asset names per target.
case "$TARGET" in
  aarch64-apple-darwin)
    XRAY_ASSET="Xray-macos-arm64-v8a.zip"
    T2S_ASSET="tun2socks-darwin-arm64.zip"
    HY_ASSET="hysteria-darwin-arm64" ;;
  x86_64-apple-darwin)
    XRAY_ASSET="Xray-macos-64.zip"
    T2S_ASSET="tun2socks-darwin-amd64.zip"
    HY_ASSET="hysteria-darwin-amd64" ;;
  x86_64-unknown-linux-gnu)
    XRAY_ASSET="Xray-linux-64.zip"
    T2S_ASSET="tun2socks-linux-amd64.zip"
    HY_ASSET="hysteria-linux-amd64" ;;
  aarch64-unknown-linux-gnu)
    XRAY_ASSET="Xray-linux-arm64-v8a.zip"
    T2S_ASSET="tun2socks-linux-arm64.zip"
    HY_ASSET="hysteria-linux-arm64" ;;
  *) die "unsupported target $TARGET" ;;
esac

[ -f "$LOCK" ] || die "$LOCK is missing"

lock_tag() {
  awk -v k="$1" '$1=="tag" && $2==k { print $3; found=1 } END { exit !found }' "$LOCK" \
    || die "no tag for $1 in $LOCK"
}

lock_sha() {
  awk -v k="$1" '$1=="sha256" && $2==k { print $3; found=1 } END { exit !found }' "$LOCK" \
    || die "no sha256 for $1 in $LOCK — bump the lock or fix the asset name"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "need sha256sum or shasum"
  fi
}

# URL-encode the slash in hysteria's "app/v2.12.3" tag.
urlencode_tag() { printf '%s' "$1" | sed 's|/|%2F|g'; }

for tool in curl unzip awk; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done

XRAY_TAG="$(lock_tag xray)"
T2S_TAG="$(lock_tag tun2socks)"
HY_TAG="$(lock_tag hysteria)"

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

fetch() {
  echo "  GET $1"
  curl --fail --silent --show-error --location --retry 3 --retry-delay 2 -o "$2" "$1" \
    || die "download failed: $1"
}

verify() {
  local file="$1" asset="$2" got want
  got="$(sha256_of "$file")"
  want="$(lock_sha "$asset")"
  if [ "$got" != "$want" ]; then
    die "sha256 mismatch for $asset
  expected $want
  got      $got
Either the download was tampered with or upstream re-tagged the release.
Do not ignore this: verify by hand before touching $LOCK."
  fi
  echo "  sha256 ok: $asset"
}

XRAY_URL="https://github.com/XTLS/Xray-core/releases/download/${XRAY_TAG}/${XRAY_ASSET}"
T2S_URL="https://github.com/xjasonlyu/tun2socks/releases/download/${T2S_TAG}/${T2S_ASSET}"
HY_URL="https://github.com/apernet/hysteria/releases/download/$(urlencode_tag "$HY_TAG")/${HY_ASSET}"

if [ "$REFRESH" -eq 1 ]; then
  echo "Recomputing sha256 for $TARGET (nothing is installed):"
  fetch "$XRAY_URL" "$TMP/$XRAY_ASSET"
  fetch "$T2S_URL" "$TMP/$T2S_ASSET"
  fetch "$HY_URL" "$TMP/$HY_ASSET"
  echo
  echo "Paste these into $LOCK and review the diff:"
  printf 'sha256 %s %s\n' "$XRAY_ASSET" "$(sha256_of "$TMP/$XRAY_ASSET")"
  printf 'sha256 %s %s\n' "$T2S_ASSET" "$(sha256_of "$TMP/$T2S_ASSET")"
  printf 'sha256 %s %s\n' "$HY_ASSET" "$(sha256_of "$TMP/$HY_ASSET")"
  exit 0
fi

mkdir -p "$BIN_DIR"
echo "Target: $TARGET"

echo "xray $XRAY_TAG ($XRAY_ASSET)"
fetch "$XRAY_URL" "$TMP/$XRAY_ASSET"
verify "$TMP/$XRAY_ASSET" "$XRAY_ASSET"
unzip -qo "$TMP/$XRAY_ASSET" -d "$TMP/xray"
[ -f "$TMP/xray/xray" ] || die "no 'xray' inside $XRAY_ASSET"
mv "$TMP/xray/xray" "$BIN_DIR/xray-$TARGET"
# geoip/geosite are architecture independent and ship as Tauri resources.
for geo in geoip.dat geosite.dat; do
  [ -f "$TMP/xray/$geo" ] || die "no $geo inside $XRAY_ASSET"
  mv "$TMP/xray/$geo" "$BIN_DIR/$geo"
done

echo "tun2socks $T2S_TAG ($T2S_ASSET)"
fetch "$T2S_URL" "$TMP/$T2S_ASSET"
verify "$TMP/$T2S_ASSET" "$T2S_ASSET"
unzip -qo "$TMP/$T2S_ASSET" -d "$TMP/t2s"
# Upstream names the file inside the archive after the archive itself.
T2S_INNER="$TMP/t2s/${T2S_ASSET%.zip}"
[ -f "$T2S_INNER" ] || die "expected ${T2S_ASSET%.zip} inside $T2S_ASSET; got: $(ls "$TMP/t2s")"
mv "$T2S_INNER" "$BIN_DIR/tun2socks-$TARGET"

echo "hysteria $HY_TAG ($HY_ASSET)"
fetch "$HY_URL" "$TMP/$HY_ASSET"
verify "$TMP/$HY_ASSET" "$HY_ASSET"
mv "$TMP/$HY_ASSET" "$BIN_DIR/hysteria-$TARGET"

chmod +x "$BIN_DIR/xray-$TARGET" "$BIN_DIR/hysteria-$TARGET" "$BIN_DIR/tun2socks-$TARGET"
echo "Done:"; ls -la "$BIN_DIR"
