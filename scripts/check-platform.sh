#!/usr/bin/env bash
# scripts/check-platform.sh
#
# Everything about the Windows and Linux ports that CAN be verified from a Mac.
#
# The full app cannot be type-checked for those targets here: reqwest pulls in
# `ring`, whose build script compiles C and therefore needs the MSVC toolchain /
# a Linux sysroot. The platform layer was split into its own crate (no tauri, no
# reqwest, no rustls — see src-tauri/crates/pvpn-platform/Cargo.toml) precisely
# so that the OS-specific code is not inside that blind spot: it is pure Rust
# plus the `windows` crate and libc, so `cargo check --target …` works for both
# without either machine. That covers the Win32 FFI, the Linux root helper and
# every route/DNS code path.
#
# What this does NOT prove: that the full app links, that `netsh` and `ip`
# accept every argument spelling, that tun2socks really names its adapter the
# way we expect, or that pkexec behaves in a live desktop session. Only CI on
# windows-latest / windows-11-arm / ubuntu-22.04 and a run on real hardware
# can.
#
# Usage: scripts/check-platform.sh
#
# A cross target that rustup has not installed is skipped with a warning that
# names the `rustup target add` to run, so a Mac without one of them still
# gets every other check. In CI (CI=true, which GitHub Actions sets) a missing
# target is an error instead: the cross-check job installs all of them, and a
# skipped one there would be a check that silently stopped running.

set -euo pipefail
cd "$(dirname "$0")/../src-tauri"

# aarch64-pc-windows-msvc since the arm64 installer (02.10.2026). It is the
# same code, but the windows crate picks some definitions per CPU, so the x64
# check does not vouch for it; here it takes seconds, on the Arm runner a full
# build.
TARGETS=(x86_64-pc-windows-msvc aarch64-pc-windows-msvc x86_64-unknown-linux-gnu)

echo "== host build (macOS) =============================================="
cargo clippy --workspace --all-targets -- -D warnings

echo
echo "== host tests ====================================================="
cargo test -p pvpn-platform
# The GUI crate's tests need the frontend bundle that generate_context! embeds.
if [ -d ../dist ]; then
  cargo test -p proxysvpn-desktop --lib
else
  echo "skipping proxysvpn-desktop tests: ../dist is missing (run npm run build)"
fi

# rustup's answer once; no rustup at all reads as "none installed".
installed="$(rustup target list --installed 2>/dev/null || true)"
case "${CI:-}" in
  true | 1) strict=1 ;;
  *) strict=0 ;;
esac

# Space-separated rather than an array: macOS's bash 3.2 calls an empty array
# unbound under `set -u`.
skipped=""
for target in "${TARGETS[@]}"; do
  if ! printf '%s\n' "$installed" | grep -qx "$target"; then
    if [ "$strict" -eq 1 ]; then
      echo "error: target $target is not installed; the CI job must add it: rustup target add $target" >&2
      exit 1
    fi
    echo "warning: target $target is not installed, so its cross-check is skipped; to run it: rustup target add $target" >&2
    skipped="$skipped $target"
  fi
done

for target in "${TARGETS[@]}"; do
  case " $skipped " in
    *" $target "*) continue ;;
  esac
  echo
  echo "== platform layer, cross-checked for $target =="
  # Clippy and not just check: the lints are where the FFI mistakes show up,
  # and these two targets get no other review on this machine.
  cargo clippy -p pvpn-platform --all-targets --target "$target" -- -D warnings
done

echo
if [ -n "$skipped" ]; then
  echo "All other checks passed; NOT cross-checked (target not installed):$skipped." >&2
  echo "To check them too: rustup target add$skipped" >&2
else
  echo "All checks that are possible without a Windows or Linux machine passed."
fi
