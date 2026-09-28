#!/usr/bin/env bash
# Runs the Network Extension's pure Swift logic on the Mac (tests/packet-tunnel).
#
# The iOS simulator does not start a packet tunnel, and the extension has no
# XCTest target, so the files under test are compiled for macOS together with
# a plain test executable. Only files that import nothing but Foundation and
# Darwin belong here; PacketTunnelProvider.swift itself needs LibXray and a
# device.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Swift 5 mode, like the PacketTunnel target (project.yml SWIFT_VERSION).
swiftc -swift-version 5 -warnings-as-errors \
  -o "$work/packet-tunnel-tests" \
  "$root/src-tauri/gen/apple/PacketTunnel/Ipv6OnlyNetwork.swift" \
  "$root/tests/packet-tunnel/main.swift"

"$work/packet-tunnel-tests"
