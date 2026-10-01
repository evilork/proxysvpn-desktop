// src-tauri/src/xray_manager.rs
//
// xray is the FRONT of the chain for every protocol now, not just for VLESS.
//
// ── What changed in the chain and why ──────────────────────────────────────
// Before:
//   VLESS       tun2socks -> xray:10808 -> node
//   Hysteria2   tun2socks -> hysteria:10809 -> node        (xray not in it)
// The second line is the bug: the split-routing rules live in the xray config,
// so on a Hysteria2 location there were no rules at all and every Russian
// bank, Gosuslugi and RuTube request left the country.
//
// After:
//   VLESS       tun2socks -> xray:10808 --vless--> node
//   Hysteria2   tun2socks -> xray:10808 --socks--> hysteria:10809 -> node
// One config, one rule set, one SOCKS port for tun2socks to talk to. Changing
// location is now "rewrite this config and restart xray" — tun2socks, the utun
// device, its address and both halves of the default route are never touched,
// which is what makes healing feel like a pause instead of "the internet went
// away".
//
// ── The loop this file also fixes ──────────────────────────────────────────
// Split routing on macOS was not merely absent on Hysteria2; on VLESS it was a
// connection storm. The `direct` outbound opened an ordinary socket, and an
// ordinary socket is routed by `0.0.0.0/1 -> utun225`, i.e. straight back into
// tun2socks, which handed it to xray, which sent it direct again. The owner's
// own log has 2 795 connections to one address inside a single second and
// 278 377 "-> direct" lines against 3 167 real "\>> proxy" ones: ninety-nine
// per cent of the "direct" traffic was the loop eating itself.
//
// The cure is `sockopt.interface`, which xray applies on Darwin as IP_BOUND_IF:
// the socket leaves through the physical interface whatever the routing table
// says. Verified against the bundled xray 26.3.27 by binding a freedom
// outbound to `lo0` and watching the request fail while an unbound control
// succeeded. Both `proxy` and `direct` are bound: the host route to the node
// is belt, this is braces, and a stale host route after a network change turns
// from a silent storm into a fast failure the supervisor can see and repair.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use pvpn_platform::process as pprocess;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::errors::{AppError, ErrorCode};
use crate::hysteria_manager::HY2_SOCKS_PORT;
use crate::pidfile::Engine;
use crate::subscription::{build_xray_config, ServerConfig, VlessConfig};

/// The one port tun2socks ever talks to. Mirrors `tun::SOCKS_PORT`, and
/// `build_runtime_config` refuses a config that listens anywhere else rather
/// than letting the two drift apart in silence.
pub const FRONT_SOCKS_PORT: u16 = 10808;

/// How long xray gets to fail on its own before we call the start a success.
///
/// A bad config, a busy port or a missing asset file all make xray exit within
/// a few hundred milliseconds; waiting this long turns "started" from a hope
/// into an observation.
const START_SETTLE_MS: u64 = 500;

/// How long a started xray gets to open its SOCKS port, and how often we look.
///
/// "Did not exit" is not "ready". On a Windows 11 VM (02.10.2026, x64 xray
/// emulated on ARM) xray needed about two seconds to listen, and the probe
/// that follows a location change ran into a closed port, failed, and started
/// the repair ladder, whose every rung restarted xray and probed too early
/// again until the connection was declared broken.
const LISTEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
const LISTEN_POLL: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Default)]
pub struct XrayState {
    child: Option<Engine>,
}

pub type SharedXrayState = Arc<Mutex<XrayState>>;

pub fn new_state() -> SharedXrayState {
    Arc::new(Mutex::new(XrayState::default()))
}

/// Locates the xray binary and the directory holding geoip.dat/geosite.dat.
///
/// Both come from the same ordered directory list as every other sidecar
/// (`crate::sidecars::dirs`), so a bundle, a dev checkout and a Windows or
/// Linux install all resolve with one rule instead of three hand-written lists.
pub fn xray_paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), AppError> {
    let dirs = crate::sidecars::dirs(app);
    let bin = crate::sidecars::find(app, "xray", "xray")?;

    let assets = dirs
        .iter()
        .find(|d| d.join("geoip.dat").is_file())
        .map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.clone()))
        // The binary's own directory is the last resort: xray starts without
        // the geo files and only fails on the first rule that needs them, so a
        // wrong guess here is recoverable while refusing to start is not.
        .unwrap_or_else(|| bin.parent().map(Path::to_path_buf).unwrap_or_default());

    Ok((bin, assets))
}

// ───────────────────────────────────────────────────────────────────────────
// The config: one shape for both protocols
// ───────────────────────────────────────────────────────────────────────────

/// A VLESS entry that exists only to be thrown away.
///
/// `build_xray_config` is the single place that turns the service's routing
/// profile into xray rules, and it takes a VLESS node. Rather than copy those
/// rules into a second builder — the "two lists" mistake this project has
/// already paid for twice — we ask it for the same config and then replace the
/// one part that differs. `swap_proxy_for_socks` refuses to return a config
/// that still contains a VLESS outbound, so the placeholder cannot leak.
fn placeholder_vless() -> VlessConfig {
    VlessConfig {
        uuid: String::new(),
        host: "0.0.0.0".to_string(),
        port: 0,
        encryption: "none".to_string(),
        public_key: String::new(),
        short_id: String::new(),
        sni: String::new(),
        fingerprint: String::new(),
        flow: String::new(),
        spider_x: String::new(),
        remark: String::new(),
        transport: crate::subscription::VlessTransport::Tcp,
    }
}

/// Build the config xray will actually run for this server.
///
/// `physical_iface` is the interface the machine really reaches the internet
/// through ("en0"). Every socket xray opens is bound to it, which is what
/// keeps the `direct` outbound from being swallowed by our own tunnel.
pub fn build_runtime_config(
    server: &ServerConfig,
    physical_iface: &str,
) -> Result<Value, AppError> {
    let mut config = unbound_config(server)?;
    bind_outbounds_to_interface(&mut config, physical_iface)?;
    check_front_port(&config)?;
    Ok(config)
}

/// Local SOCKS port of the race partner (Watafast): while the tunnel rides the
/// chosen node, the warm-up dials a node on ANOTHER transport through here,
/// and whichever answers first carries the session.
pub const RACE_SOCKS_PORT: u16 = 10809;
/// Tag of the partner's outbound.
pub const RACE_TAG: &str = "race-b";

/// Fresh random username/password for one race's SOCKS inbound.
///
/// The inbound forwards straight to another country for as long as the race
/// lasts, on a fixed loopback port any local process can reach — `noauth`
/// there would let anything on the machine ride it. Drawn from the system
/// CSPRNG through `entropy.rs`, the same source `netmem::salt` uses, rather
/// than pulling in a `rand` dependency for two short-lived tokens.
pub fn race_credentials() -> Result<(String, String), AppError> {
    Ok((random_hex_token(8)?, random_hex_token(16)?))
}

fn random_hex_token(bytes: usize) -> Result<String, AppError> {
    let mut buf = vec![0u8; bytes];
    crate::entropy::fill(&mut buf)
        .map_err(|_| config_bug("could not read randomness for the race inbound's password"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// The runtime config for `server` with `partner` added as a second outbound,
/// reachable only through `RACE_SOCKS_PORT`. The tunnel's traffic still goes
/// to `server`: the partner carries nothing but the warm-up until it wins.
///
/// `creds` (user, pass) are required, not optional: a race inbound with no
/// password is exactly the defect this signature exists to make impossible to
/// build by accident.
pub fn build_race_config(
    server: &ServerConfig,
    partner: &VlessConfig,
    physical_iface: &str,
    creds: (&str, &str),
) -> Result<Value, AppError> {
    if !matches!(server, ServerConfig::Vless(_)) {
        return Err(config_bug("a race needs a VLESS node to ride the tunnel"));
    }
    let mut config = unbound_config(server)?;
    add_race_partner(&mut config, partner, creds)?;
    bind_outbounds_to_interface(&mut config, physical_iface)?;
    check_front_port(&config)?;
    Ok(config)
}

fn add_race_partner(config: &mut Value, partner: &VlessConfig, creds: (&str, &str)) -> Result<(), AppError> {
    let theirs = build_xray_config(partner);
    let mut outbound = theirs
        .get("outbounds")
        .and_then(Value::as_array)
        .and_then(|list| list.iter().find(|o| o.get("tag").and_then(Value::as_str) == Some("proxy")))
        .cloned()
        .ok_or_else(|| config_bug("the partner config has no proxy outbound"))?;
    outbound["tag"] = json!(RACE_TAG);

    let (user, pass) = creds;
    config
        .get_mut("outbounds")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| config_bug("config has no outbounds array"))?
        .push(outbound);
    config
        .get_mut("inbounds")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| config_bug("config has no inbounds array"))?
        .push(json!({
            "tag": "race-in",
            "listen": "127.0.0.1",
            "port": RACE_SOCKS_PORT,
            "protocol": "socks",
            // A fixed loopback port with no password is reachable by any
            // other process on the machine, and this one forwards straight
            // to another country for as long as the race lasts.
            "settings": {
                "udp": false,
                "auth": "password",
                "accounts": [{ "user": user, "pass": pass }]
            }
        }));
    // First rule: nothing else may catch the race inbound's traffic.
    config
        .pointer_mut("/routing/rules")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| config_bug("config has no routing rules"))?
        .insert(
            0,
            json!({ "type": "field", "inboundTag": ["race-in"], "outboundTag": RACE_TAG }),
        );
    Ok(())
}

/// The config before it is bound to the physical interface.
fn unbound_config(server: &ServerConfig) -> Result<Value, AppError> {
    let config = match server {
        ServerConfig::Vless(cfg) => build_xray_config(cfg),
        ServerConfig::Hy2(_) => {
            let mut base = build_xray_config(&placeholder_vless());
            swap_proxy_for_socks(&mut base, HY2_SOCKS_PORT)?;
            base
        }
    };
    Ok(config)
}

/// Point the `proxy` outbound at a local SOCKS server instead of a node.
///
/// This is the whole of the "hysteria becomes an outbound of xray" change. It
/// fails loudly when the expected outbound is not there: a config that
/// silently kept a placeholder VLESS node would start, listen, and send every
/// byte to 0.0.0.0.
fn swap_proxy_for_socks(config: &mut Value, port: u16) -> Result<(), AppError> {
    let outbounds = config
        .get_mut("outbounds")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| config_bug("config has no outbounds array"))?;

    let slot = outbounds
        .iter()
        .position(|o| o.get("tag").and_then(Value::as_str) == Some("proxy"))
        .ok_or_else(|| config_bug("config has no outbound tagged proxy"))?;

    outbounds[slot] = json!({
        "tag": "proxy",
        "protocol": "socks",
        "settings": {
            "servers": [{ "address": "127.0.0.1", "port": port }]
        }
    });

    // The placeholder must be gone, not merely overwritten somewhere else.
    if outbounds
        .iter()
        .any(|o| o.get("protocol").and_then(Value::as_str) == Some("vless"))
    {
        return Err(config_bug("a vless outbound survived the swap"));
    }
    Ok(())
}

/// True when every server this outbound dials is on the loopback interface.
///
/// That is the Hysteria2 hop: `proxy` becomes socks to 127.0.0.1:10809. Such a
/// socket must NOT be pinned to the physical interface. On Darwin IP_BOUND_IF
/// makes the connect fail with "can't assign requested address", and on Linux
/// SO_BINDTODEVICE skips the `lo`-only local route, so the SYN leaves through
/// eth0 and dies. Either way xray could not reach hysteria and every Hysteria2
/// location was dead, DNS included. Nothing about the tunnel needs the
/// binding here: loopback never touches the routing table that `0.0.0.0/1`
/// lives in, and hysteria's own UDP to the node leaves through the /32 host
/// route.
fn dials_only_loopback(outbound: &Value) -> bool {
    let Some(servers) = outbound
        .pointer("/settings/servers")
        .and_then(Value::as_array)
    else {
        return false;
    };
    !servers.is_empty()
        && servers.iter().all(|server| {
            server
                .get("address")
                .and_then(Value::as_str)
                .is_some_and(is_loopback_host)
        })
}

fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Bind every outbound that opens a socket to the physical interface.
///
/// `blackhole` never dials, so it is left alone; binding it would only add a
/// line that has to be explained later. An outbound that only dials loopback
/// is left alone too — see `dials_only_loopback` for why binding it breaks it.
fn bind_outbounds_to_interface(config: &mut Value, iface: &str) -> Result<(), AppError> {
    if iface.trim().is_empty() {
        return Err(config_bug("physical interface name is empty"));
    }

    let outbounds = config
        .get_mut("outbounds")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| config_bug("config has no outbounds array"))?;

    for outbound in outbounds.iter_mut() {
        let dials = !matches!(
            outbound.get("protocol").and_then(Value::as_str),
            Some("blackhole")
        );
        if !dials || dials_only_loopback(outbound) {
            continue;
        }
        let stream = outbound
            .as_object_mut()
            .ok_or_else(|| config_bug("outbound is not an object"))?
            .entry("streamSettings")
            .or_insert_with(|| json!({}));
        let stream = stream
            .as_object_mut()
            .ok_or_else(|| config_bug("streamSettings is not an object"))?;
        // Merge rather than replace: REALITY settings live in the same object.
        let sockopt = stream.entry("sockopt").or_insert_with(|| json!({}));
        let sockopt = sockopt
            .as_object_mut()
            .ok_or_else(|| config_bug("sockopt is not an object"))?;
        sockopt.insert("interface".to_string(), json!(iface));
    }
    Ok(())
}

/// tun2socks is told to talk to `FRONT_SOCKS_PORT` and nothing else, so a
/// config that listens elsewhere would produce a tunnel that swallows every
/// packet in silence. Cheaper to notice here than in a support ticket.
fn check_front_port(config: &Value) -> Result<(), AppError> {
    let listening = config
        .get("inbounds")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|inbound| inbound.get("port"))
        .and_then(Value::as_u64);

    match listening {
        Some(port) if port == u64::from(FRONT_SOCKS_PORT) => Ok(()),
        _ => Err(config_bug("front inbound does not listen on the SOCKS port")),
    }
}

/// A config we built ourselves and got wrong is not a failure the user can
/// act on, so it travels as ENGINE_START_FAILED with the reason in the log.
fn config_bug(reason: &str) -> AppError {
    crate::logger::log("error", "xray", &format!("config: {reason}"));
    AppError::new(ErrorCode::EngineStartFailed)
}

// ───────────────────────────────────────────────────────────────────────────
// The process
// ───────────────────────────────────────────────────────────────────────────

pub async fn start(
    state: &SharedXrayState,
    bin: &Path,
    assets_dir: &Path,
    config: Value,
) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    if guard.child.is_some() {
        // Not an error the user should ever see: the caller restarts instead.
        return Err(config_bug("start called while xray is already running"));
    }

    let config_str = serde_json::to_string(&config)
        .map_err(|_| config_bug("config could not be serialised"))?;

    let mut cmd = Command::new(bin);
    cmd.arg("run")
        .arg("-config")
        .arg("stdin:")
        .env("XRAY_LOCATION_ASSET", assets_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Without this a console window pops up on Windows for every engine start.
    pprocess::no_window(&mut cmd);

    let mut child = Engine::spawn("xray", &mut cmd, bin).map_err(|e| {
        crate::logger::log("error", "xray", &format!("spawn failed: {e}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    {
        let Some(mut stdin) = child.child().stdin.take() else {
            child.stop().await;
            return Err(config_bug("xray stdin was not captured"));
        };
        if let Err(e) = stdin.write_all(config_str.as_bytes()).await {
            crate::logger::log("error", "xray", &format!("config write failed: {e}"));
            child.stop().await;
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
        if let Err(e) = stdin.shutdown().await {
            crate::logger::log("error", "xray", &format!("config close failed: {e}"));
            child.stop().await;
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
    }

    // "info" is a DEFAULT here, not a verdict: logger::log lets a line that
    // names its own level keep it. Passing "warn" for stderr — what this file
    // used to do — is what painted the log window orange from end to end.
    pump(child.child().stdout.take(), "info");
    pump(child.child().stderr.take(), "info");

    tokio::time::sleep(std::time::Duration::from_millis(START_SETTLE_MS)).await;
    match child.try_wait() {
        Ok(Some(status)) => {
            crate::logger::log("error", "xray", &format!("exited at once: {status}"));
            // A port already held by a leftover process is the one start
            // failure with a different cure ("try again" does work, once the
            // old process is gone), so it gets its own code.
            let code = if port_in_use(FRONT_SOCKS_PORT).await {
                ErrorCode::PortBusy
            } else {
                ErrorCode::EngineStartFailed
            };
            return Err(AppError::new(code));
        }
        Ok(None) => {}
        Err(e) => {
            crate::logger::log("error", "xray", &format!("try_wait failed: {e}"));
            child.stop().await;
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
    }

    let deadline = tokio::time::Instant::now() + LISTEN_TIMEOUT;
    while !port_in_use(FRONT_SOCKS_PORT).await {
        if let Ok(Some(status)) = child.try_wait() {
            crate::logger::log("error", "xray", &format!("exited before listening: {status}"));
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
        if tokio::time::Instant::now() >= deadline {
            // Not fatal: the probe that follows decides whether traffic flows.
            crate::logger::log(
                "warn",
                "xray",
                &format!("SOCKS port not open after {}s; going on", LISTEN_TIMEOUT.as_secs()),
            );
            break;
        }
        tokio::time::sleep(LISTEN_POLL).await;
    }

    guard.child = Some(child);
    Ok(())
}

fn pump<R>(stream: Option<R>, default_level: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let Some(stream) = stream else { return };
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            crate::logger::log(default_level, "xray", &line);
        }
    });
}

pub async fn stop(state: &SharedXrayState) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    if let Some(child) = guard.child.take() {
        child.stop().await;
    }
    // An xray we lost the handle to keeps holding 10808, and the next start
    // then fails with "port busy" for no reason the user can see. Only one WE
    // started is swept: the `pkill -x xray` that stood here until 28.09.2026
    // also took down any other VPN client's xray on the Mac.
    crate::pidfile::sweep("xray").await;
    Ok(())
}

/// Replace the running config without touching anything below us.
///
/// This is the whole "soft location change": tun2socks keeps its device, its
/// address and both halves of the default route, and only the engine behind
/// the SOCKS port changes. Roughly a second, and TCP sessions to the old exit
/// die — which we promise out loud rather than claiming it is seamless.
pub async fn restart(
    state: &SharedXrayState,
    bin: &Path,
    assets_dir: &Path,
    config: Value,
) -> Result<(), AppError> {
    stop(state).await?;
    start(state, bin, assets_dir, config).await
}

/// Is our own xray process alive?
///
/// Asked of the child handle rather than of `pgrep`: another copy of xray on
/// the machine (a second client, a leftover) is not our engine, and counting
/// it would keep the screen green over a dead tunnel.
pub async fn is_running(state: &SharedXrayState) -> bool {
    let mut guard = state.lock().await;
    match guard.child.as_mut() {
        Some(c) => match c.try_wait() {
            Ok(None) => true,
            _ => {
                guard.child = None;
                false
            }
        },
        None => false,
    }
}

/// Is anything listening on this local port?
///
/// A connect to loopback either completes or is refused immediately, so this
/// is a microsecond answer with no timeout to tune.
pub async fn port_in_use(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {

    /// Live check, off by default: the race config for two real links,
    /// written out for the bundled xray. Links come from the environment.
    #[test]
    #[ignore]
    fn live_race_config() {
        let a = crate::subscription::parse_vless_url(&std::env::var("WATAFAST_LIVE_LINK").unwrap()).unwrap();
        let b = crate::subscription::parse_vless_url(&std::env::var("WATAFAST_LIVE_LINK2").unwrap()).unwrap();
        let cfg = build_race_config(&ServerConfig::Vless(a), &b, "en0", ("racer", "s3cr3t")).unwrap();
        std::fs::write(std::env::var("WATAFAST_LIVE_OUT").unwrap(), serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
    }

    #[test]
    fn a_race_adds_the_partner_behind_its_own_port_and_leaves_the_tunnel_alone() {
        let main = ServerConfig::Vless(placeholder_vless());
        let mut partner = placeholder_vless();
        partner.host = "203.0.113.9".into();
        partner.transport = crate::subscription::VlessTransport::Xhttp {
            path: "/p".into(),
            mode: crate::subscription::XhttpMode::StreamOne,
            host: None,
        };
        partner.flow = String::new();
        let config = build_race_config(&main, &partner, "en0", ("racer", "s3cr3t")).expect("race config");

        // The tunnel's port is still first and still goes to the chosen node.
        assert_eq!(config["inbounds"][0]["port"], u64::from(FRONT_SOCKS_PORT));
        let race_in = config["inbounds"].as_array().unwrap().iter().find(|i| i["tag"] == "race-in").expect("race inbound");
        assert_eq!(race_in["port"], u64::from(RACE_SOCKS_PORT));
        assert_eq!(race_in["listen"], "127.0.0.1", "never on a public address");
        // Defence in depth: a fixed loopback port with no password is
        // reachable by any other process on the machine, and this one
        // forwards straight to another country for as long as the race lasts.
        assert_eq!(race_in["settings"]["auth"], "password", "must never be noauth");
        assert_eq!(race_in["settings"]["accounts"][0]["user"], "racer");
        assert_eq!(race_in["settings"]["accounts"][0]["pass"], "s3cr3t");
        assert_eq!(config["routing"]["rules"][0]["inboundTag"][0], "race-in");
        assert_eq!(config["routing"]["rules"][0]["outboundTag"], RACE_TAG);

        let race = outbound(&config, RACE_TAG);
        assert_eq!(race["streamSettings"]["network"], "xhttp");
        assert_eq!(race["settings"]["vnext"][0]["address"], "203.0.113.9");
        assert_eq!(race["streamSettings"]["sockopt"]["interface"], "en0", "the partner leaves by the real network too");
        assert_eq!(outbound(&config, "proxy")["settings"]["vnext"][0]["address"], main.host());
    }

    #[test]
    fn hysteria_does_not_race() {
        let hy2 = ServerConfig::Hy2(crate::subscription::Hy2Config {
            password: "p".into(),
            host: "203.0.113.7".into(),
            port: 443,
            sni: "www.bing.com".into(),
            pin_sha256: String::new(),
            insecure: true,
            remark: "NL".into(),
        });
        assert!(build_race_config(&hy2, &placeholder_vless(), "en0", ("u", "p")).is_err());
    }

    /// The password comes from the system CSPRNG: different every call, and
    /// hex-only so it always survives a JSON string and a SOCKS5 sub-negotiation.
    #[test]
    fn race_credentials_are_random_hex_and_differ_every_time() {
        let (user_a, pass_a) = race_credentials().expect("randomness available in CI too");
        let (user_b, pass_b) = race_credentials().expect("second draw");

        assert_eq!(user_a.len(), 16, "8 bytes of hex");
        assert_eq!(pass_a.len(), 32, "16 bytes of hex");
        assert!(user_a.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(pass_a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(user_a, user_b, "two draws must not collide in a test run");
        assert_ne!(pass_a, pass_b);
    }
    use super::*;
    use crate::subscription::Hy2Config;

    fn vless() -> ServerConfig {
        ServerConfig::Vless(VlessConfig {
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            host: "node.example".into(),
            port: 443,
            encryption: "none".into(),
            public_key: "pk".into(),
            short_id: "ab".into(),
            sni: "sni.example".into(),
            fingerprint: "chrome".into(),
            flow: "xtls-rprx-vision".into(),
            spider_x: String::new(),
            remark: "Германия".into(),
            transport: crate::subscription::VlessTransport::Tcp,
        })
    }

    fn hy2() -> ServerConfig {
        ServerConfig::Hy2(Hy2Config {
            password: "secret".into(),
            host: "node.example".into(),
            port: 443,
            sni: "sni.example".into(),
            pin_sha256: String::new(),
            insecure: false,
            remark: "Нидерланды".into(),
        })
    }

    fn outbound<'a>(config: &'a Value, tag: &str) -> &'a Value {
        config
            .get("outbounds")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|o| o.get("tag").and_then(Value::as_str) == Some(tag))
            })
            .expect("outbound present")
    }

    #[test]
    fn hy2_front_is_xray_talking_socks_to_hysteria() {
        let config = build_runtime_config(&hy2(), "en0").expect("config");
        let proxy = outbound(&config, "proxy");
        assert_eq!(proxy["protocol"], "socks");
        assert_eq!(proxy["settings"]["servers"][0]["address"], "127.0.0.1");
        assert_eq!(
            proxy["settings"]["servers"][0]["port"],
            u64::from(HY2_SOCKS_PORT)
        );
    }

    /// A loopback dial pinned to the physical NIC cannot connect: Darwin's
    /// IP_BOUND_IF answers "can't assign requested address" and Linux's
    /// SO_BINDTODEVICE skips the `lo`-only local route. The Hysteria2 hop is
    /// such a dial, so binding it killed every Hysteria2 location.
    #[test]
    fn the_hysteria_hop_is_not_bound_to_the_physical_interface() {
        let config = build_runtime_config(&hy2(), "en0").expect("config");
        let proxy = outbound(&config, "proxy");
        assert!(
            proxy["streamSettings"]["sockopt"]["interface"].is_null(),
            "a loopback outbound must not carry sockopt.interface: {proxy}"
        );
        // Everything that does leave the machine stays pinned.
        assert_eq!(outbound(&config, "direct")["streamSettings"]["sockopt"]["interface"], "en0");
    }

    #[test]
    fn only_outbounds_that_dial_loopback_count_as_local() {
        let local = json!({ "protocol": "socks", "settings": { "servers": [{ "address": "127.0.0.1", "port": 1 }] } });
        let local6 = json!({ "protocol": "socks", "settings": { "servers": [{ "address": "::1", "port": 1 }] } });
        let named = json!({ "protocol": "http", "settings": { "servers": [{ "address": "localhost", "port": 1 }] } });
        let remote = json!({ "protocol": "socks", "settings": { "servers": [{ "address": "203.0.113.5", "port": 1 }] } });
        let mixed = json!({ "protocol": "socks", "settings": { "servers": [
            { "address": "127.0.0.1", "port": 1 }, { "address": "203.0.113.5", "port": 1 }
        ] } });
        let freedom = json!({ "protocol": "freedom" });
        assert!(dials_only_loopback(&local));
        assert!(dials_only_loopback(&local6));
        assert!(dials_only_loopback(&named));
        assert!(!dials_only_loopback(&remote));
        assert!(!dials_only_loopback(&mixed), "one remote server is enough to need the binding");
        assert!(!dials_only_loopback(&freedom));
        assert!(!dials_only_loopback(&vless_outbound_of(&vless())));
    }

    fn vless_outbound_of(server: &ServerConfig) -> Value {
        let config = build_runtime_config(server, "en0").expect("config");
        outbound(&config, "proxy").clone()
    }

    #[test]
    fn the_placeholder_node_never_survives() {
        let config = build_runtime_config(&hy2(), "en0").expect("config");
        let outbounds = config["outbounds"].as_array().expect("outbounds");
        assert!(outbounds
            .iter()
            .all(|o| o["protocol"].as_str() != Some("vless")));
        // Structural, not textual: the routing rules are whatever the service
        // last sent, and grepping the whole config for an address would make
        // this test depend on them.
        assert!(outbounds
            .iter()
            .all(|o| o["settings"]["vnext"].is_null()));
    }

    #[test]
    fn both_protocols_get_the_same_routing_rules() {
        // The entire point of the unified layer: a Hysteria2 location is no
        // longer a location without split routing.
        let a = build_runtime_config(&vless(), "en0").expect("config");
        let b = build_runtime_config(&hy2(), "en0").expect("config");
        assert_eq!(a["routing"], b["routing"]);
        assert!(!a["routing"]["rules"]
            .as_array()
            .expect("rules")
            .is_empty());
    }

    #[test]
    fn dialing_outbounds_are_pinned_to_the_physical_interface() {
        // Without this the `direct` outbound is routed by 0.0.0.0/1 back into
        // our own tun device — the connection storm in the owner's log.
        let config = build_runtime_config(&vless(), "en3").expect("config");
        assert_eq!(outbound(&config, "direct")["streamSettings"]["sockopt"]["interface"], "en3");
        assert_eq!(outbound(&config, "proxy")["streamSettings"]["sockopt"]["interface"], "en3");
    }

    #[test]
    fn binding_keeps_reality_settings_intact() {
        let config = build_runtime_config(&vless(), "en0").expect("config");
        let proxy = outbound(&config, "proxy");
        assert_eq!(proxy["streamSettings"]["security"], "reality");
        assert_eq!(
            proxy["streamSettings"]["realitySettings"]["serverName"],
            "sni.example"
        );
    }

    #[test]
    fn the_blackhole_is_not_bound_to_anything() {
        let config = build_runtime_config(&vless(), "en0").expect("config");
        assert!(outbound(&config, "block").get("streamSettings").is_none());
    }

    #[test]
    fn an_empty_interface_name_is_refused() {
        // Binding to "" would silently mean "do not bind" and bring the loop
        // back, so it has to be a failure rather than a default.
        assert!(build_runtime_config(&vless(), "  ").is_err());
    }

    #[test]
    fn the_front_always_listens_where_tun2socks_looks() {
        let config = build_runtime_config(&hy2(), "en0").expect("config");
        assert_eq!(config["inbounds"][0]["port"], u64::from(FRONT_SOCKS_PORT));
        assert_eq!(FRONT_SOCKS_PORT, crate::tun::SOCKS_PORT);
    }

    #[test]
    fn a_config_without_a_proxy_outbound_is_refused() {
        let mut config = json!({ "outbounds": [{ "tag": "direct", "protocol": "freedom" }] });
        assert!(swap_proxy_for_socks(&mut config, 10809).is_err());
    }

    #[test]
    fn a_front_on_the_wrong_port_is_refused() {
        let config = json!({ "inbounds": [{ "port": 1080 }] });
        assert!(check_front_port(&config).is_err());
    }
}

