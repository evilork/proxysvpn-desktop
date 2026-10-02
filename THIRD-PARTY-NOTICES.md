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

## Other Go modules inside the desktop engines

Besides the GPL modules above, the three desktop sidecars statically link
about a hundred more Go modules (for example `gvisor.dev/gvisor`,
`golang.org/x/crypto`, `github.com/miekg/dns`, `github.com/apernet/quic-go`)
under permissive licences such as MIT, BSD and Apache-2.0. They are not yet
listed one by one here or on the "Open-source licences" screen; each binary
names its modules and versions in `go version -m <binary>`, and each module's
licence is the one in its upstream repository at that version.

## LGPL libraries inside the Linux AppImage

The AppImage (`ProxysVPN_<version>_amd64.AppImage`, docs/STEAMDECK.md) carries
the desktop toolkit with it, because SteamOS and many other systems do not
have WebKitGTK 4.1 installed. linuxdeploy copies WebKitGTK, GTK 3 and the
libraries they link from the CI image (Ubuntu 22.04) into the image's
`usr/lib`, leaving the base system (glibc, libstdc++, GL, X11, fontconfig,
freetype, harfbuzz, fribidi, libgpg-error and a few more) to the machine it
runs on. The `.deb` carries none of them: it depends on the distribution's
own packages.

They are **shared libraries, linked dynamically**: the app loads them from
`usr/lib` when it starts and does not contain any of their code. Each can be
replaced with another build of the same interface: extract the image
(`./ProxysVPN_<version>_amd64.AppImage --appimage-extract`), put your build of
the library under the same file name into `squashfs-root/usr/lib` (or delete
ours, and the system's is used), and run `squashfs-root/AppRun`. The files in
the image are Ubuntu 22.04's builds of the version current when CI made it,
unmodified; their complete corresponding source is Ubuntu's source package of
that version (`apt-get source <package>=<version>`, or the Launchpad page
below), and the CI step "AppImage carries the helper" prints the libraries the
image holds. The licence texts ship inside the app and the libraries are
listed on its "Open-source licences" screen for Linux, marked "AppImage only".

| Library | Files | Licence | Upstream | Ubuntu 22.04 source |
|---|---|---|---|---|
| WebKitGTK | libwebkit2gtk-4.1, libjavascriptcoregtk-4.1 | LGPL-2.1-only AND BSD-2-Clause | https://webkitgtk.org/releases/ | [webkit2gtk](https://launchpad.net/ubuntu/jammy/+source/webkit2gtk) |
| GTK 3 | libgtk-3, libgdk-3 | LGPL-2.0-or-later | https://gitlab.gnome.org/GNOME/gtk | [gtk+3.0](https://launchpad.net/ubuntu/jammy/+source/gtk+3.0) |
| GLib | libglib-2.0, libgobject-2.0, libgio-2.0, libgmodule-2.0 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/glib | [glib2.0](https://launchpad.net/ubuntu/jammy/+source/glib2.0) |
| GDK-Pixbuf | libgdk_pixbuf-2.0 and its loaders | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/gdk-pixbuf | [gdk-pixbuf](https://launchpad.net/ubuntu/jammy/+source/gdk-pixbuf) |
| Pango | libpango-1.0, libpangocairo-1.0, libpangoft2-1.0 | LGPL-2.0-or-later | https://gitlab.gnome.org/GNOME/pango | [pango1.0](https://launchpad.net/ubuntu/jammy/+source/pango1.0) |
| cairo | libcairo, libcairo-gobject | LGPL-2.1-only OR MPL-1.1 | https://gitlab.freedesktop.org/cairo/cairo | [cairo](https://launchpad.net/ubuntu/jammy/+source/cairo) |
| ATK | libatk-1.0 | LGPL-2.0-or-later | https://gitlab.gnome.org/GNOME/atk | [atk1.0](https://launchpad.net/ubuntu/jammy/+source/atk1.0) |
| AT-SPI | libatspi, libatk-bridge-2.0 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/at-spi2-core | [at-spi2-core](https://launchpad.net/ubuntu/jammy/+source/at-spi2-core) |
| libsoup | libsoup-3.0 | LGPL-2.0-or-later AND LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/libsoup | [libsoup3](https://launchpad.net/ubuntu/jammy/+source/libsoup3) |
| GStreamer | libgstreamer-1.0, the gst-plugins-base and gst-plugins-bad libraries | LGPL-2.1-or-later | https://gitlab.freedesktop.org/gstreamer/gstreamer | [gstreamer1.0](https://launchpad.net/ubuntu/jammy/+source/gstreamer1.0) |
| librsvg | librsvg-2 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/librsvg | [librsvg](https://launchpad.net/ubuntu/jammy/+source/librsvg) |
| libsecret | libsecret-1 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/libsecret | [libsecret](https://launchpad.net/ubuntu/jammy/+source/libsecret) |
| Enchant | libenchant-2 | LGPL-2.0-or-later | https://github.com/rrthomas/enchant | [enchant-2](https://launchpad.net/ubuntu/jammy/+source/enchant-2) |
| Hyphen | libhyphen | LGPL-2.1-or-later OR MPL-1.1 (upstream also offers GPL-2.0-only) | https://github.com/hunspell/hyphen | [hyphen](https://launchpad.net/ubuntu/jammy/+source/hyphen) |
| libmanette | libmanette-0.2 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/libmanette | [libmanette](https://launchpad.net/ubuntu/jammy/+source/libmanette) |
| libgudev | libgudev-1.0 | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/libgudev | [libgudev](https://launchpad.net/ubuntu/jammy/+source/libgudev) |
| libseccomp | libseccomp | LGPL-2.1-only | https://github.com/seccomp/libseccomp | [libseccomp](https://launchpad.net/ubuntu/jammy/+source/libseccomp) |
| libtasn1 | libtasn1 | LGPL-2.1-or-later | https://gitlab.com/gnutls/libtasn1 | [libtasn1-6](https://launchpad.net/ubuntu/jammy/+source/libtasn1-6) |
| Libgcrypt | libgcrypt | LGPL-2.1-or-later | https://gnupg.org/software/libgcrypt/ | [libgcrypt20](https://launchpad.net/ubuntu/jammy/+source/libgcrypt20) |
| systemd | libsystemd | LGPL-2.1-or-later | https://github.com/systemd/systemd | [systemd](https://launchpad.net/ubuntu/jammy/+source/systemd) |
| libthai | libthai | LGPL-2.1-or-later | https://github.com/tlwg/libthai | [libthai](https://launchpad.net/ubuntu/jammy/+source/libthai) |
| libdatrie | libdatrie | LGPL-2.1-or-later | https://github.com/tlwg/libdatrie | [libdatrie](https://launchpad.net/ubuntu/jammy/+source/libdatrie) |
| util-linux | libmount, libblkid | LGPL-2.1-or-later | https://github.com/util-linux/util-linux | [util-linux](https://launchpad.net/ubuntu/jammy/+source/util-linux) |

Licences are the projects' own, as their SPDX expressions in Fedora's packages
state them; WebKitGTK also contains code under further permissive licences,
listed in its source tree. Permissively licensed libraries in the image
(libxml2, libxslt, libwebp, libjpeg-turbo, libpng, pixman, libepoxy, WPE and
others) are not yet listed one by one here or on the screen. The list is kept
in `scripts/gen-notices.mjs` (`APPIMAGE_LIBRARIES`).

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
