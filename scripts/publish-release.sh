#!/usr/bin/env bash
# scripts/publish-release.sh
# Publishes ProxysVPN Desktop to GitHub Releases.
#
# What it does:
#   1. Stages README.md, README_RU.md, LICENSE, RELEASE_NOTES.md (if missing)
#   2. Commits any pending changes with auto-message (asks for confirmation)
#   3. Pushes to origin main
#   4. Verifies the .dmg exists
#   5. Creates GitHub Release v0.1.0-beta and uploads the .dmg
#
# Requirements:
#   - gh (GitHub CLI) installed: brew install gh
#   - Authenticated:                gh auth login
#   - Built .dmg present:           bash src-tauri/scripts/build-release.sh
#
# Usage (from project root):
#   bash scripts/publish-release.sh

set -euo pipefail

VERSION="0.1.0-beta"
TAG="v${VERSION}"
DMG_NAME="ProxysVPN_0.1.0_aarch64.dmg"
DMG_PATH="src-tauri/target/release/bundle/dmg/${DMG_NAME}"

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
PROJECT_ROOT="$(pwd)"

echo "═══════════════════════════════════════════════════════"
echo "  ProxysVPN Desktop — publish ${TAG}"
echo "═══════════════════════════════════════════════════════"

# ── Pre-flight checks ────────────────────────────────────────────────
if ! command -v gh >/dev/null 2>&1; then
    echo "ERROR: GitHub CLI (gh) is not installed."
    echo "   brew install gh && gh auth login"
    exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
    echo "ERROR: gh is not authenticated."
    echo "   gh auth login"
    exit 1
fi

REMOTE_URL="$(git remote get-url origin 2>/dev/null || true)"
if [[ -z "$REMOTE_URL" ]]; then
    echo "ERROR: no 'origin' remote. Set it: git remote add origin <url>"
    exit 1
fi
echo "Remote: $REMOTE_URL"

# Check repo is public (warn if not)
REPO_VIS=$(gh repo view --json visibility -q .visibility 2>/dev/null || echo "unknown")
echo "Visibility: $REPO_VIS"
if [[ "$REPO_VIS" == "PRIVATE" ]]; then
    echo "WARNING: repo is PRIVATE. Releases will only be visible to repo collaborators."
    read -r -p "Continue? [y/N] " ans
    [[ "$ans" =~ ^[yY] ]] || { echo "aborted"; exit 1; }
fi

# ── Stage missing meta files ─────────────────────────────────────────
HERE="$(cd "$(dirname "$0")" && pwd)"

stage_if_missing() {
    local src="$1"
    local dst="$2"
    if [[ ! -f "$dst" ]]; then
        cp "$src" "$dst"
        echo "  + staged $dst"
    elif ! cmp -s "$src" "$dst"; then
        echo "  ~ keeping existing $dst (differs from template)"
    fi
}

# Look for templates relative to script location
TPL_DIR="$HERE/release-templates"
if [[ -d "$TPL_DIR" ]]; then
    echo ""
    echo "Staging meta files..."
    stage_if_missing "$TPL_DIR/README.md" "README.md"
    stage_if_missing "$TPL_DIR/README_RU.md" "README_RU.md"
    stage_if_missing "$TPL_DIR/LICENSE" "LICENSE"
    stage_if_missing "$TPL_DIR/RELEASE_NOTES.md" "RELEASE_NOTES.md"
fi

# Clean up duplicate gitignore entries
if [[ -f .gitignore ]]; then
    awk '!seen[$0]++ || /^$/ || /^#/' .gitignore > .gitignore.tmp
    if ! cmp -s .gitignore .gitignore.tmp; then
        mv .gitignore.tmp .gitignore
        echo "  + deduplicated .gitignore"
    else
        rm .gitignore.tmp
    fi
fi

# ── Commit pending changes ───────────────────────────────────────────
git add -A
if ! git diff --cached --quiet; then
    echo ""
    echo "Pending changes:"
    git diff --cached --stat
    echo ""
    read -r -p "Commit these and proceed? [y/N] " ans
    if [[ "$ans" =~ ^[yY] ]]; then
        git commit -m "release: prepare ${TAG}

- add bilingual README (RU + EN)
- add LICENSE (MIT)
- add release notes
- add log viewer + theme toggle in UI"
    else
        echo "aborted"
        exit 1
    fi
else
    echo "No staged changes."
fi

# ── Push ─────────────────────────────────────────────────────────────
echo ""
echo "Pushing main..."
git push origin main

# ── Verify .dmg exists ───────────────────────────────────────────────
if [[ ! -f "$DMG_PATH" ]]; then
    echo ""
    echo "ERROR: $DMG_PATH not found."
    echo "Build it first:  bash src-tauri/scripts/build-release.sh"
    exit 1
fi

DMG_SIZE=$(du -h "$DMG_PATH" | awk '{print $1}')
DMG_SHA256=$(shasum -a 256 "$DMG_PATH" | awk '{print $1}')
echo ""
echo ".dmg ready:"
echo "  path:    $DMG_PATH"
echo "  size:    $DMG_SIZE"
echo "  sha256:  $DMG_SHA256"

# ── Create or update RELEASE_NOTES with checksum ─────────────────────
TMP_NOTES=$(mktemp)
if [[ -f RELEASE_NOTES.md ]]; then
    # Replace SHA-256 placeholder line with actual hash
    awk -v hash="$DMG_SHA256" '
        /^\*\*SHA-256/ { print "**SHA-256 of the .dmg:** `" hash "`"; next }
        { print }
    ' RELEASE_NOTES.md > "$TMP_NOTES"
else
    echo "Release ${TAG}" > "$TMP_NOTES"
    echo "" >> "$TMP_NOTES"
    echo "**SHA-256:** \`${DMG_SHA256}\`" >> "$TMP_NOTES"
fi

# ── Create / update tag ──────────────────────────────────────────────
if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo ""
    echo "Tag $TAG exists locally."
    read -r -p "Force-update it (delete and recreate)? [y/N] " ans
    if [[ "$ans" =~ ^[yY] ]]; then
        git tag -d "$TAG"
        git push origin ":refs/tags/$TAG" 2>/dev/null || true
    else
        echo "Reusing existing tag."
    fi
fi

if ! git rev-parse "$TAG" >/dev/null 2>&1; then
    git tag -a "$TAG" -m "ProxysVPN Desktop $TAG"
    git push origin "$TAG"
    echo "Tag $TAG pushed."
fi

# ── Create release ───────────────────────────────────────────────────
echo ""
echo "Creating release on GitHub..."

if gh release view "$TAG" >/dev/null 2>&1; then
    echo "Release $TAG already exists."
    read -r -p "Delete and recreate? [y/N] " ans
    if [[ "$ans" =~ ^[yY] ]]; then
        gh release delete "$TAG" --yes
    else
        echo "Will upload .dmg to existing release..."
        gh release upload "$TAG" "$DMG_PATH" --clobber
        echo ""
        echo "Done. Release URL:"
        gh release view "$TAG" --json url -q .url
        rm -f "$TMP_NOTES"
        exit 0
    fi
fi

gh release create "$TAG" \
    --title "ProxysVPN Desktop $TAG" \
    --notes-file "$TMP_NOTES" \
    --prerelease \
    "$DMG_PATH"

rm -f "$TMP_NOTES"

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  ✓ Release published"
echo "═══════════════════════════════════════════════════════"
echo ""
echo "URL: $(gh release view "$TAG" --json url -q .url)"
echo ""
echo "Share this link:"
echo "  $(gh release view "$TAG" --json url -q .url)"
echo ""
echo "Direct .dmg download:"
DMG_URL=$(gh release view "$TAG" --json assets -q '.assets[] | select(.name | endswith(".dmg")) | .url')
echo "  $DMG_URL"
