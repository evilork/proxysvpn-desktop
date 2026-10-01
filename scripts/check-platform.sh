#!/usr/bin/env bash
# scripts/check-platform.sh
#
# Everything about the Windows (and Linux) port that CAN be verified on a Mac.
#
# The full app cannot be type-checked for Windows here: reqwest pulls in `ring`,
# whose build script compiles C and therefore needs the MSVC toolchain and the
# Windows SDK. The platform layer was split into its own crate (no tauri, no
# reqwest, no rustls — see src-tauri/crates/pvpn-platform/Cargo.toml) precisely
# so that the OS-specific code is not in that blind spot: it is pure Rust plus
# the `windows` crate, so `cargo check --target x86_64-pc-windows-msvc` works
# without a Windows machine.
#
# What this does NOT prove: that the full app links on Windows, that netsh
# accepts every argument spelling, or that tun2socks creates a Wintun adapter
# named the way we expect. Only CI on windows-latest and a run on real hardware
# can.
#
# Usage: scripts/check-platform.sh

set -euo pipefail
cd "$(dirname "$0")/../src-tauri"

WIN_TARGET="x86_64-pc-windows-msvc"

echo "== host build (macOS) =============================================="
cargo check --workspace --all-targets

echo
echo "== host tests ====================================================="
cargo test -p pvpn-platform
# The GUI crate's tests need the frontend bundle that generate_context! embeds.
if [ -d ../dist ]; then
  cargo test -p proxysvpn-desktop --lib
else
  echo "skipping proxysvpn-desktop tests: ../dist is missing (run npm run build)"
fi

echo
echo "== platform layer, cross-checked for Windows ======================"
if rustup target list --installed | grep -qx "$WIN_TARGET"; then
  cargo check -p pvpn-platform --target "$WIN_TARGET" --all-targets
else
  echo "target $WIN_TARGET is not installed; run:"
  echo "  rustup target add $WIN_TARGET"
  exit 1
fi

echo
echo "All checks that are possible without a Windows toolchain passed."
