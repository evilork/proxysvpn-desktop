#!/usr/bin/env bash
# src-tauri/scripts/bench-configs.sh — v4 (simpler, faster, watchdogged)

export LC_ALL=C
export LANG=C

set -uo pipefail

INPUT="${1:-}"
[[ -z "$INPUT" ]] && { echo "usage: $0 <subscription-or-vless>"; exit 1; }

# ── Locate xray ───────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
XRAY=""
for c in \
    "$SCRIPT_DIR/../binaries/xray-aarch64-apple-darwin" \
    "$SCRIPT_DIR/../binaries/xray" \
    "/Applications/ProxysVPN.app/Contents/MacOS/xray"
do
    [[ -x "$c" ]] && { XRAY="$c"; break; }
done
[[ -n "$XRAY" ]] || { echo "ERROR: xray not found"; exit 1; }
ASSETS_DIR="$(dirname "$XRAY")"

# ── Get VLESS URL ─────────────────────────────────────────────────────
if [[ "$INPUT" =~ ^vless:// ]]; then
    VLESS="$INPUT"
else
    raw=$(curl -sS --max-time 15 "$INPUT" || true)
    [[ -n "$raw" ]] || { echo "ERROR: empty subscription"; exit 1; }
    decoded=$(echo "$raw" | base64 -d 2>/dev/null || echo "$raw")
    VLESS=$(echo "$decoded" | grep -m1 '^vless://' || true)
    [[ -n "$VLESS" ]] || { echo "ERROR: no vless:// in subscription"; exit 1; }
fi

# ── Parse ─────────────────────────────────────────────────────────────
url="${VLESS#vless://}"
url="${url%%#*}"
UUID="${url%%@*}"
url="${url#*@}"
hp="${url%%\?*}"
hp="${hp%%/*}"
HOST="${hp%:*}"
PORT="${hp##*:}"
[[ "$PORT" =~ ^[0-9]+$ ]] || { echo "ERROR: bad PORT '$PORT'"; exit 1; }

query=""
[[ "$url" == *\?* ]] && query="${url#*\?}"
get_param() {
    local v
    v=$(echo "$query" | tr '&' '\n' | grep "^$1=" | head -1 | cut -d= -f2-)
    [[ -z "$v" ]] && return
    printf '%b' "${v//%/\\x}"
}
ENCRYPTION="$(get_param encryption)"; [[ -n "$ENCRYPTION" ]] || ENCRYPTION="none"
PBK="$(get_param pbk)"
SID="$(get_param sid)"
SNI="$(get_param sni)"; [[ -n "$SNI" ]] || SNI="$HOST"
FP="$(get_param fp)";   [[ -n "$FP" ]]  || FP="chrome"
FLOW="$(get_param flow)"
SPX="$(get_param spx)"

echo "VLESS: $HOST:$PORT  flow=${FLOW:-(none)}  sni=$SNI"
echo ""

SOCKS_PORT=1090
TMPDIR="$(mktemp -d /tmp/proxysvpn-bench.XXXXXX)"

# Aggressive cleanup on exit — kills any lingering xray spawned by us
cleanup() {
    pkill -9 -f "xray.*proxysvpn-bench" 2>/dev/null || true
    rm -rf "$TMPDIR" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

if lsof -iTCP:${SOCKS_PORT} -sTCP:LISTEN -t 2>/dev/null | head -1 >/dev/null; then
    echo "ERROR: port $SOCKS_PORT in use, disconnect VPN app first"
    exit 1
fi

command -v jq >/dev/null || { echo "ERROR: install jq (brew install jq)"; exit 1; }

# Build a user JSON inline (no jq for performance reasons - jq adds 50ms each call)
USER_JSON=""
if [[ -n "$FLOW" ]]; then
    USER_JSON="{\"id\":\"$UUID\",\"encryption\":\"$ENCRYPTION\",\"flow\":\"$FLOW\"}"
else
    USER_JSON="{\"id\":\"$UUID\",\"encryption\":\"$ENCRYPTION\"}"
fi

REALITY_JSON="{\"serverName\":\"$SNI\",\"fingerprint\":\"$FP\",\"publicKey\":\"$PBK\",\"shortId\":\"$SID\""
[[ -n "$SPX" ]] && REALITY_JSON="$REALITY_JSON,\"spiderX\":\"$SPX\""
REALITY_JSON="$REALITY_JSON}"

OUTBOUND_NOSOCKOPT="{
  \"tag\":\"proxy\",\"protocol\":\"vless\",
  \"settings\":{\"vnext\":[{\"address\":\"$HOST\",\"port\":$PORT,\"users\":[$USER_JSON]}]},
  \"streamSettings\":{\"network\":\"tcp\",\"security\":\"reality\",\"realitySettings\":$REALITY_JSON}
}"

OUTBOUND_SOCKOPT="{
  \"tag\":\"proxy\",\"protocol\":\"vless\",
  \"settings\":{\"vnext\":[{\"address\":\"$HOST\",\"port\":$PORT,\"users\":[$USER_JSON]}]},
  \"streamSettings\":{\"network\":\"tcp\",\"security\":\"reality\",\"realitySettings\":$REALITY_JSON,
    \"sockopt\":{\"tcpKeepAliveIdle\":100,\"tcpKeepAliveInterval\":30,\"tcpNoDelay\":true,\"tcpcongestion\":\"bbr\"}}
}"

INBOUND_BASIC="{\"tag\":\"socks-in\",\"listen\":\"127.0.0.1\",\"port\":$SOCKS_PORT,\"protocol\":\"socks\",\"settings\":{\"udp\":true,\"auth\":\"noauth\"},\"sniffing\":{\"enabled\":true,\"destOverride\":[\"http\",\"tls\"]}}"
INBOUND_ROUTEONLY="{\"tag\":\"socks-in\",\"listen\":\"127.0.0.1\",\"port\":$SOCKS_PORT,\"protocol\":\"socks\",\"settings\":{\"udp\":true,\"auth\":\"noauth\"},\"sniffing\":{\"enabled\":true,\"destOverride\":[\"http\",\"tls\"],\"routeOnly\":true}}"
INBOUND_NOSNIFF="{\"tag\":\"socks-in\",\"listen\":\"127.0.0.1\",\"port\":$SOCKS_PORT,\"protocol\":\"socks\",\"settings\":{\"udp\":true,\"auth\":\"noauth\"}}"

DIRECT='{"tag":"direct","protocol":"freedom"}'
BLOCK='{"tag":"block","protocol":"blackhole"}'

ROUTING_BASIC='{"domainStrategy":"IPIfNonMatch","rules":[{"type":"field","outboundTag":"direct","ip":["geoip:private"]}]}'
ROUTING_SMART='{"domainStrategy":"IPIfNonMatch","rules":[{"type":"field","outboundTag":"block","network":"udp","port":"443"},{"type":"field","outboundTag":"direct","ip":["geoip:private"]},{"type":"field","outboundTag":"direct","ip":["geoip:ru"]},{"type":"field","outboundTag":"direct","domain":["domain:yandex.ru","domain:vk.com","domain:mail.ru","domain:gosuslugi.ru","domain:sber.ru","domain:tinkoff.ru","domain:ozon.ru","domain:wildberries.ru","domain:avito.ru"]}]}'

DNS_FRAG=',"dns":{"servers":["1.1.1.1","8.8.8.8"],"queryStrategy":"UseIP"}'

build_config() {
    local v="$1"
    local in out routing dns=""
    case "$v" in
        baseline)            in="$INBOUND_BASIC";    out="$OUTBOUND_NOSOCKOPT"; routing="$ROUTING_BASIC" ;;
        smart)               in="$INBOUND_BASIC";    out="$OUTBOUND_NOSOCKOPT"; routing="$ROUTING_SMART" ;;
        smart_dns)           in="$INBOUND_BASIC";    out="$OUTBOUND_NOSOCKOPT"; routing="$ROUTING_SMART"; dns="$DNS_FRAG" ;;
        smart_routeonly)     in="$INBOUND_ROUTEONLY";out="$OUTBOUND_NOSOCKOPT"; routing="$ROUTING_SMART" ;;
        smart_sockopt)       in="$INBOUND_BASIC";    out="$OUTBOUND_SOCKOPT";   routing="$ROUTING_SMART" ;;
        smart_dns_routeonly) in="$INBOUND_ROUTEONLY";out="$OUTBOUND_NOSOCKOPT"; routing="$ROUTING_SMART"; dns="$DNS_FRAG" ;;
        no_sniff)            in="$INBOUND_NOSNIFF";  out="$OUTBOUND_NOSOCKOPT"; routing="$ROUTING_BASIC" ;;
        all_p4)              in="$INBOUND_ROUTEONLY";out="$OUTBOUND_SOCKOPT";   routing="$ROUTING_SMART"; dns="$DNS_FRAG" ;;
    esac
    echo "{\"log\":{\"loglevel\":\"warning\"},\"inbounds\":[$in],\"outbounds\":[$out,$DIRECT,$BLOCK],\"routing\":$routing$dns}"
}

# ── Probe to pick a working URL ────────────────────────────────────────
echo "==> Starting probe xray..."
PROBE_CFG="$(build_config baseline)"
echo "$PROBE_CFG" > "$TMPDIR/proxysvpn-bench-probe.json"

XRAY_LOCATION_ASSET="$ASSETS_DIR" "$XRAY" -config "$TMPDIR/proxysvpn-bench-probe.json" \
    >"$TMPDIR/probe.log" 2>&1 &
PROBE_PID=$!

ready=false
for i in 1 2 3 4 5 6 7 8 9 10; do
    sleep 0.3
    if nc -z -G 1 127.0.0.1 "$SOCKS_PORT" 2>/dev/null; then
        ready=true; break
    fi
    kill -0 $PROBE_PID 2>/dev/null || break
done

if ! $ready; then
    echo "ERROR: xray failed to start. Log:"
    tail -10 "$TMPDIR/probe.log" 2>/dev/null | sed 's/^/    /'
    exit 1
fi
echo "    probe xray ready (pid $PROBE_PID)"

# Try several URLs through proxy with TIGHT timeout (5s)
CANDIDATES=(
    "https://speed.cloudflare.com/__down?bytes=10485760"   # 10 MB
    "http://cachefly.cachefly.net/10mb.test"
    "https://speedtest.tele2.net/10MB.zip"
    "https://proof.ovh.net/files/10Mb.dat"
)

echo "==> Picking working test URL via proxy (5s probe each)..."
DOWNLOAD_URL=""
for u in "${CANDIDATES[@]}"; do
    code=$(curl -o /dev/null -s -L --max-time 5 \
        --proxy "socks5h://127.0.0.1:$SOCKS_PORT" \
        -w '%{http_code}' "$u" --range 0-1023 2>&1 || echo "000")
    if [[ "$code" == "200" || "$code" == "206" ]]; then
        DOWNLOAD_URL="$u"
        echo "    using: $u"
        break
    else
        echo "    skip:  $u  (code=$code)"
    fi
done

kill -9 $PROBE_PID 2>/dev/null
wait $PROBE_PID 2>/dev/null
sleep 0.5

[[ -n "$DOWNLOAD_URL" ]] || { echo "ERROR: no test URL works through proxy"; exit 1; }
echo ""

# ── Bench function with WATCHDOG ──────────────────────────────────────
TEST_BYTES=10485760  # 10 MB
HARD_TIMEOUT=15      # seconds, then we kill curl

bench() {
    local name="$1"
    local desc="$2"
    local config="$3"

    printf "  %-22s  %-50s  " "$name" "$desc"

    local cfg_path="$TMPDIR/proxysvpn-bench-$name.json"
    echo "$config" > "$cfg_path"

    XRAY_LOCATION_ASSET="$ASSETS_DIR" "$XRAY" -config "$cfg_path" \
        >"$TMPDIR/xray-$name.log" 2>&1 &
    local xray_pid=$!

    local ready=false
    for i in 1 2 3 4 5 6 7 8; do
        sleep 0.3
        if nc -z -G 1 127.0.0.1 "$SOCKS_PORT" 2>/dev/null; then
            ready=true; break
        fi
        kill -0 $xray_pid 2>/dev/null || break
    done

    if ! $ready; then
        echo "FAILED to start"
        tail -2 "$TMPDIR/xray-$name.log" 2>/dev/null | sed 's/^/      /'
        kill -9 $xray_pid 2>/dev/null
        wait $xray_pid 2>/dev/null
        return
    fi

    # Single curl with hard timeout; tight max-time.
    local result
    result=$(curl -o /dev/null -s -L \
        --proxy "socks5h://127.0.0.1:$SOCKS_PORT" \
        --max-time "$HARD_TIMEOUT" \
        --connect-timeout 5 \
        -w '%{speed_download} %{time_total} %{http_code}' \
        "$DOWNLOAD_URL" 2>&1) || true

    kill -9 $xray_pid 2>/dev/null
    wait $xray_pid 2>/dev/null
    sleep 0.2

    local bps time_s code
    read -r bps time_s code <<< "$result"

    if [[ "$code" != "200" && "$code" != "206" ]]; then
        echo "http=$code"
        return
    fi

    local mbps
    mbps=$(awk -v b="$bps" 'BEGIN { printf "%.0f", b * 8 / 1000000 }')
    printf "%5s Mbps  (%.1fs)\n" "$mbps" "$time_s"
}

# ── Run ───────────────────────────────────────────────────────────────
echo "Method: 1 curl × 10 MB × max ${HARD_TIMEOUT}s (single-flow)"
echo ""
printf "  %-22s  %-50s  %s\n" "name" "description" "throughput"
printf "  %-22s  %-50s  %s\n" "----" "-----------" "----------"

bench baseline             "original config (pre-P4)"          "$(build_config baseline)"
bench smart                "+ smart routing"                   "$(build_config smart)"
bench smart_dns            "+ smart routing + DNS section"     "$(build_config smart_dns)"
bench smart_routeonly      "+ smart routing + routeOnly:true"  "$(build_config smart_routeonly)"
bench smart_sockopt        "+ smart routing + sockopt"         "$(build_config smart_sockopt)"
bench smart_dns_routeonly  "+ smart routing + DNS + routeOnly" "$(build_config smart_dns_routeonly)"
bench no_sniff             "minimal, no sniffing"              "$(build_config no_sniff)"
bench all_p4               "ALL P4 features"                   "$(build_config all_p4)"

echo ""
echo "Done. Highest = best config for the test endpoint via VPN."
