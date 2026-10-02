#!/usr/bin/env bash
# scripts/appimage-lgpl-sources.sh
#
# Writes where the shared libraries in the Linux AppImage come from: for every
# file linuxdeploy put into the image's usr/lib, the Ubuntu binary package it
# was copied from and that package's source package, each with its exact
# version, as dpkg on the build machine knows them. That is the "complete
# corresponding source" the LGPL asks us to point at (THIRD-PARTY-NOTICES.md):
# `apt-get source <source package>=<version>`.
#
#   bash scripts/appimage-lgpl-sources.sh <AppImage> <list file>
#       write the list for the image's libraries
#   bash scripts/appimage-lgpl-sources.sh --check <AppImage> <list file>
#       fail unless the image's libraries are still the ones the list names
#       and the image carries that very list as LGPL_SOURCES_INSIDE
#
# CI (desktop-build, "LGPL sources of the AppImage") writes the list into
# src-tauri/linux/LGPL-SOURCES.txt, builds the AppImage again so that
# tauri.linux.conf.json puts it inside as usr/share/doc/proxysvpn-desktop/
# LGPL-SOURCES.txt, checks that image with --check, and uploads the same file
# beside it as LGPL-SOURCES-linux-appimage.txt, which
# scripts/publish-release.sh attaches to the release.
#
# It must run where the image was built: dpkg answers for the packages
# installed there, which are the ones linuxdeploy copied from. The output is
# sorted and holds no time, so --check on the same machine compares exactly.
# The image is extracted with its own --appimage-extract (no FUSE needed).

set -euo pipefail

LGPL_SOURCES_INSIDE="usr/share/doc/proxysvpn-desktop/LGPL-SOURCES.txt"

die() { printf 'appimage-lgpl-sources: %s\n' "$*" >&2; exit 1; }

check=0
if [[ "${1:-}" == "--check" ]]; then
    check=1
    shift
fi
[[ $# -eq 2 ]] || die "usage: bash scripts/appimage-lgpl-sources.sh [--check] <AppImage> <list file>"
image="$1"
list="$2"
[[ -f "$image" ]] || die "no AppImage at $image"
command -v dpkg >/dev/null 2>&1 && command -v dpkg-query >/dev/null 2>&1 \
    || die "dpkg is needed: run this on the Debian or Ubuntu machine that built the image"

image_abs="$(cd "$(dirname "$image")" && pwd)/$(basename "$image")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
( cd "$work" && "$image_abs" --appimage-extract >/dev/null ) || die "could not extract $image"
root="$work/squashfs-root"
[[ -d "$root/usr/lib" ]] || die "$image has no usr/lib"

# owners LIB: the binary packages, without their architecture, that own a
# file named LIB, one per line. dpkg -S prints "pkg[:arch][, pkg…]: /path",
# and "diversion by …" lines, which are not ownership.
owners() {
    { dpkg -S "*/$1" 2>/dev/null || true; } | awk -v want="$1" '
        /^diversion / { next }
        {
            at = index($0, ": /")
            if (at == 0) next
            path = substr($0, at + 2)
            n = split(path, part, "/")
            if (part[n] != want) next
            m = split(substr($0, 1, at - 1), pkg, ", ")
            for (i = 1; i <= m; i++) {
                name = pkg[i]
                sub(/:.*/, "", name)
                print name
            }
        }' | LC_ALL=C sort -u
}

render() {
    local libs rel lib pkg distro
    local map="$work/map" unowned="$work/unowned"
    : > "$map"
    : > "$unowned"
    libs="$(cd "$root" && find usr/lib \( -type f -o -type l \) -name '*.so*' | LC_ALL=C sort)"
    [[ -n "$libs" ]] || die "$image has no shared library in usr/lib"
    while IFS= read -r rel; do
        lib="${rel##*/}"
        pkg="$(owners "$lib")"
        if [[ -z "$pkg" ]]; then
            printf '%s\n' "$rel" >> "$unowned"
            continue
        fi
        while IFS= read -r name; do
            printf '%s %s\n' "$rel" "$name" >> "$map"
        done <<< "$pkg"
    done <<< "$libs"
    [[ -s "$map" ]] || die "no library in $image belongs to an installed package"

    distro="the build machine"
    if [[ -r /etc/os-release ]]; then
        # shellcheck disable=SC1091
        distro="$(. /etc/os-release && printf '%s' "${PRETTY_NAME:-$distro}")"
    fi

    printf '%s\n' \
        "# $(basename "$image"): where the shared libraries in its usr/lib come from." \
        "# Written by scripts/appimage-lgpl-sources.sh where the image was built" \
        "# ($distro). The image carries this file as" \
        "# $LGPL_SOURCES_INSIDE, and the release attaches it as" \
        "# LGPL-SOURCES-linux-appimage.txt." \
        "#" \
        "# linuxdeploy copied each library from the binary package named below, at" \
        "# exactly that version: the distribution's own build, except that linuxdeploy" \
        "# rewrote its RUNPATH and may have stripped it. Nothing else was changed." \
        "# The complete corresponding source of each is the distribution's source" \
        "# package of the version given, unmodified:" \
        "#   apt-get source <source package>=<version>" \
        "# or, for Ubuntu, https://launchpad.net/ubuntu/+source/<source package>/<version>" \
        "# THIRD-PARTY-NOTICES.md says which of them are under the LGPL and how to" \
        "# replace one in the image." \
        "" \
        "[packages]" \
        "# binary package=version  source package=version"
    # shellcheck disable=SC2016 # dpkg-query's own ${…} fields, not the shell's
    cut -d' ' -f2 "$map" | LC_ALL=C sort -u \
        | xargs dpkg-query -W -f='${Package}=${Version} ${source:Package}=${source:Version}\n' \
        | LC_ALL=C sort -u
    printf '%s\n' "" "[libraries]" "# file in the image  binary package"
    LC_ALL=C sort -u "$map"
    if [[ -s "$unowned" ]]; then
        printf '%s\n' "" "[not from a package]"
        cat "$unowned"
    fi
}

if [[ "$check" -eq 0 ]]; then
    render > "$work/list"
    mkdir -p "$(dirname "$list")"
    cp "$work/list" "$list"
    printf 'appimage-lgpl-sources: wrote %s\n' "$list" >&2
    exit 0
fi

[[ -f "$list" ]] || die "no list at $list"
render > "$work/list"
if ! cmp -s "$work/list" "$list"; then
    diff -u "$list" "$work/list" >&2 || true
    die "the libraries in $image are not the ones $list names"
fi
cmp -s "$root/$LGPL_SOURCES_INSIDE" "$list" \
    || die "$image does not carry $list as $LGPL_SOURCES_INSIDE"
printf 'appimage-lgpl-sources: %s matches %s, inside and out\n' "$(basename "$image")" "$list" >&2
