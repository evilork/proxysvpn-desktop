#!/usr/bin/env bash
# Builds LibXray.xcframework (Xray-core as a C library) for the Apple
# PacketTunnel extension and installs it into src-tauri/gen/apple/Frameworks/.
# Also regenerates scripts/libxray-notices.json, the licence list of every Go
# module linked into that library (input for the in-app licences screen).
#
# Why this engine: Xray-core runs every transport the fleet serves (REALITY
# with Vision or XHTTP, Hysteria2) and is MPL-2.0; libXray (MIT) wraps it in a
# single C entry point. sing-box is GPL-3.0 and has no XHTTP. docs/IOS.md.
#
# Why the cgo build ("apple go") rather than gomobile: it has a tvOS slice and a
# settable minimum OS version, and it is one Go runtime in one static library.
# Never link a second Go library (Libbox, a separate hysteria build) into the
# same extension: two Go runtimes in one process crash at load.
#
# Two changes to what libXray links, both for the licence (checked 28.09.2026):
#   - Xray-core's Shadowsocks 2022 is built on sagernet/sing, GPL-3.0-or-later,
#     and its config loader and CLI pull it in even when no config uses it.
#     scripts/libxray/xray-core-no-gpl.patch removes it (we never use it) and
#     is applied to a pinned Xray-core checkout, which libXray's "local" mode
#     then builds against.
#   - XTLS/REALITY imports juju/ratelimit (LGPL-3.0) for a server-only feature.
#     scripts/libxray/ratelimit is our MIT stand-in with the same three names.
# The licence check at the end refuses any GPL-family module that remains.
#
# Everything is pinned, so the same tag of this repository builds the same
# engine: the libXray tag AND its commit (a moved tag fails the build), the
# Xray-core pseudo-version libXray's go.mod must carry, and the Go toolchain
# libXray's go.mod asks for. Go module downloads are verified by go.sum and
# the public checksum database.
#
# Requirements: full Xcode (iOS, iOS Simulator, macOS, tvOS, tvOS Simulator
# SDKs), Go (any 1.21+; it fetches the pinned toolchain itself), python3, git.
#
# Usage:
#   bash scripts/build-libxray.sh               # build + notices
#   bash scripts/build-libxray.sh --notices-only

set -euo pipefail

# ── Pins ─────────────────────────────────────────────────────────────────────
# Bump all together, from the libXray release notes and its go.mod; then check
# that the patch still applies (the build fails if it does not).
LIBXRAY_TAG="v26.9.9"
LIBXRAY_COMMIT="50b95979f5db551bd273165cf469e5daaf791341"
# Xray-core v26.9.9 (commit 52a412d9e2f5c2a5142b1b4e2ab3771dacb8b120), as the
# Go pseudo-version libXray v26.9.9 pins in go.mod.
XRAY_CORE_MODULE_VERSION="v1.260327.1-0.20260908222543-52a412d9e2f5"
GO_TOOLCHAIN="go1.27.1"

XRAY_CORE_TAG="v26.9.9"
XRAY_CORE_COMMIT="52a412d9e2f5c2a5142b1b4e2ab3771dacb8b120"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PATCH_DIR="$REPO_ROOT/scripts/libxray"
DEST_DIR="$REPO_ROOT/src-tauri/gen/apple/Frameworks"
NOTICES_FILE="$REPO_ROOT/scripts/libxray-notices.json"

NOTICES_ONLY=0
case "${1:-}" in
    "") ;;
    --notices-only) NOTICES_ONLY=1 ;;
    *) echo "usage: $0 [--notices-only]" >&2; exit 2 ;;
esac

fail() { echo "error: $*" >&2; exit 1; }

command -v go >/dev/null 2>&1 || fail "Go is required (brew install go)"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"
command -v git >/dev/null 2>&1 || fail "git is required"
if [ "$NOTICES_ONLY" = 0 ] && ! xcode-select -p 2>/dev/null | grep -q "Xcode.app"; then
    fail "full Xcode is required, then: sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
fi

# The exact toolchain, downloaded by the go command if missing. A newer local
# Go would otherwise build with itself, and the engine would differ per Mac.
export GOTOOLCHAIN="$GO_TOOLCHAIN"

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/libxray-build.XXXXXX")"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "==> Cloning libXray $LIBXRAY_TAG"
git clone --quiet --depth 1 --branch "$LIBXRAY_TAG" \
    https://github.com/XTLS/libXray.git "$WORK_DIR/libXray"
cd "$WORK_DIR/libXray"

HEAD_COMMIT="$(git rev-parse HEAD)"
[ "$HEAD_COMMIT" = "$LIBXRAY_COMMIT" ] \
    || fail "libXray $LIBXRAY_TAG is commit $HEAD_COMMIT, expected $LIBXRAY_COMMIT (tag moved?)"
grep -q "github.com/xtls/xray-core $XRAY_CORE_MODULE_VERSION\$" go.mod \
    || fail "libXray $LIBXRAY_TAG no longer pins Xray-core $XRAY_CORE_MODULE_VERSION - update the pins above"
[ "$(go version | awk '{print $3}')" = "$GO_TOOLCHAIN" ] \
    || fail "go did not switch to $GO_TOOLCHAIN (GOTOOLCHAIN=$GOTOOLCHAIN): $(go version)"

echo "==> Cloning Xray-core $XRAY_CORE_TAG and applying scripts/libxray/xray-core-no-gpl.patch"
# libXray's "local" mode builds against ../Xray-core instead of the module.
git clone --quiet --depth 1 --branch "$XRAY_CORE_TAG" \
    https://github.com/XTLS/Xray-core.git "$WORK_DIR/Xray-core"
XRAY_HEAD="$(git -C "$WORK_DIR/Xray-core" rev-parse HEAD)"
[ "$XRAY_HEAD" = "$XRAY_CORE_COMMIT" ] \
    || fail "Xray-core $XRAY_CORE_TAG is commit $XRAY_HEAD, expected $XRAY_CORE_COMMIT (tag moved?)"
case "$XRAY_CORE_MODULE_VERSION" in
    *"-${XRAY_CORE_COMMIT:0:12}") ;;
    *) fail "Xray-core commit $XRAY_CORE_COMMIT is not the one libXray pins ($XRAY_CORE_MODULE_VERSION)" ;;
esac
git -C "$WORK_DIR/Xray-core" apply --check "$PATCH_DIR/xray-core-no-gpl.patch" \
    || fail "xray-core-no-gpl.patch no longer applies to Xray-core $XRAY_CORE_TAG - redo it for the new version"
git -C "$WORK_DIR/Xray-core" apply "$PATCH_DIR/xray-core-no-gpl.patch"

# Both replacements go into go.mod BEFORE libXray's build script runs: it
# snapshots go.mod and restores it afterwards, and the module list below must
# see the same graph the build compiled.
go mod edit \
    -replace="github.com/xtls/xray-core=../Xray-core" \
    -replace="github.com/juju/ratelimit=$PATCH_DIR/ratelimit"
go mod tidy

if [ "$NOTICES_ONLY" = 0 ]; then
    echo "==> Building LibXray.xcframework with $(go version | awk '{print $3}') (this takes a while)"
    # libXray's own build: iOS, iOS Simulator, macOS, tvOS and tvOS Simulator
    # slices of one c-archive, plus module.modulemap (Swift: import LibXray).
    # It also downloads geoip/geosite .dat files into the clone; they are not
    # part of the framework and are thrown away with it (rules stay inline).
    python3 build/main.py apple go local
    [ -d LibXray.xcframework ] || fail "LibXray.xcframework was not produced"

    mkdir -p "$DEST_DIR"
    rm -rf "$DEST_DIR/LibXray.xcframework"
    cp -R LibXray.xcframework "$DEST_DIR/"
    cat >"$DEST_DIR/LibXray.version" <<EOF
libXray $LIBXRAY_TAG ($LIBXRAY_COMMIT)
Xray-core $XRAY_CORE_TAG ($XRAY_CORE_COMMIT) + scripts/libxray/xray-core-no-gpl.patch
github.com/juju/ratelimit => scripts/libxray/ratelimit
$GO_TOOLCHAIN
EOF
    echo "==> Installed: $DEST_DIR/LibXray.xcframework"
fi

echo "==> Listing the Go modules linked into the iOS slice"
# The package graph of the c-archive entry point for the device slice: what is
# actually compiled in, not everything go.mod mentions (test-only modules and
# other platforms' dependencies are left out).
MODULES_FILE="$WORK_DIR/modules.txt"
# Tab-separated: path, required version, source directory, replaced (0/1).
GOOS=ios GOARCH=arm64 CGO_ENABLED=1 GOFLAGS=-tags=ios go list -deps \
    -f '{{with .Module}}{{.Path}}{{"\t"}}{{.Version}}{{"\t"}}{{with .Replace}}{{.Dir}}{{"\t"}}1{{else}}{{.Dir}}{{"\t"}}0{{end}}{{end}}' \
    ./cgo_bridge | sed '/^$/d' | sort -u >"$MODULES_FILE"
[ -s "$MODULES_FILE" ] || fail "go list returned no modules"

python3 - "$MODULES_FILE" "$NOTICES_FILE" "$LIBXRAY_TAG" "$LIBXRAY_COMMIT" "$XRAY_CORE_COMMIT" "$(go env GOROOT)" "$GO_TOOLCHAIN" <<'PY'
"""Writes the licence list of the engine's Go modules.

Fails on a licence it cannot name and on any GPL-family text: copyleft beyond
MPL-2.0's file level must never reach the App Store extension.
"""
import json
import os
import re
import sys

(modules_file, out_file, libxray_tag, libxray_commit, xray_core_commit,
 goroot, go_version) = sys.argv[1:8]

LICENSE_NAMES = ("LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "COPYING", "LICENSE-MIT")

# Modules this build does not take as published (see the header of the script).
XRAY_CORE = "github.com/xtls/xray-core"
RATELIMIT = "github.com/juju/ratelimit"


def classify(text: str) -> str:
    flat = " ".join(text.split())
    # MPL-2.0 first: its text names the GNU licences as "Secondary Licenses".
    if "Mozilla Public License Version 2.0" in flat or "Mozilla Public License, version 2.0" in flat:
        return "MPL-2.0"
    if "Apache License" in flat and "Version 2.0" in flat:
        return "Apache-2.0"
    if "Permission is hereby granted, free of charge" in flat:
        return "MIT"
    if "Permission to use, copy, modify, and/or distribute this software for any purpose" in flat:
        return "ISC"
    if "Redistribution and use in source and binary forms" in flat:
        return "BSD-3-Clause" if "Neither the name" in flat or "names of its contributors" in flat else "BSD-2-Clause"
    if "This is free and unencumbered software released into the public domain" in flat:
        return "Unlicense"
    if re.search(r"GNU (AFFERO |LESSER )?GENERAL PUBLIC LICENSE|\bL?GPL(v?[23])?\b", flat, re.I):
        return "GPL-family"
    return ""


def source_url(path: str, version: str) -> str:
    # A pseudo-version ends in the 12-character commit it was cut from.
    commit = re.search(r"-[0-9]{14}-([0-9a-f]{12})$", version)
    ref = commit.group(1) if commit else version.split("+")[0]
    parts = path.split("/")
    if parts[0] == "github.com" and len(parts) >= 3:
        return f"https://github.com/{parts[1]}/{parts[2]}/tree/{ref}"
    return f"https://pkg.go.dev/{path}@{version}"


def read_text(directory: str, names) -> "str | None":
    for name in names:
        candidate = os.path.join(directory, name)
        if os.path.isfile(candidate):
            with open(candidate, encoding="utf-8", errors="replace") as handle:
                return handle.read()
    return None


entries = []
problems = []
with open(modules_file, encoding="utf-8") as handle:
    for line in handle:
        fields = line.rstrip("\n").split("\t")
        if len(fields) != 4:
            problems.append(f"unexpected go list line: {line.strip()!r}")
            continue
        path, version, directory, replaced = fields
        # The main module (libXray itself) has no version in its own build.
        version = version or libxray_tag
        text = read_text(directory, LICENSE_NAMES)
        if text is None:
            problems.append(f"{path}: no licence file in {directory}")
            continue
        spdx = classify(text)
        if spdx == "GPL-family":
            problems.append(f"{path}: GPL-family licence - must not ship in the App Store build")
            continue
        if not spdx:
            problems.append(f"{path}: licence not recognised")
            continue
        entry = {"name": path, "version": version, "license": spdx}
        if path == "github.com/xtls/libxray":
            entry["source"] = f"https://github.com/XTLS/libXray/tree/{libxray_commit}"
        elif path == XRAY_CORE:
            entry["source"] = f"https://github.com/XTLS/Xray-core/tree/{xray_core_commit}"
            entry["modifications"] = "scripts/libxray/xray-core-no-gpl.patch (Shadowsocks 2022 and the CLI commands removed)"
        elif replaced == "1" and path == RATELIMIT:
            entry["name"] = f"{RATELIMIT} API (ProxysVPN's own implementation)"
            entry["version"] = "-"
            entry["source"] = "scripts/libxray/ratelimit"
        elif replaced == "1":
            problems.append(f"{path}: replaced by {directory}, which this script does not know")
            continue
        else:
            entry["source"] = source_url(path, version)
        entry["licenseText"] = text
        notice = read_text(directory, ("NOTICE", "NOTICE.txt", "NOTICE.md"))
        if notice:
            entry["noticeText"] = notice
        entries.append(entry)

go_license = read_text(goroot, ("LICENSE",))
if go_license is None or classify(go_license) != "BSD-3-Clause":
    problems.append(f"Go standard library: licence not found or not BSD-3-Clause in {goroot}")
else:
    entries.append({
        "name": "Go standard library and runtime",
        "version": go_version,
        "license": "BSD-3-Clause",
        "source": f"https://go.googlesource.com/go/+/refs/tags/{go_version}",
        "licenseText": go_license,
    })

if problems:
    sys.stderr.write("licence check failed:\n  " + "\n  ".join(problems) + "\n")
    sys.exit(1)

entries.sort(key=lambda e: e["name"].lower())
document = {
    "generatedBy": "scripts/build-libxray.sh",
    "component": "Apple tunnel engine (PacketTunnel.appex): LibXray.xcframework",
    "libXray": {"tag": libxray_tag, "commit": libxray_commit},
    "modules": entries,
}
with open(out_file, "w", encoding="utf-8") as handle:
    json.dump(document, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(f"wrote {out_file}: {len(entries)} components")
PY
