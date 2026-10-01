// PacketTunnelProvider.swift
// The VPN engine on iOS: an NEPacketTunnelProvider hosting Xray-core, linked
// from LibXray.xcframework (scripts/build-libxray.sh). Replaces the desktop
// xray/hysteria/tun2socks process chain - iOS allows neither child processes
// nor route edits - and, since 28.09.2026, the sing-box engine (no XHTTP,
// GPL-3.0; docs/IOS.md).
//
// The Rust core builds everything this file needs (src-tauri/src/xray_apple.rs)
// and it arrives as NETunnelProviderProtocol.providerConfiguration["config"]:
//   {"version": 1, "xray": {Xray config}, "tunnel": {addresses, routes, DNS},
//    "ipv6Only": {what changes on a network without IPv4}}
// This file only adapts the Xray half to an IPv6-only network when it is on
// one (Ipv6OnlyNetwork.swift), applies the tunnel half to the system, finds
// the utun file descriptor, hands it to Xray as env["xray.tun.fd"] and starts
// the engine.
//
// One Go runtime per process: LibXray is the only Go library this extension
// may link. A second one (Libbox, a separate Hysteria build) crashes at load.

import Darwin
import Foundation
import LibXray
import NetworkExtension
import os.log

// The extension's own bundle id (the app's id + ".PacketTunnel", project.yml)
// rather than a literal, so Console.app filters by the id the build carries.
private let extensionBundleId = Bundle.main.bundleIdentifier ?? "PacketTunnel"
private let tunnelLog = OSLog(subsystem: extensionBundleId, category: "Tunnel")

/// The configuration format this build understands (xray_apple.rs).
private let configurationVersion = 1

class PacketTunnelProvider: NEPacketTunnelProvider {
    private let engine = XrayEngine()
    private var memoryTimer: DispatchSourceTimer?

    override func startTunnel(options _: [String: NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        let configuration: TunnelConfiguration
        do {
            configuration = try TunnelConfiguration(protocolConfiguration)
        } catch {
            os_log("bad configuration: %{public}@", log: tunnelLog, type: .error, error.localizedDescription)
            completionHandler(error)
            return
        }

        // Now, while the process still sees only the physical network: once
        // the tunnel's IPv4 address exists, an IPv6-only network no longer
        // looks like one to getaddrinfo.
        var startConfig = configuration.xray
        if let outcome = Ipv6OnlyNetwork.adapt(startConfig, plan: configuration.ipv6Only,
                                               synthesize: Nat64.synthesize) {
            startConfig = outcome.xray
            os_log("IPv6-only network: %{public}d addresses mapped through NAT64, %{public}d direct routes through the node",
                   log: tunnelLog, type: .info, outcome.mapped, outcome.rerouted)
            if outcome.skipped > 0 {
                os_log("IPv6-only plan did not match the config at %{public}d places", log: tunnelLog, type: .fault,
                       outcome.skipped)
            }
        }
        let xrayConfig = startConfig

        setTunnelNetworkSettings(configuration.networkSettings()) { [weak self] error in
            guard let self else {
                completionHandler(TunnelError.engine("provider released during start"))
                return
            }
            if let error {
                os_log("network settings refused: %{public}@", log: tunnelLog, type: .error,
                       error.localizedDescription)
                completionHandler(error)
                return
            }
            do {
                // The utun exists only once the settings are applied.
                guard let fd = Self.tunnelFileDescriptor() else { throw TunnelError.noTunnelDescriptor }
                var xray = xrayConfig
                // Xray copies root "env" into the process environment before
                // it builds the tun inbound, which then reads this descriptor
                // instead of creating an interface (Xray-core proxy/tun).
                xray["env"] = ["xray.tun.fd": String(fd)]
                try self.engine.start(xray)
                os_log("xray %{public}@ started", log: tunnelLog, type: .info, self.engine.version())
                self.startMemoryLog()
                completionHandler(nil)
            } catch {
                // Engine errors may quote the config, which holds the node and
                // the account id: private in the unified log.
                os_log("start failed: %{private}@", log: tunnelLog, type: .error, error.localizedDescription)
                completionHandler(error)
            }
        }
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        os_log("stopping (reason %{public}ld)", log: tunnelLog, type: .info, reason.rawValue)
        memoryTimer?.cancel()
        memoryTimer = nil
        engine.stop()
        completionHandler()
    }

    // MARK: - utun descriptor

    /// The utun fd behind packetFlow, which Xray reads and writes directly.
    /// NetworkExtension does not expose it, so scan the open fds for a
    /// kernel-control socket whose interface name is utunN - the method
    /// Xray-core documents for iOS. The headers that define these two
    /// constants (<sys/sys_domain.h>, <net/if_utun.h>) are not visible to
    /// Swift on iOS, hence the literals.
    private static func tunnelFileDescriptor() -> Int32? {
        let sysprotoControl: Int32 = 2 // SYSPROTO_CONTROL
        let utunOptIfname: Int32 = 2 // UTUN_OPT_IFNAME
        var name = [CChar](repeating: 0, count: Int(IFNAMSIZ))
        for fd: Int32 in 0 ... 1024 {
            var len = socklen_t(name.count)
            guard getsockopt(fd, sysprotoControl, utunOptIfname, &name, &len) == 0 else {
                continue
            }
            let bytes = name.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }
            if String(decoding: bytes, as: UTF8.self).hasPrefix("utun") {
                return fd
            }
        }
        return nil
    }

    // MARK: - Memory log

    /// The system kills a packet tunnel extension that outgrows its limit
    /// (~50 MiB on iOS; unmeasured on tvOS). libXray caps the Go heap at
    /// 30 MiB; this logs what the whole process really holds, every 30 s at
    /// info level, so a device soak test can read it in Console.app
    /// (subsystem = this extension's bundle id, category "Memory").
    private func startMemoryLog() {
        memoryTimer?.cancel()
        let timer = DispatchSource.makeTimerSource(queue: DispatchQueue(label: extensionBundleId + ".memory"))
        timer.schedule(deadline: .now(), repeating: .seconds(30), leeway: .seconds(5))
        timer.setEventHandler { MemoryReport.log() }
        timer.resume()
        memoryTimer = timer
    }
}

// MARK: - Configuration

private enum TunnelError: LocalizedError {
    case missingConfiguration
    case unsupportedVersion(Int)
    case malformedConfiguration(String)
    case noTunnelDescriptor
    case engine(String)

    var errorDescription: String? {
        switch self {
        case .missingConfiguration:
            return "missing tunnel configuration in the VPN profile"
        case let .unsupportedVersion(version):
            return "tunnel configuration version \(version) is not supported by this build"
        case let .malformedConfiguration(what):
            return "malformed tunnel configuration: \(what)"
        case .noTunnelDescriptor:
            return "could not locate the utun file descriptor"
        case let .engine(message):
            return "xray: \(message)"
        }
    }
}

/// The tunnel half of the configuration (xray_apple.rs `TunnelSettings`).
private struct TunnelSettings: Decodable {
    struct IPv4Route: Decodable {
        let address: String
        let mask: String
    }

    struct IPv6Route: Decodable {
        let address: String
        let prefix: Int
    }

    let ipv4Address: String
    let ipv4Mask: String
    let ipv4Excluded: [IPv4Route]
    let ipv6Address: String
    let ipv6Prefix: Int
    let ipv6Excluded: [IPv6Route]
    let dnsServers: [String]
    let mtu: Int
}

private struct TunnelConfiguration {
    let xray: [String: Any]
    let tunnel: TunnelSettings
    let ipv6Only: Ipv6OnlyPlan

    init(_ protocolConfiguration: NEVPNProtocol) throws {
        guard let proto = protocolConfiguration as? NETunnelProviderProtocol,
              let text = proto.providerConfiguration?["config"] as? String
        else { throw TunnelError.missingConfiguration }
        guard let root = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else {
            throw TunnelError.malformedConfiguration("not an object")
        }
        let version = root["version"] as? Int ?? 0
        guard version == configurationVersion else { throw TunnelError.unsupportedVersion(version) }
        guard let xray = root["xray"] as? [String: Any] else {
            throw TunnelError.malformedConfiguration("no xray config")
        }
        guard let tunnelObject = root["tunnel"] else {
            throw TunnelError.malformedConfiguration("no tunnel settings")
        }
        let decoded: TunnelSettings
        do {
            let tunnelData = try JSONSerialization.data(withJSONObject: tunnelObject)
            decoded = try JSONDecoder().decode(TunnelSettings.self, from: tunnelData)
        } catch {
            throw TunnelError.malformedConfiguration("tunnel settings: \(error.localizedDescription)")
        }
        guard !decoded.dnsServers.isEmpty, decoded.mtu >= 1280 else {
            throw TunnelError.malformedConfiguration("tunnel settings out of range")
        }
        self.xray = xray
        tunnel = decoded
        // Absent in a profile saved by an earlier build. A malformed one is a
        // core/extension mismatch: the tunnel still starts, as it did before
        // the plan existed, and says so.
        if let raw = root["ipv6Only"] {
            if let plan = Ipv6OnlyPlan(json: raw) {
                ipv6Only = plan
            } else {
                os_log("malformed IPv6-only plan ignored", log: tunnelLog, type: .fault)
                ipv6Only = .empty
            }
        } else {
            ipv6Only = .empty
        }
    }

    /// IPv4 and IPv6 both routed into the tunnel by default, the local
    /// networks excluded, and every name resolved by the tunnel's resolver,
    /// which Xray answers (xray_apple.rs). Routing IPv6 in as well is what
    /// keeps it from leaking past the tunnel on a dual-stack network.
    func networkSettings() -> NEPacketTunnelNetworkSettings {
        // Only a label (Settings > VPN): the provider's own traffic never
        // enters the tunnel whatever this says.
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
        settings.mtu = NSNumber(value: tunnel.mtu)

        let ipv4 = NEIPv4Settings(addresses: [tunnel.ipv4Address], subnetMasks: [tunnel.ipv4Mask])
        ipv4.includedRoutes = [NEIPv4Route.default()]
        ipv4.excludedRoutes = tunnel.ipv4Excluded.map {
            NEIPv4Route(destinationAddress: $0.address, subnetMask: $0.mask)
        }
        settings.ipv4Settings = ipv4

        let ipv6 = NEIPv6Settings(addresses: [tunnel.ipv6Address],
                                  networkPrefixLengths: [NSNumber(value: tunnel.ipv6Prefix)])
        ipv6.includedRoutes = [NEIPv6Route.default()]
        ipv6.excludedRoutes = tunnel.ipv6Excluded.map {
            NEIPv6Route(destinationAddress: $0.address, networkPrefixLength: NSNumber(value: $0.prefix))
        }
        settings.ipv6Settings = ipv6

        let dns = NEDNSSettings(servers: tunnel.dnsServers)
        dns.matchDomains = [""] // every name, not just some domains
        settings.dnsSettings = dns
        return settings
    }
}

// MARK: - Engine

/// libXray's single C entry point: CGoInvoke takes a JSON request and returns
/// a JSON response that only CGoFree may release (libXray README).
private final class XrayEngine {
    func start(_ config: [String: Any]) throws {
        let data = try JSONSerialization.data(withJSONObject: config)
        guard let text = String(data: data, encoding: .utf8) else {
            throw TunnelError.malformedConfiguration("xray config is not UTF-8")
        }
        _ = try invoke("runXray", payload: ["xrayJson": text])
    }

    func stop() {
        do {
            _ = try invoke("stopXray", payload: [:])
        } catch {
            os_log("stop failed: %{private}@", log: tunnelLog, type: .error, error.localizedDescription)
        }
    }

    func version() -> String {
        let data = (try? invoke("xrayVersion", payload: [:])) as? [String: Any]
        return data?["version"] as? String ?? "?"
    }

    private func invoke(_ method: String, payload: [String: Any]) throws -> Any? {
        let request: [String: Any] = ["apiVersion": 3, "method": method, "payload": payload]
        let requestData = try JSONSerialization.data(withJSONObject: request)
        guard let requestText = String(data: requestData, encoding: .utf8) else {
            throw TunnelError.engine("request is not UTF-8")
        }
        let responseText: String = try requestText.withCString { pointer in
            // Go copies the request (C.GoString) and never writes to it.
            guard let raw = CGoInvoke(UnsafeMutablePointer(mutating: pointer)) else {
                throw TunnelError.engine("\(method): no response")
            }
            defer { CGoFree(raw) }
            return String(cString: raw)
        }
        guard let response = try JSONSerialization.jsonObject(with: Data(responseText.utf8)) as? [String: Any] else {
            throw TunnelError.engine("\(method): unreadable response")
        }
        guard response["success"] as? Bool == true else {
            throw TunnelError.engine("\(method): \(response["error"] as? String ?? "failed")")
        }
        return response["data"]
    }
}

// MARK: - Memory

private enum MemoryReport {
    private static let memoryLog = OSLog(subsystem: extensionBundleId, category: "Memory")

    static func log() {
        let mib = 1024.0 * 1024.0
        let footprint = physicalFootprint().map { Double($0) / mib } ?? -1
        #if os(iOS) || os(tvOS)
        // What the process may still allocate before the system kills it.
        let available = Double(os_proc_available_memory()) / mib
        #else
        let available = -1.0
        #endif
        os_log("footprint %{public}.1f MiB, available %{public}.1f MiB",
               log: memoryLog, type: .info, footprint, available)
    }

    /// phys_footprint, the figure the system's memory limit is checked
    /// against (what Xcode's memory gauge shows).
    private static func physicalFootprint() -> UInt64? {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.phys_footprint : nil
    }
}
