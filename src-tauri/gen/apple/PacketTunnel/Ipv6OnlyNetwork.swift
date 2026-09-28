// Ipv6OnlyNetwork.swift
// What the extension changes in the Xray config before starting it on a
// network without IPv4: an IPv6-only network with NAT64, which is what App
// Review tests on (guideline 2.5.5) and what some carriers run.
//
// Go opens a socket to an IPv4 literal as AF_INET and has no NAT64 of its
// own, so there a node written as an address and the Yandex resolver would be
// unreachable, and every route that leaves "direct" to an IPv4 address would
// be a dead end. getaddrinfo is what knows the network's NAT64 prefix: asked
// about an IPv4 literal with AI_DEFAULT it answers with the synthesized IPv6
// address on such a network and with the literal itself wherever IPv4 works
// (Apple, "Supporting IPv6-only Networks").
//
// The Rust core says where the literals sit and which routes go direct
// (src-tauri/src/xray_apple.rs `Ipv6OnlyPlan`); this file only asks the
// network and applies the answer. Everything but `Nat64.synthesize` is pure,
// so tests/packet-tunnel runs it on the Mac with a fake network
// (scripts/test-packet-tunnel.sh).

import Darwin
import Foundation

/// One step of a path into the Xray config: an object key or an array index.
enum ConfigPathStep: Equatable {
    case key(String)
    case index(Int)
}

typealias ConfigPath = [ConfigPathStep]

/// The plan the Rust core writes next to the Xray config (`"ipv6Only"`).
struct Ipv6OnlyPlan: Equatable {
    struct Literal: Equatable {
        let ipv4: String
        /// String values equal to `ipv4`, replaced by the synthesized address.
        let replace: [ConfigPath]
        /// Address lists the synthesized address is added to, next to `ipv4`.
        let append: [ConfigPath]
    }

    let literals: [Literal]
    /// `outboundTag`s that go from "direct" to "proxy".
    let viaNode: [ConfigPath]

    /// A profile saved by a build older than the plan carries none.
    static let empty = Ipv6OnlyPlan(literals: [], viaNode: [])

    init(literals: [Literal], viaNode: [ConfigPath]) {
        self.literals = literals
        self.viaNode = viaNode
    }

    /// Decodes what JSONSerialization made of `"ipv6Only"`; nil when it is
    /// not the shape xray_apple.rs writes.
    init?(json: Any) {
        guard let object = json as? [String: Any],
              let rawLiterals = object["literals"] as? [Any],
              let viaNode = Self.paths(object["viaNode"])
        else { return nil }
        var literals: [Literal] = []
        for raw in rawLiterals {
            guard let entry = raw as? [String: Any],
                  let ipv4 = entry["ipv4"] as? String,
                  let replace = Self.paths(entry["replace"]),
                  let append = Self.paths(entry["append"])
            else { return nil }
            literals.append(Literal(ipv4: ipv4, replace: replace, append: append))
        }
        self.init(literals: literals, viaNode: viaNode)
    }

    private static func paths(_ json: Any?) -> [ConfigPath]? {
        guard let list = json as? [Any] else { return nil }
        var paths: [ConfigPath] = []
        for raw in list {
            guard let steps = raw as? [Any], !steps.isEmpty else { return nil }
            var path: ConfigPath = []
            for step in steps {
                // JSONSerialization gives NSNumber for numbers and for
                // true/false alike; a boolean is no index.
                if let key = step as? String {
                    path.append(.key(key))
                } else if let number = step as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
                          let index = Int(exactly: number.doubleValue), index >= 0 {
                    path.append(.index(index))
                } else {
                    return nil
                }
            }
            paths.append(path)
        }
        return paths
    }
}

enum Ipv6OnlyNetwork {
    struct Outcome {
        let xray: [String: Any]
        /// Literals that got a synthesized address.
        let mapped: Int
        /// Direct routes now going through the node.
        let rerouted: Int
        /// Places that did not hold what the plan said: a core/extension
        /// mismatch, left untouched.
        let skipped: Int
    }

    /// The config for the network the process is on now, or nil when that
    /// network has IPv4 (or nothing answered) and the config stays as it is.
    ///
    /// Must run before the tunnel's settings are applied: once the utun has
    /// its IPv4 address, getaddrinfo sees IPv4 on the device and stops
    /// synthesizing, which would make an IPv6-only network look like any
    /// other.
    static func adapt(_ xray: [String: Any], plan: Ipv6OnlyPlan,
                      synthesize: (String) -> String?) -> Outcome? {
        let answers = plan.literals.compactMap { literal in
            synthesize(literal.ipv4).map { (literal, $0) }
        }
        // One synthesized answer means the network has no IPv4: NAT64
        // synthesis depends on the network, not on the address asked about.
        guard !answers.isEmpty else { return nil }

        var config: Any = xray
        var skipped = 0
        func update(_ path: ConfigPath, _ change: (Any) -> Any?) {
            if let updated = updating(config, at: path[...], change) {
                config = updated
            } else {
                skipped += 1
            }
        }

        for (literal, synthesized) in answers {
            for path in literal.replace {
                update(path) { old in (old as? String) == literal.ipv4 ? synthesized : nil }
            }
            for path in literal.append {
                update(path) { old in
                    guard var list = old as? [Any],
                          list.contains(where: { ($0 as? String) == literal.ipv4 })
                    else { return nil }
                    if !list.contains(where: { ($0 as? String) == synthesized }) {
                        list.append(synthesized)
                    }
                    return list
                }
            }
        }
        var rerouted = 0
        for path in plan.viaNode {
            update(path) { old in
                guard (old as? String) == "direct" else { return nil }
                rerouted += 1
                return "proxy"
            }
        }
        guard let adapted = config as? [String: Any] else { return nil }
        return Outcome(xray: adapted, mapped: answers.count, rerouted: rerouted, skipped: skipped)
    }

    /// `node` with the value at `path` replaced by `change(old)`; nil when the
    /// path leads nowhere or `change` refuses the old value.
    private static func updating(_ node: Any, at path: ArraySlice<ConfigPathStep>,
                                 _ change: (Any) -> Any?) -> Any? {
        guard let step = path.first else { return change(node) }
        switch step {
        case let .key(key):
            guard var object = node as? [String: Any], let child = object[key],
                  let updated = updating(child, at: path.dropFirst(), change)
            else { return nil }
            object[key] = updated
            return object
        case let .index(index):
            guard var array = node as? [Any], array.indices.contains(index),
                  let updated = updating(array[index], at: path.dropFirst(), change)
            else { return nil }
            array[index] = updated
            return array
        }
    }
}

enum Nat64 {
    /// The IPv6 address the network synthesizes for `ipv4`, or nil when the
    /// literal itself is the one to dial (the network has IPv4) or nothing
    /// answered.
    static func synthesize(_ ipv4: String) -> String? {
        var hints = addrinfo()
        hints.ai_family = PF_UNSPEC
        hints.ai_socktype = SOCK_STREAM
        // AI_ADDRCONFIG | AI_V4MAPPED_CFG: what Apple documents for NAT64
        // synthesis of an IPv4 literal.
        hints.ai_flags = AI_DEFAULT
        var list: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(ipv4, nil, &hints, &list) == 0, let first = list else { return nil }
        defer { freeaddrinfo(first) }

        var synthesized: String?
        var cursor: UnsafeMutablePointer<addrinfo>? = first
        while let entry = cursor {
            let info = entry.pointee
            if info.ai_family == AF_INET {
                return nil
            }
            if info.ai_family == AF_INET6, synthesized == nil, let address = info.ai_addr {
                synthesized = numericHost(address, length: info.ai_addrlen)
            }
            cursor = info.ai_next
        }
        // An IPv4-mapped address (::ffff:a.b.c.d) is the literal again in
        // other clothes, and a scoped one is not a NAT64 answer.
        guard let synthesized, !synthesized.lowercased().hasPrefix("::ffff:"),
              !synthesized.contains("%")
        else { return nil }
        return synthesized
    }

    private static func numericHost(_ address: UnsafeMutablePointer<sockaddr>, length: socklen_t) -> String? {
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        guard getnameinfo(address, length, &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 else {
            return nil
        }
        let bytes = host.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }
        return String(decoding: bytes, as: UTF8.self)
    }
}
