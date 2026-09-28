// VpnBridge.swift
// App-side VPN control: drives the PacketTunnel Network Extension through
// NETunnelProviderManager and exposes a tiny C ABI consumed by the Rust core
// (src-tauri/src/ios_vpn.rs). Rust and this file are linked into the same
// binary, so @_cdecl symbols resolve at link time — no headers needed.
//
// Contract with Rust:
//   pvpn_tunnel_start(config_json)  — save NE profile + start tunnel (async,
//                                     returns immediately; Rust polls status)
//   pvpn_tunnel_stop()              — stop tunnel
//   pvpn_tunnel_status() -> i32     — NEVPNStatus rawValue; -2 while the
//                                     manager is still loading preferences
//   pvpn_tunnel_last_error()        — strdup'ed message or NULL, consumed once
//   pvpn_string_free(ptr)           — frees strings returned above

import Foundation
import NetworkExtension
import os.log

private let bridgeLog = OSLog(subsystem: "com.proxysvpn.desktop", category: "VpnBridge")

final class VpnController {
    static let shared = VpnController()

    /// PRODUCT_BUNDLE_IDENTIFIER of the PacketTunnel target. project.yml
    /// derives it as the app's id + ".PacketTunnel" (Apple requires the
    /// prefix), so deriving it here too keeps a bundle id switch to the one
    /// line in project.yml instead of a second literal to forget.
    static let providerBundleId = (Bundle.main.bundleIdentifier ?? "") + ".PacketTunnel"

    private enum LoadState { case notLoaded, loading, loaded }

    private let lock = NSLock()
    private var manager: NETunnelProviderManager?
    private var loadState: LoadState = .notLoaded
    private var lastError: String?

    private init() {}

    // MARK: - Error slot

    private func setError(_ message: String?) {
        lock.lock()
        lastError = message
        lock.unlock()
        if let message {
            os_log("error: %{public}@", log: bridgeLog, type: .error, message)
        }
    }

    func takeError() -> String? {
        lock.lock()
        defer { lock.unlock() }
        let err = lastError
        lastError = nil
        return err
    }

    // MARK: - Preferences loading

    /// Loads the existing NE profile (if any). Safe to call repeatedly.
    private func beginLoad(completion: ((NETunnelProviderManager?) -> Void)? = nil) {
        lock.lock()
        if loadState == .loading {
            lock.unlock()
            completion?(nil)
            return
        }
        loadState = .loading
        lock.unlock()

        NETunnelProviderManager.loadAllFromPreferences { [self] managers, error in
            if let error {
                setError("load VPN preferences: \(error.localizedDescription)")
            }
            let ours = managers?.first(where: {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?
                    .providerBundleIdentifier == Self.providerBundleId
            })
            lock.lock()
            manager = ours
            loadState = .loaded
            lock.unlock()
            completion?(ours)
        }
    }

    // MARK: - Public API

    func statusRaw() -> Int32 {
        lock.lock()
        let state = loadState
        let mgr = manager
        lock.unlock()

        switch state {
        case .notLoaded:
            beginLoad()
            return -2
        case .loading:
            return -2
        case .loaded:
            guard let mgr else {
                return Int32(NEVPNStatus.disconnected.rawValue)
            }
            return Int32(mgr.connection.status.rawValue)
        }
    }

    func start(configJSON: String) {
        setError(nil)

        NETunnelProviderManager.loadAllFromPreferences { [self] managers, error in
            if let error {
                setError("load VPN preferences: \(error.localizedDescription)")
                return
            }
            let mgr = managers?.first(where: {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?
                    .providerBundleIdentifier == Self.providerBundleId
            }) ?? NETunnelProviderManager()

            let proto = NETunnelProviderProtocol()
            proto.providerBundleIdentifier = Self.providerBundleId
            proto.serverAddress = "ProxysVPN"
            proto.providerConfiguration = ["config": configJSON as NSString]
            mgr.protocolConfiguration = proto
            mgr.localizedDescription = "ProxysVPN"
            mgr.isEnabled = true

            mgr.saveToPreferences { [self] error in
                if let error {
                    let ns = error as NSError
                    if ns.domain == NEVPNErrorDomain,
                       ns.code == NEVPNError.Code.configurationReadWriteFailed.rawValue {
                        // The one-time "Allow VPN configuration" dialog was rejected.
                        setError("VPN configuration was not allowed")
                    } else {
                        setError("save VPN profile: \(error.localizedDescription)")
                    }
                    return
                }
                // Apple requires a reload between save and start.
                mgr.loadFromPreferences { [self] error in
                    if let error {
                        setError("reload VPN profile: \(error.localizedDescription)")
                        return
                    }
                    lock.lock()
                    manager = mgr
                    loadState = .loaded
                    lock.unlock()
                    do {
                        try mgr.connection.startVPNTunnel()
                    } catch {
                        setError("start tunnel: \(error.localizedDescription)")
                    }
                }
            }
        }
    }

    func stop() {
        lock.lock()
        let state = loadState
        let mgr = manager
        lock.unlock()

        switch state {
        case .loaded:
            mgr?.connection.stopVPNTunnel()
        default:
            beginLoad { $0?.connection.stopVPNTunnel() }
        }
    }
}

// MARK: - C ABI for Rust

@_cdecl("pvpn_tunnel_start")
public func pvpn_tunnel_start(_ configJSON: UnsafePointer<CChar>?) {
    guard let configJSON else { return }
    VpnController.shared.start(configJSON: String(cString: configJSON))
}

@_cdecl("pvpn_tunnel_stop")
public func pvpn_tunnel_stop() {
    VpnController.shared.stop()
}

@_cdecl("pvpn_tunnel_status")
public func pvpn_tunnel_status() -> Int32 {
    VpnController.shared.statusRaw()
}

@_cdecl("pvpn_tunnel_last_error")
public func pvpn_tunnel_last_error() -> UnsafeMutablePointer<CChar>? {
    guard let err = VpnController.shared.takeError() else { return nil }
    return strdup(err)
}

@_cdecl("pvpn_string_free")
public func pvpn_string_free(_ ptr: UnsafeMutablePointer<CChar>?) {
    free(ptr)
}
