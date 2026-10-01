// src/platformCopy.ts
//
// Which sentence about DNS each platform may say. Pure, with no build-time
// input: node --test imports it as is (tests/platformCopy.test.ts), and the
// screens pass in what they know (the platform from `app_info`, the channel
// from src/dist.ts).
//
// The resolvers differ by engine and by what the tunnel does with the
// system's own lookups:
//   • iOS — the Apple engine (xray_apple.rs) asks Cloudflare over HTTPS
//     through the server, sends names routed direct to Yandex's public DNS,
//     and looks up the server's own name with the network's resolver.
//   • macOS — sysdns.rs points the system resolver at the in-tunnel address
//     xray answers; Linux publishes the same address on the tunnel device
//     (net/linux_priv.rs). Cloudflare and Google answer through the server.
//   • Windows — the system resolver is left alone (net/windows.rs) and the
//     tunnel carries IPv4 only, so lookups to a resolver on the local network
//     and IPv6 traffic go outside it. Until that changes, Windows gets its
//     own sentence, and the Tunnel screen says its DNS and address-family
//     choices reach only the engine itself.

/** The platforms `app_info` reports (src/bridge.ts AppInfo["platform"]). */
export type CopyPlatform = "macos" | "ios" | "windows" | "linux" | "other";

export type DnsNoticeKey = "notice.dns.apple" | "notice.dns.desktop" | "notice.dns.windows";

/**
 * The DNS line of the data notice. Before `app_info` answers, the build
 * decides: every App Store build today is iOS, the direct build a desktop one.
 */
export function dnsNoticeKey(platform: CopyPlatform | undefined, isAppStore: boolean): DnsNoticeKey {
  switch (platform) {
    case "ios":
      return "notice.dns.apple";
    case "windows":
      return "notice.dns.windows";
    case "macos":
    case "linux":
      return "notice.dns.desktop";
    default:
      return isAppStore ? "notice.dns.apple" : "notice.dns.desktop";
  }
}

export interface TunnelHintKeys {
  ip: "tun.ipHint" | "tun.ipHintWindows";
  dns: "tun.dnsHint" | "tun.dnsHintWindows";
}

/** The hints under "Тип адресов" and "Резолвер" on the Tunnel screen. */
export function tunnelHintKeys(platform: CopyPlatform | undefined): TunnelHintKeys {
  if (platform === "windows") {
    return { ip: "tun.ipHintWindows", dns: "tun.dnsHintWindows" };
  }
  return { ip: "tun.ipHint", dns: "tun.dnsHint" };
}
