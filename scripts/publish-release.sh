#!/usr/bin/env bash
# scripts/publish-release.sh
#
# Creates a DRAFT GitHub release of ProxysVPN Desktop from installers that CI
# already built. It never touches git history: no `git add`, no commit, no
# push, no tag created, moved or deleted, no release deleted or overwritten.
# Publishing the draft (which is when GitHub creates the tag, at the commit
# named below) stays a click on the release page.
#
# The previous version of this script staged the whole working tree with
# `git add -A`, committed and pushed main directly, hard-coded v0.1.0-beta and
# offered to delete and recreate tags and releases. In a public repository
# that is how untracked local files end up published.
#
# Steps for a release (see RELEASE_NOTES.md, top section):
#   1. Merge the release branch to main and wait for desktop-build to pass.
#   2. Actions -> desktop-build -> Run workflow on main, tick upload_installers.
#   3. Download the three artifacts of THAT run into one folder and unzip
#      them there (each brings its SHA256SUMS-<artifact>.txt). Subfolders are
#      fine: the files are looked up at any depth (scripts/release-sums.sh).
#   4. bash scripts/publish-release.sh <that folder>
#   5. Read the draft on GitHub, then publish it.
#
# Requirements: gh (authenticated), git, node, shasum or sha256sum.
#
# Usage (from anywhere inside the repository):
#   bash scripts/publish-release.sh /path/to/downloaded-installers

set -euo pipefail

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

[[ $# -eq 1 ]] || die "usage: bash scripts/publish-release.sh <folder with the three installers>"
ASSETS_DIR="$(cd "$1" && pwd)" || die "no such folder: $1"

cd "$(git rev-parse --show-toplevel)"
# shellcheck source=scripts/release-sums.sh
source scripts/release-sums.sh

command -v gh >/dev/null 2>&1 || die "GitHub CLI (gh) is not installed"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated: gh auth login"
command -v node >/dev/null 2>&1 || die "node is needed to read the version"

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$@"; }
else
    sha256() { shasum -a 256 "$@"; }
fi

# ── What is being released ──────────────────────────────────────────────
VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"
PKG_VERSION="$(node -p "require('./package.json').version")"
[[ "$VERSION" == "$PKG_VERSION" ]] || die "tauri.conf.json says $VERSION, package.json says $PKG_VERSION"
grep -q "^version = \"$VERSION\"" src-tauri/Cargo.toml || die "src-tauri/Cargo.toml is not at $VERSION"

# The tag is the first heading of RELEASE_NOTES.md: "# ProxysVPN Desktop vX.Y.Z[-suffix]".
TAG="$(sed -n '1s/^# ProxysVPN Desktop \(v[^ ]*\)$/\1/p' RELEASE_NOTES.md)"
[[ -n "$TAG" ]] || die "RELEASE_NOTES.md must start with '# ProxysVPN Desktop v$VERSION...'"
[[ "$TAG" == "v$VERSION" || "$TAG" == "v$VERSION-"* ]] || die "RELEASE_NOTES.md is about $TAG, the app is $VERSION"

# ── The commit the release points at ────────────────────────────────────
[[ -z "$(git status --porcelain)" ]] || die "the working tree is not clean; release from a clean checkout of main"
git fetch --quiet origin main
HEAD_SHA="$(git rev-parse HEAD)"
MAIN_SHA="$(git rev-parse origin/main)"
[[ "$HEAD_SHA" == "$MAIN_SHA" ]] || die "HEAD ($HEAD_SHA) is not origin/main ($MAIN_SHA)"

if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    die "tag $TAG already exists on origin; this script never moves or deletes tags"
fi
if gh release view "$TAG" >/dev/null 2>&1; then
    die "a release $TAG already exists; this script never deletes or overwrites releases"
fi

# ── The three installers, nothing else ──────────────────────────────────
DMG="ProxysVPN_${VERSION}_aarch64.dmg"
EXE="ProxysVPN_${VERSION}_x64-setup.exe"
DEB="ProxysVPN_${VERSION}_amd64.deb"
hint="download the artifacts of the CI run on $MAIN_SHA"
DMG_PATH="$(release_find_asset "$ASSETS_DIR" "$DMG")" || die "no single $DMG under $ASSETS_DIR ($hint)"
EXE_PATH="$(release_find_asset "$ASSETS_DIR" "$EXE")" || die "no single $EXE under $ASSETS_DIR ($hint)"
DEB_PATH="$(release_find_asset "$ASSETS_DIR" "$DEB")" || die "no single $DEB under $ASSETS_DIR ($hint)"

# Bare file names, whatever subfolder each installer came in, so
# `sha256sum -c SHA256SUMS.txt` works next to the downloaded release assets.
SUMS="$ASSETS_DIR/SHA256SUMS.txt"
for path in "$DMG_PATH" "$EXE_PATH" "$DEB_PATH"; do
    ( cd "$(dirname "$path")" && sha256 "$(basename "$path")" )
done > "$SUMS"

# Cross-check against the sums the CI run wrote into its artifacts. Once any
# of those files is there, every installer must have a matching line in them:
# a missing line is an installer the run never vouched for.
ci_sums=()
while IFS= read -r f; do
    ci_sums+=("$f")
done < <(release_find_ci_sums "$ASSETS_DIR")
if [[ ${#ci_sums[@]} -gt 0 ]]; then
    release_check_against_ci "$SUMS" "${ci_sums[@]}" \
        || die "the installers do not match the CI run's SHA256SUMS files (${#ci_sums[@]} found)"
    echo "Checksums match the CI run's own SHA256SUMS files (${#ci_sums[@]} found)."
else
    echo "WARN: no SHA256SUMS-*.txt from CI next to the installers; not cross-checked."
fi

# ── Notes: the top section of RELEASE_NOTES.md, plus the sums ────────────
NOTES="$(mktemp)"
trap 'rm -f "$NOTES"' EXIT
awk 'NR > 1 && /^---$/ { exit } NR > 1 { print }' RELEASE_NOTES.md > "$NOTES"
{
    echo ""
    echo "## SHA-256"
    echo ""
    echo '```'
    cat "$SUMS"
    echo '```'
} >> "$NOTES"

# A suffix (v0.3.2-beta) makes a pre-release; a plain version becomes Latest,
# which is what /releases/latest links resolve to. A plain string rather than
# an array: macOS's bash 3.2 calls an empty array unbound under `set -u`.
KIND="--latest"
[[ "$TAG" == *-* ]] && KIND="--prerelease"

echo ""
echo "About to create a DRAFT release:"
echo "  tag:     $TAG (created by GitHub at $HEAD_SHA when the draft is published)"
echo "  assets:  $DMG, $EXE, $DEB, SHA256SUMS.txt"
echo "  kind:    ${KIND#--}"
read -r -p "Create the draft? [y/N] " answer
[[ "$answer" =~ ^[yY]$ ]] || die "nothing created"

gh release create "$TAG" \
    --draft \
    --target "$HEAD_SHA" \
    --title "ProxysVPN Desktop $TAG" \
    --notes-file "$NOTES" \
    "$KIND" \
    "$DMG_PATH" "$EXE_PATH" "$DEB_PATH" "$SUMS"

echo ""
echo "Draft created. Read it, then publish it on GitHub:"
gh release view "$TAG" --json url -q .url
