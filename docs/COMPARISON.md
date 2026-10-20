# ProxysVPN Desktop compared with Happ, v2RayTun, Hiddify and NekoBox

Five clients that people in Russia and abroad use to reach the open internet
over VLESS, Hysteria and their relatives, side by side. The table says what
each one is and does, as its own page or store listing states it, and stops
there: which one is right depends on whether you run your own server or use
a service, and on your devices.

Checked on 21 October 2026. Where a maker's page does not say something, the
cell says "not stated" rather than guessing. If a cell is out of date, open an
issue in this repository with the maker's page that proves it.

## The one difference that decides most choices

**Happ, v2RayTun, Hiddify and NekoBox are general clients.** You bring a
server or a subscription from any provider, paste or scan it, and the client
connects. Each of them says so itself: Happ "does not include any servers",
v2RayTun "does not provide a VPN service", Hiddify is "compatible with all
proxy management panels".

**ProxysVPN Desktop is the client of one service.** It signs in to a
ProxysVPN account with a pairing code and gets the service's nodes, routing
rules and protocols from there. It cannot take a server from another
provider. In return the set-up has no links or configuration files, and the
split routing the service maintains (Russian sites go straight out) is on from
the first connection.

## Side by side

| | ProxysVPN Desktop | Happ | v2RayTun | Hiddify | NekoBox |
|---|---|---|---|---|---|
| **Works with** | The ProxysVPN service only | Any server or subscription | Any server or subscription | Any server or subscription; "compatible with all proxy management panels" | Any server or subscription |
| **Platforms** (as the maker lists them) | macOS 12+ (Apple Silicon and Intel), Windows 10/11 (x64 and ARM64), Linux x86_64 (`.deb`, AppImage; an aarch64 AppImage is on the Releases page too), Steam Deck (AppImage); Android APK on the Releases page; iPhone, iPad and Apple TV through the links on [proxysvpn.com/desktop](https://proxysvpn.com/desktop) | Windows, macOS, Linux; iOS, Android; tvOS, Android TV | Android (Google Play), iPhone and iPad (App Store, iOS 16+), Windows (in winget as `v2RayTun.Windows`) | Android, iOS, Windows, macOS, Linux | Android 5.0+ only |
| **Protocols** | VLESS Reality / XHTTP (Xray-core), Hysteria2, and the service's own WataFast protocol; nothing else, because these are what the service runs | VLESS (Reality), VMess, Trojan, Shadowsocks, Socks, Hysteria2 | VLESS, VMess, Trojan, Shadowsocks, SOCKS; Reality | sing-box core: VLESS, VMess, Reality, TUIC, Hysteria, WireGuard, SSH "etc." | sing-box core: SOCKS, HTTP(S), SSH, Shadowsocks, VMess, Trojan, VLESS, AnyTLS, ShadowTLS, TUIC, Hysteria 1/2, WireGuard; Trojan-Go, NaïveProxy and Mieru through plugins |
| **System-wide tunnel on the computer** | Yes, on all three systems: a TUN device and tun2socks, with administrator rights (password on macOS, UAC on Windows, polkit on Linux). IPv4 only; IPv6 goes into the tunnel but no node has an IPv6 exit | Not stated on the official page | Not stated on the official page | "TUN mode" | VPN mode on Android |
| **Split routing** | Built in and maintained by the service: Russian sites and anything on your "always direct" list go straight out; a Rules screen edits the list; the same rules apply on every protocol | "Configuration of proxy servers based on flexible routing rules" | "DNS and route configuration" | Not stated in the README | Routing rules inside a subscription are ignored; own rules not stated in the README |
| **Set-up** | An 8-character pairing code from the account or the bot, the round code on the app's screen scanned from a phone, or the account's link. One account for all devices | Your own server or a subscription; routing rules | Your own server or a subscription; import by QR code, clipboard or link | Subscription formats sing-box, V2ray, Clash, Clash Meta; automatic subscription update; remaining days and traffic shown | Formats Shadowsocks SIP008, Clash / Clash Meta, v2rayN, sing-box outbound; nodes only |
| **Source code** | Not published. This repository holds the installers, the release notes, the third-party notices and the source of our changes to Xray-core (MPL-2.0) | No public source repository that we could find; licence not stated | No public source repository that we could find; licence not stated | Open: GPLv3 with Hiddify's additional conditions, among them no commercial use without written consent | Open: GPL-3.0. The README warns that the Google Play build "has been controlled by a third party since May 2024" and is not open source |
| **Price** | The app is free; the ProxysVPN service is prepaid, and that is what you pay for | Free (App Store); the site takes donations | Free with in-app purchases (App Store) | Free; "ad-free and open-source" | Free software; the README asks for donations |
| **Code signing** | macOS: signed with an Apple Developer ID (Watafast LLC) and notarized since 0.3.7. Windows and Linux: not signed yet; SHA-256 sums on every release, and the built-in updater installs only a file that matches a release document signed with our key | Not stated | Not stated | Not stated | Not stated |
| **Telemetry** | None. The "Privacy" section of the README lists every address the app talks to | Not stated | App Store privacy label: data not collected | Not stated | Not stated |
| **Updates** | The app announces a new version itself and installs it after you press Install (from 0.3.9) | Through the stores and the maker's site | Through the stores | GitHub releases, stores | GitHub releases |
| **Last code change on GitHub** | Not public | Not public | Not public | 17 October 2026 | 9 February 2026 |

## Where ProxysVPN Desktop is the narrower choice

- It works with one service. If you run your own server, or keep subscriptions
  from several providers, you need a general client.
- IPv4 only: a program that connects to an IPv6 address on its own, with
  anything but HTTP or HTTPS, gets no connection over IPv6 while the VPN is on.
- The Windows and Linux installers are not code-signed yet; Windows warns before
  it runs them. Compare the SHA-256 with `SHA256SUMS.txt` on the release page.
- No Flatpak, no Homebrew cask yet. Linux gets a `.deb` and an AppImage.
- Some builds have not been run on real hardware by us yet; each release's
  notes name them (as of 0.3.14: the Intel Mac, the Steam Deck).
- Russian and English interface only.
- The code is not public. What is public: the installers, the release notes
  with their known limitations, the third-party notices, and our Xray-core
  patches.

## Where it differs from the general clients

- No links and no files: a pairing code, or the round code scanned from the
  phone, and the account's nodes are there.
- The service's split routing and its "always direct" list, kept up by the
  service and applied on every protocol, because xray is always the front of
  the chain.
- Three transports from one account: VLESS Reality / XHTTP, Hysteria2 and the
  service's own WataFast protocol. The app swaps a node that stops getting
  through for another and restarts a dead engine itself, and says so in the
  window.
- One account on the phone, the computer, the TV and the Steam Deck.

## Sources

- ProxysVPN Desktop: [README](../README.md), [RELEASE_NOTES.md](../RELEASE_NOTES.md),
  [proxysvpn.com/desktop](https://proxysvpn.com/desktop).
- Happ: [happ.su](https://www.happ.su/) (platforms, routing rules, "does not include
  any servers"); [App Store listing](https://apps.apple.com/app/happ-proxy-utility/id6504287215)
  (protocols, price).
- v2RayTun: [App Store listing](https://apps.apple.com/app/id6476628951) (protocols,
  price, "does not provide a VPN service"); [winget-pkgs](https://github.com/microsoft/winget-pkgs/tree/master/manifests/v/v2RayTun/Windows)
  (the Windows build).
- Hiddify: [README](https://github.com/hiddify/hiddify-app#readme) (platforms,
  protocols, features); [LICENSE.md](https://github.com/hiddify/hiddify-app/blob/main/LICENSE.md)
  (GPLv3 with additional conditions).
- NekoBox: [README](https://github.com/MatsuriDayo/NekoBoxForAndroid#readme)
  (protocols, subscription formats, the Google Play warning, GPL-3.0).
- "Last code change on GitHub" is the repository's push date as the GitHub API
  reported it on 21 October 2026.
