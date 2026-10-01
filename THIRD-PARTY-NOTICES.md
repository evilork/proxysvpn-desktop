# Third-party components

ProxysVPN Desktop ships several components it does not own. None of them is
committed to this repository — `scripts/fetch-binaries.sh` downloads the pinned
versions listed in `scripts/sidecars.lock` and verifies their SHA-256 sums
before a build can bundle them.

| Component | Version | Upstream | Licence | Shipped as |
|---|---|---|---|---|
| Xray-core | 26.3.27 | https://github.com/XTLS/Xray-core | MPL-2.0; the binary also contains GPL-3.0-or-later code, see below | sidecar `xray` + `geoip.dat`, `geosite.dat` |
| tun2socks | 2.6.0 | https://github.com/xjasonlyu/tun2socks | MIT | sidecar `tun2socks` |
| hysteria | 2.9.3 | https://github.com/apernet/hysteria | MIT; the binary also contains GPL-3.0-or-later code, see below | sidecar `hysteria` |
| Wintun | 0.14.1 | https://www.wintun.net | Prebuilt Binaries Licence (WireGuard LLC) | `wintun.dll`, Windows only |

## GPL-3.0-or-later code inside the desktop engines

The desktop sidecars are the unmodified upstream release binaries. Two of them
statically link Go modules licensed under the GNU General Public License,
version 3 or (at your option) any later version (`go version -m` on the pinned
releases):

| Engine binary | Module | Version | Source |
|---|---|---|---|
| Xray-core 26.3.27 | github.com/sagernet/sing | v0.5.1 | https://github.com/SagerNet/sing/tree/v0.5.1 |
| Xray-core 26.3.27 | github.com/sagernet/sing-shadowsocks | v0.2.7 | https://github.com/SagerNet/sing-shadowsocks/tree/v0.2.7 |
| hysteria 2.9.3 | github.com/sagernet/sing | v0.3.2 | https://github.com/SagerNet/sing/tree/v0.3.2 |
| hysteria 2.9.3 | github.com/apernet/sing-tun | v0.2.6-0.20250920121535-299f04629986 | https://github.com/apernet/sing-tun/tree/299f04629986 |

The complete corresponding source of each engine binary is its upstream
release tag (https://github.com/XTLS/Xray-core/tree/v26.3.27,
https://github.com/apernet/hysteria/tree/app/v2.9.3) together with the module
versions above. The full GPL-3.0 text ships inside the app
(`src/assets/licenses/GPL-3.0-or-later.txt`, shown on the "Open-source
licences" screen) and the components are listed there for macOS, Windows and
Linux. The iOS engine is built without these modules
(`scripts/libxray/xray-core-no-gpl.patch`).

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
