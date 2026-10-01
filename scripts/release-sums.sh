# scripts/release-sums.sh
#
# Sourced by scripts/publish-release.sh, and by tests/releaseSums.test.ts.
# Defines functions only: sourcing it runs nothing, writes nothing and changes
# no shell option of the caller. Works with macOS's bash 3.2.
#
# The folder publish-release.sh is given is wherever the owner unzipped the CI
# artifacts. An artifact is flat today, but one from before the upload was
# fixed holds `dmg/`, `nsis/` or `deb/`, and an unzip tool may add a folder
# named after the zip. So the installers and the CI's SHA256SUMS-*.txt files
# are looked up at any depth, and a name found twice is an error rather than a
# guess.

# release_find_asset DIR NAME
#   Print the one path under DIR, at any depth, whose file name is NAME.
#   Fails with a message on stderr when there is none or more than one.
release_find_asset() {
    local dir="$1" name="$2" found="" path
    while IFS= read -r -d '' path; do
        if [[ -n "$found" ]]; then
            printf 'ERROR: %s is there twice: %s and %s; keep one\n' "$name" "$found" "$path" >&2
            return 1
        fi
        found="$path"
    done < <(find "$dir" -type f -name "$name" -print0)
    if [[ -z "$found" ]]; then
        printf 'ERROR: missing %s under %s\n' "$name" "$dir" >&2
        return 1
    fi
    printf '%s\n' "$found"
}

# release_find_optional_asset DIR NAME
#   As release_find_asset, except that none is not an error: it prints
#   nothing and succeeds. More than one still fails.
release_find_optional_asset() {
    local dir="$1" name="$2"
    if [[ -z "$(find "$dir" -type f -name "$name" -print)" ]]; then
        return 0
    fi
    release_find_asset "$dir" "$name"
}

# release_find_ci_sums DIR
#   Print every SHA256SUMS-*.txt under DIR, at any depth, one per line, in a
#   stable order. Prints nothing when there is none. Our own SHA256SUMS.txt
#   (no dash) is not one of them.
release_find_ci_sums() {
    find "$1" -type f -name 'SHA256SUMS-*.txt' | LC_ALL=C sort
}

# release_ci_sum_for NAME SUMS_FILE...
#   Print the sha256 the CI files record for NAME, in lower case.
#
#   Reads both line forms sha256sum writes: "<hash>  name" (text mode, macOS
#   and Linux) and "<hash> *name" (binary mode — Git Bash's sha256sum on the
#   Windows runner writes that). A CRLF line ending is tolerated.
#
#   Fails when no file names NAME, or when two lines disagree about it: once
#   the CI sums are there, an installer they do not vouch for is not
#   "probably fine".
release_ci_sum_for() {
    local name="$1"
    shift
    local sums count
    sums="$(awk -v want="$name" '
        { sub(/\r$/, "") }
        NF == 2 {
            file = $2
            sub(/^\*/, "", file)
            if (file == want) print tolower($1)
        }
    ' "$@" | LC_ALL=C sort -u)"
    if [[ -z "$sums" ]]; then
        printf 'ERROR: %s has no line in the CI run'\''s SHA256SUMS files\n' "$name" >&2
        return 1
    fi
    count="$(printf '%s\n' "$sums" | wc -l | tr -d ' ')"
    if [[ "$count" != "1" ]]; then
        printf 'ERROR: the CI run'\''s SHA256SUMS files disagree about %s\n' "$name" >&2
        return 1
    fi
    printf '%s\n' "$sums"
}

# release_check_against_ci OUR_SUMS SUMS_FILE...
#   Every line of OUR_SUMS ("<hash>  name", as sha256sum or shasum writes it)
#   must match the CI files exactly. Fails at the first installer that is
#   missing from them or differs.
release_check_against_ci() {
    local ours="$1"
    shift
    local sum name expected
    while read -r sum name; do
        [[ -n "$sum" ]] || continue
        name="${name#\*}"
        expected="$(release_ci_sum_for "$name" "$@")" || return 1
        if [[ "$expected" != "$(printf '%s' "$sum" | tr 'A-F' 'a-f')" ]]; then
            printf 'ERROR: %s differs from the sum its CI run recorded\n' "$name" >&2
            return 1
        fi
    done < "$ours"
}

# release_ci_names NAME [SUMS_FILE...]
#   Succeeds when a line of the CI files names NAME, in either line form.
#   With no file it fails: nothing was vouched for.
release_ci_names() {
    local name="$1"
    shift
    [[ $# -gt 0 ]] || return 1
    awk -v want="$name" '
        { sub(/\r$/, "") }
        NF == 2 {
            file = $2
            sub(/^\*/, "", file)
            if (file == want) found = 1
        }
        END { exit found ? 0 : 1 }
    ' "$@"
}

# release_collect_assets DIR VERSION [SUMS_FILE...]
#   Print the path of every installer of release VERSION under DIR, one per
#   line, in a fixed order: the three every release has, then the optional
#   ones that are there.
#
#   Required: the macOS dmg, the x64 Windows installer, the deb. Optional:
#   the arm64 Windows installer (CI builds it since 02.10.2026) and an
#   AppImage, should Linux get one again; each is summed and attached when
#   it is there, and the release goes out without it when it is not, which
#   is said on stderr. An optional installer that the CI sums files list
#   but the folder lacks is an error, though: that run built it, so the
#   folder is an incomplete download, not a release without it.
#
#   Fails as release_find_asset does for a required installer that is
#   missing and for any installer found twice.
release_collect_assets() {
    local dir="$1" version="$2"
    shift 2
    local name path
    local required="ProxysVPN_${version}_aarch64.dmg ProxysVPN_${version}_x64-setup.exe ProxysVPN_${version}_amd64.deb"
    local optional="ProxysVPN_${version}_arm64-setup.exe ProxysVPN_${version}_amd64.AppImage"
    # The names hold no blanks (the product name and a semver), so plain word
    # splitting is enough and keeps this bash 3.2 without arrays.
    for name in $required; do
        path="$(release_find_asset "$dir" "$name")" || return 1
        printf '%s\n' "$path"
    done
    for name in $optional; do
        path="$(release_find_optional_asset "$dir" "$name")" || return 1
        if [[ -n "$path" ]]; then
            printf '%s\n' "$path"
        elif release_ci_names "$name" "$@"; then
            printf 'ERROR: the CI run built %s, but it is not under %s; download all its artifacts\n' "$name" "$dir" >&2
            return 1
        else
            printf 'note: no %s under %s; released without it\n' "$name" "$dir" >&2
        fi
    done
}
