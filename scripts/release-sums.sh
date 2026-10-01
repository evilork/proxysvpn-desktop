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
