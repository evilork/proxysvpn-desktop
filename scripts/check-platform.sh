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
# windows-latest / ubuntu-22.04 and a run on real hardware can.
#
# Usage: scripts/check-platform.sh

set -euo pipefail
cd "$(dirname "$0")/../src-tauri"

TARGETS=(x86_64-pc-windows-msvc x86_64-unknown-linux-gnu)

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

missing=0
for target in "${TARGETS[@]}"; do
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "target $target is not installed; run: rustup target add $target"
    missing=1
  fi
done
[ "$missing" -eq 0 ] || exit 1

for target in "${TARGETS[@]}"; do
  echo
  echo "== platform layer, cross-checked for $target =="
  # Clippy and not just check: the lints are where the FFI mistakes show up,
  # and these two targets get no other review on this machine.
  cargo clippy -p pvpn-platform --all-targets --target "$target" -- -D warnings
done

echo
echo "All checks that are possible without a Windows or Linux machine passed."
