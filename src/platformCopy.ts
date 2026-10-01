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
//     own sentence, and the Tunnel screen says its DNS choice reaches only
//     the engine itself.
//   • IPv6 — no desktop tunnel carries it (macOS and Linux route only the two
//     IPv4 halves), so the desktop sentence says so as well.

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

/**
 * Whether the Tunnel screen offers the IPv4 / IPv6 / Both choice at all.
 *
 * Nowhere today. iOS never did (the Apple engine fixes the policy itself).
 * On desktop no tunnel carries IPv6 yet — macOS and Linux route only the two
 * IPv4 halves, Windows leaves IPv6 alone — while macOS and Linux point the
 * system resolver at the engine: "IPv6" or "Both" handed out AAAA records and
 * sent nearly all traffic of a dual-stack network outside the VPN with the
 * shield still green. The core ignores a stored choice on desktop for the
 * same reason (tunnel_prefs.rs IpKind::effective). Turn it back on per
 * platform once its tunnel captures IPv6.
 */
export function offersIpFamilyChoice(platform: CopyPlatform | undefined): boolean {
  switch (platform) {
    case "ios":
    case "macos":
    case "linux":
    case "windows":
    case "other":
    case undefined:
      return false;
  }
}
