#!/usr/bin/env bash
# scripts/libxray/build-xray-windows.sh
#
# Builds the xray engine the Windows installers ship: the upstream tag
# v26.3.27 with xray-core-windows-pvpn1.patch (next to this script) applied.
# The patch says what it changes and why: in a split by programs the stock
# engine died under a burst of new connections ("fatal error: thread
# exhaustion"), and IPv6 flows never matched a program.
#
# Both files sit in scripts/libxray because that folder is published as the
# source of our changes to Xray-core. The engine links GPL-3.0-or-later Go
# modules (THIRD-PARTY-NOTICES.md), so the script that builds it belongs to
# that source as much as the patch does.
#
# fetch-binaries.sh runs this for both Windows targets and checks the result
# against the xray-pvpn1 rows of scripts/sidecars.lock, so a Windows bundle is
# still exactly the file the lock names. That holds because Go builds the same
# bytes from the same source on any host, given the same toolchain, no cgo and
# trimmed paths: the sums in the lock were taken on macOS (arm64) and the
# Windows runners must arrive at the same ones.
#
# Before it builds, it runs the tests the patch brings, on the host: an engine
# in which one of the fixes no longer holds does not get built.
#
# Usage:
#   scripts/libxray/build-xray-windows.sh --tag v26.3.27 \
#     --patch scripts/libxray/xray-core-windows-pvpn1.patch --arch amd64|arm64 --out <file>
#
# --tag and --patch are what sidecars.lock says; they must match the pins
# below, so the lock and this script cannot drift apart unnoticed.
#
# Needs git and Go. Any Go that switches toolchains will do (1.21 or later):
# GOTOOLCHAIN below makes it fetch the pinned one when it is another version.
# Runs on macOS, Linux and Git Bash on Windows.

set -euo pipefail
cd "$(dirname "$0")/../.."

XRAY_TAG="v26.3.27"
XRAY_COMMIT="d2758a023cd7f4174a5a5fa4ff66e487d4342ba0"
XRAY_REPO="https://github.com/XTLS/Xray-core.git"
PATCH="scripts/libxray/xray-core-windows-pvpn1.patch"
# Another Go version builds other bytes, so the toolchain is part of the
# pinned sums: move it only together with the xray-pvpn1 rows of the lock.
GO_TOOLCHAIN="go1.26.2"
# Printed by `xray version` after the version number, so a log shows which
# build ran. Same place the upstream release puts its commit.
BUILD_NAME="pvpn1"

# The patch's tests, as "<package> <-run expression> <how many>". The count
# makes a renamed or dropped test fail here instead of passing as "no tests".
PATCH_TESTS=(
  "./app/router/ ^TestProcessLookupSource 3"
  "./common/net/ ^(TestABurstOfLookupsSharesOneSystemCall|TestAKeptCopyAnswersOnlyWhileFresh|TestAMissAsksForACopyBegunAfterTheLookup|TestTablesAreKeptApart|TestAFailedCopyIsReportedAndNotKept|TestPathsAreKeptBrieflyAndLookedUpFewAtATime|TestPathsKeptStayBounded|TestAFailedPathIsNotKept)$ 8"
  "./proxy/tun/ ^(TestSecondCloseOfAnOldFlowKeepsTheNewOne|TestPacketsRacingACloseNeverPanic)$ 2"
  "./transport/internet/ ^(TestABurstOfSocketsAsksForTheInterfaceOnce|TestAnInterfaceIndexIsAskedAgainWhenStale|TestInterfaceNamesAreKeptApart|TestAMissingInterfaceIsReportedAndNotKept)$ 4"
)

fail() {
  echo "build-xray-windows: $*" >&2
  exit 1
}

TAG=""
PATCH_ARG=""
ARCH=""
OUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --tag) TAG="${2:?--tag needs a value}"; shift 2 ;;
    --patch) PATCH_ARG="${2:?--patch needs a value}"; shift 2 ;;
    --arch) ARCH="${2:?--arch needs a value}"; shift 2 ;;
    --out) OUT="${2:?--out needs a value}"; shift 2 ;;
    -h|--help) sed -n '2,34p' "$0"; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[ "$TAG" = "$XRAY_TAG" ] || fail "sidecars.lock asks for '$TAG', this script builds $XRAY_TAG: move both together"
[ "$PATCH_ARG" = "$PATCH" ] || fail "sidecars.lock names '$PATCH_ARG', this script applies $PATCH: move both together"
[ -s "$PATCH" ] || fail "$PATCH is missing"
[ -n "$OUT" ] || fail "--out is required"
[ -d "$(dirname "$OUT")" ] || fail "the folder of $OUT does not exist"

# GOAMD64 and GOARM64 are named although they are the defaults: a value left
# in the environment would build other instructions and other bytes.
case "$ARCH" in
  amd64) export GOAMD64="v1" ;;
  arm64) export GOARM64="v8.0" ;;
  *) fail "--arch must be amd64 or arm64, not '$ARCH'" ;;
esac

command -v git >/dev/null 2>&1 || fail "needs git"
command -v go >/dev/null 2>&1 || fail "needs Go (1.21 or later)"

# GOENV=off: nothing a `go env -w` left on this machine takes part either.
export GOENV=off
export GOTOOLCHAIN="$GO_TOOLCHAIN"
export CGO_ENABLED=0
export GOFLAGS="-mod=readonly"
export GOWORK=off
unset GOEXPERIMENT
GOT_GO="$(go env GOVERSION)" || fail "go could not switch to $GO_TOOLCHAIN"
[ "$GOT_GO" = "$GO_TOOLCHAIN" ] || fail "go is $GOT_GO, the pinned sums need $GO_TOOLCHAIN"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SRC="$WORK/xray-core"

echo "  Xray-core $XRAY_TAG ($XRAY_COMMIT), $GO_TOOLCHAIN, windows/$ARCH"
# LF on every host: a Windows checkout with CRLF neither takes the patch nor
# builds the same bytes.
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$XRAY_TAG" \
  -c core.autocrlf=false -c core.eol=lf "$XRAY_REPO" "$SRC"
GOT_COMMIT="$(git -C "$SRC" rev-parse HEAD)"
[ "$GOT_COMMIT" = "$XRAY_COMMIT" ] || fail "tag $XRAY_TAG is $GOT_COMMIT now, not $XRAY_COMMIT: refusing to build"

# Through standard input, so no path has to survive Git Bash's translation.
git -C "$SRC" apply --check < "$PATCH" || fail "$PATCH does not apply to $XRAY_TAG"
git -C "$SRC" apply < "$PATCH"
echo "  patch applied"

for entry in "${PATCH_TESTS[@]}"; do
  read -r pkg expr want <<< "$entry"
  log="$WORK/test.log"
  if ! ( cd "$SRC" && go test -count=1 -v -run "$expr" "$pkg" ) > "$log" 2>&1; then
    cat "$log" >&2
    fail "the patch's tests in $pkg failed"
  fi
  passed="$(grep -c '^--- PASS' "$log" || true)"
  [ "$passed" = "$want" ] || { cat "$log" >&2; fail "$pkg: $passed of the patch's $want tests ran"; }
  echo "  tests ok: $pkg ($passed)"
done

# The flags of the upstream release build (.github/workflows/release.yml at
# the tag), with our build name in place of its commit.
( cd "$SRC" && GOOS=windows GOARCH="$ARCH" go build -o xray-pvpn1.exe \
    -trimpath -buildvcs=false -gcflags="all=-l=4" \
    -ldflags="-X github.com/xtls/xray-core/core.build=${BUILD_NAME} -s -w -buildid=" ./main )
mv "$SRC/xray-pvpn1.exe" "$OUT"

if command -v sha256sum >/dev/null 2>&1; then
  sum="$(sha256sum "$OUT" | awk '{print $1}')"
else
  sum="$(shasum -a 256 "$OUT" | awk '{print $1}')"
fi
echo "  built xray.exe (windows/$ARCH), sha256 $sum"
