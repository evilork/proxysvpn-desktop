# Third-party components

ProxysVPN Desktop ships several components it does not own. None of them is
committed to this repository — `scripts/fetch-binaries.sh` downloads the pinned
versions listed in `scripts/sidecars.lock` and verifies their SHA-256 sums
before a build can bundle them.

| Component | Version | Upstream | Licence | Shipped as |
|---|---|---|---|---|
| Xray-core | 26.3.27 | https://github.com/XTLS/Xray-core | MPL-2.0 | sidecar `xray` + `geoip.dat`, `geosite.dat` |
| tun2socks | 2.6.0 | https://github.com/xjasonlyu/tun2socks | GPL-3.0 | sidecar `tun2socks` |
| hysteria | 2.9.3 | https://github.com/apernet/hysteria | MIT | sidecar `hysteria` |
| Wintun | 0.14.1 | https://www.wintun.net | Prebuilt Binaries Licence (WireGuard LLC) | `wintun.dll`, Windows only |

## Wintun

Wintun is the kernel TUN driver that tun2socks uses on Windows. The prebuilt
`wintun.dll` is distributed under WireGuard LLC's "Prebuilt Binaries License",
which permits redistribution when the DLL travels **alongside software that uses
it only through the published API** (`wintun.h`) — which is exactly how
tun2socks consumes it — and which forbids modifying the binary or removing its
notices.

Accordingly the Windows bundle contains the unmodified `wintun.dll` next to
`wintun-LICENSE.txt`, the licence text taken verbatim from the upstream archive.
Both are installed into the application directory; neither may be stripped from
a release.

The DLL is loaded at run time by `tun2socks.exe`, from the directory the
executable lives in — which is why it is mapped to the install root in
`src-tauri/tauri.windows.conf.json` rather than into a resources subfolder.
