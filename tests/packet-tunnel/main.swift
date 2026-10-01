// tests/packet-tunnel/main.swift
//
// The Network Extension's pure logic, run on the Mac:
//   bash scripts/test-packet-tunnel.sh
// The simulator does not start a packet tunnel and there is no XCTest target,
// so this is a plain executable compiled together with the file under test
// (src-tauri/gen/apple/PacketTunnel/Ipv6OnlyNetwork.swift). It exits non-zero
// on the first failed group.
//
// The plan below is the literal form xray_apple.rs writes; its Rust test
// `the_envelope_carries_the_plan_in_the_form_the_extension_reads` pins the
// same shape from the other side. Addresses are documentation examples
// except Yandex's public resolver, which the app names on purpose.

import Foundation

var passed = 0
var failures: [String] = []

func check(_ condition: @autoclosure () -> Bool, _ what: String, line: Int = #line) {
    if condition() {
        passed += 1
    } else {
        failures.append("line \(line): \(what)")
    }
}

func json(_ text: String) -> Any {
    guard let data = text.data(using: .utf8),
          let value = try? JSONSerialization.jsonObject(with: data)
    else {
        fatalError("fixture is not JSON: \(text)")
    }
    return value
}

func value(_ root: Any, _ path: ConfigPath) -> Any? {
    path.reduce(Optional(root)) { node, step in
        switch step {
        case let .key(key): return (node as? [String: Any])?[key]
        case let .index(i): return (node as? [Any]).flatMap { $0.indices.contains(i) ? $0[i] : nil }
        }
    }
}

func string(_ root: Any, _ path: ConfigPath) -> String? {
    value(root, path) as? String
}

func strings(_ root: Any, _ path: ConfigPath) -> [String]? {
    (value(root, path) as? [Any])?.compactMap { $0 as? String }
}

let node = "192.0.2.10"
let yandex = "77.88.8.8"

let config = json("""
{
  "outbounds": [
    {"tag": "proxy", "protocol": "vless",
     "settings": {"vnext": [{"address": "\(node)", "port": 443}]}},
    {"tag": "direct", "protocol": "freedom"}
  ],
  "dns": {"servers": [
    {"address": "https://1.1.1.1/dns-query"},
    {"address": "\(yandex)", "port": 53, "domains": ["domain:ya.ru"]}
  ]},
  "routing": {"rules": [
    {"inboundTag": ["dns-in"], "ip": ["\(yandex)"], "outboundTag": "direct"},
    {"inboundTag": ["dns-in"], "outboundTag": "proxy"},
    {"ip": ["10.0.0.0/8", "192.168.0.0/16"], "outboundTag": "direct"},
    {"domain": ["domain:ya.ru"], "outboundTag": "direct"},
    {"ip": ["5.8.0.0/16"], "outboundTag": "direct"}
  ]}
}
""") as! [String: Any]

let planJSON = json("""
{
  "literals": [
    {"ipv4": "\(node)", "replace": [["outbounds", 0, "settings", "vnext", 0, "address"]], "append": []},
    {"ipv4": "\(yandex)", "replace": [["dns", "servers", 1, "address"]], "append": [["routing", "rules", 0, "ip"]]}
  ],
  "viaNode": [["routing", "rules", 3, "outboundTag"], ["routing", "rules", 4, "outboundTag"]]
}
""")

/// A NAT64 network with the well-known prefix, as RFC 6052 writes it.
func nat64(_ ipv4: String) -> String? {
    let octets = ipv4.split(separator: ".").compactMap { UInt16($0) }
    guard octets.count == 4, octets.allSatisfy({ $0 <= 255 }) else { return nil }
    let high = octets[0] << 8 | octets[1]
    let low = octets[2] << 8 | octets[3]
    return "64:ff9b::" + String(high, radix: 16) + ":" + String(low, radix: 16)
}

// ── Decoding ────────────────────────────────────────────────────────────────

do {
    let plan = Ipv6OnlyPlan(json: planJSON)
    check(plan != nil, "the plan xray_apple.rs writes decodes")
    check(plan?.literals.count == 2, "both literals")
    check(plan?.literals.first?.replace == [[.key("outbounds"), .index(0), .key("settings"),
                                             .key("vnext"), .index(0), .key("address")]],
          "keys and indices keep their kinds")
    check(plan?.viaNode.count == 2, "both direct routes")

    let malformed = [
        #"{"literals": []}"#,
        #"{"literals": [{"ipv4": 1, "replace": [], "append": []}], "viaNode": []}"#,
        #"{"literals": [], "viaNode": [[]]}"#,
        #"{"literals": [], "viaNode": [["routing", true]]}"#,
        #"{"literals": [], "viaNode": [["routing", -1]]}"#,
        #"{"literals": [], "viaNode": [["routing", 1.5]]}"#,
        #"["literals"]"#,
    ]
    for text in malformed {
        check(Ipv6OnlyPlan(json: json(text)) == nil, "refused: \(text)")
    }
    check(Ipv6OnlyPlan(json: json(#"{"literals": [], "viaNode": []}"#)) == .empty, "an empty plan is valid")
}

// ── A network with IPv4: nothing changes ─────────────────────────────────────

do {
    let plan = Ipv6OnlyPlan(json: planJSON)!
    check(Ipv6OnlyNetwork.adapt(config, plan: plan, synthesize: { _ in nil }) == nil,
          "no synthesized answer, no change")
    check(Ipv6OnlyNetwork.adapt(config, plan: .empty, synthesize: nat64) == nil,
          "an empty plan (a profile from an older build) changes nothing")
}

// ── An IPv6-only network with NAT64 ──────────────────────────────────────────

if let plan = Ipv6OnlyPlan(json: planJSON),
   let outcome = Ipv6OnlyNetwork.adapt(config, plan: plan, synthesize: nat64) {
    let adapted = outcome.xray
    let rule = { (i: Int) -> ConfigPath in [.key("routing"), .key("rules"), .index(i)] }

    check(string(adapted, plan.literals[0].replace[0]) == "64:ff9b::c000:20a", "the node is dialled through NAT64")
    check(string(adapted, [.key("dns"), .key("servers"), .index(1), .key("address")]) == "64:ff9b::4d58:808",
          "Yandex is asked through NAT64")
    check(strings(adapted, rule(0) + [.key("ip")]) == [yandex, "64:ff9b::4d58:808"],
          "the resolver's queries still leave directly, to either address")
    check(string(adapted, rule(3) + [.key("outboundTag")]) == "proxy", "direct names go through the node")
    check(string(adapted, rule(4) + [.key("outboundTag")]) == "proxy", "direct networks go through the node")
    check(string(adapted, rule(2) + [.key("outboundTag")]) == "direct", "the LAN stays direct")
    check(string(adapted, rule(0) + [.key("outboundTag")]) == "direct", "the resolver's rule stays direct")
    check(string(adapted, [.key("dns"), .key("servers"), .index(0), .key("address")]) == "https://1.1.1.1/dns-query",
          "the DoH server is left alone")
    check(outcome.mapped == 2 && outcome.rerouted == 2 && outcome.skipped == 0,
          "counts: \(outcome.mapped) mapped, \(outcome.rerouted) rerouted, \(outcome.skipped) skipped")
    check(JSONSerialization.isValidJSONObject(adapted), "the result is still a config Xray can be given")
    check(string(config, plan.literals[0].replace[0]) == node, "the input config is not modified")
} else {
    failures.append("NAT64: the plan was not applied")
}

// ── A plan that does not match the config ────────────────────────────────────

do {
    let stale = Ipv6OnlyPlan(literals: [
        .init(ipv4: node,
              replace: [[.key("outbounds"), .index(1), .key("tag")], [.key("outbounds"), .index(9), .key("tag")]],
              append: [[.key("routing"), .key("rules"), .index(3), .key("domain")]]),
    ], viaNode: [[.key("routing"), .key("rules"), .index(1), .key("outboundTag")]])
    let outcome = Ipv6OnlyNetwork.adapt(config, plan: stale, synthesize: nat64)
    check(outcome?.skipped == 4, "every place that holds something else is skipped: \(String(describing: outcome?.skipped))")
    check(outcome?.rerouted == 0, "a route that is not direct is not touched")
    check(string(outcome?.xray ?? [:], [.key("outbounds"), .index(1), .key("tag")]) == "direct",
          "a value other than the literal is never replaced")
    check(strings(outcome?.xray ?? [:], [.key("routing"), .key("rules"), .index(3), .key("domain")]) == ["domain:ya.ru"],
          "a list without the literal gets nothing added")
}

do {
    // Already carrying the synthesized address: nothing is added twice.
    var twice = config
    var routing = twice["routing"] as! [String: Any]
    var rules = routing["rules"] as! [Any]
    var first = rules[0] as! [String: Any]
    first["ip"] = [yandex, "64:ff9b::4d58:808"]
    rules[0] = first
    routing["rules"] = rules
    twice["routing"] = routing
    let plan = Ipv6OnlyPlan(json: planJSON)!
    let adapted = Ipv6OnlyNetwork.adapt(twice, plan: plan, synthesize: nat64)?.xray ?? [:]
    check(strings(adapted, [.key("routing"), .key("rules"), .index(0), .key("ip")]) == [yandex, "64:ff9b::4d58:808"],
          "no duplicate address")
}

// ── This Mac ─────────────────────────────────────────────────────────────────

// A development Mac has IPv4 (a Mac sharing a NAT64 network keeps its own),
// so getaddrinfo answers with the literal and there is nothing to map.
check(Nat64.synthesize(yandex) == nil, "on a network with IPv4 the literal is dialled as it is")

if failures.isEmpty {
    print("packet-tunnel: \(passed) checks passed")
} else {
    for failure in failures {
        print("FAIL \(failure)")
    }
    print("packet-tunnel: \(failures.count) failed, \(passed) passed")
    exit(1)
}
