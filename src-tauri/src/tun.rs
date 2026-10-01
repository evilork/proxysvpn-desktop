// src-tauri/src/tun.rs
//
// The utun device, the routing table and tun2socks. Assumes the process is
// already root (scripts/launcher.sh arranges that).
//
// ── What this layer is now responsible for, and what it is not ─────────────
// tun2socks always talks to ONE local port (`SOCKS_PORT`), whatever protocol
// the node speaks, because xray is always the front of the chain now. That is
// what makes changing location cheap: the engine behind the port is replaced
// and this file is not involved at all — the device, its address, both halves
// of the default route and DNS are never touched. A location change that tears
// the routes down is indistinguishable from "the internet went away", and this
// is the layer where that difference is decided.
//
// The old per-file watchdog is gone. It ran every five seconds, repaired
// routes, and on the death of tun2socks simply `return`ed — silently, into a
// green screen. Route repair still happens, but it is now something the
// supervisor in lib.rs CALLS (`ensure_routes`), so the one component with an
// AppHandle decides what the user is told. A watchdog with no voice is how a
// dead tunnel stayed green for as long as the window was open.
//
// ── Why the physical default route is read from netstat ────────────────────
// `route -n get default` answers with OUR tunnel once it is up: the two halves
// `0.0.0.0/1` and `128.0.0.0/1` are more specific than `0.0.0.0/0`, so the
// lookup lands on utun225 and the old code's repair path quietly gave up every
// time it was needed. The `default` entry itself is untouched and still names
// the real gateway, so the table is read directly instead.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use pvpn_platform::{net, paths, privilege};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::errors::{AppError, ErrorCode};
use crate::pidfile::Engine;

/// Name of the TUN device. The platform layer owns the value (`utun225` on
/// macOS) and a test below pins it, so this file cannot address one interface
/// while the platform layer configures another.
pub const TUN_NAME: &str = net::DEVICE;
pub const TUN_ADDR: &str = "198.18.0.1";

/// The only SOCKS port tun2socks ever speaks to. xray listens here for both
/// protocols; `xray_manager::FRONT_SOCKS_PORT` is the same number and a test
/// keeps the two from drifting.
pub const SOCKS_PORT: u16 = 10808;

/// How long the device gets to appear after tun2socks is spawned.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(5);
const DEVICE_POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct TunState {
    child: Option<Engine>,
    /// Resolved address of the node the host route points at. Never logged,
    /// never shown, never serialised.
    server_ip: Option<String>,
    /// How the machine really reaches the internet, as of the last time we
    /// looked. A change here means the network changed under us.
    physical: Option<PhysicalRoute>,
    /// The system resolver pointed into the tunnel (sysdns.rs). Dropping it
    /// hands DNS back to the real network; so does our death.
    dns: Option<crate::sysdns::TunnelDns>,
}

pub type SharedTunState = Arc<Mutex<TunState>>;

pub fn new_state() -> SharedTunState {
    Arc::new(Mutex::new(TunState::default()))
}

/// The real way out of this machine: gateway and interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalRoute {
    /// `Some("192.168.1.1")`, or `None` on a link-level default (cellular
    /// often has no gateway address of its own).
    pub gateway: Option<String>,
    /// `en0`, `pdp_ip0`, …
    pub interface: String,
}

impl PhysicalRoute {
    /// Arguments that send one host through this route rather than ours.
    fn via(&self) -> Vec<String> {
        match &self.gateway {
            Some(gw) => vec![gw.clone()],
            // A link-level default has no next-hop address to name; pointing
            // at the interface is the same instruction in the other form.
            None => vec!["-interface".to_string(), self.interface.clone()],
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Binary and privileges
// ───────────────────────────────────────────────────────────────────────────

pub fn tun2socks_path(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    crate::sidecars::find(app, "tun2socks", "tun")
}

/// May this process reconfigure interfaces and routes? Root on macOS (the
/// launcher arranges it); see `pvpn_platform::privilege` for the others.
pub fn is_root() -> bool {
    privilege::is_elevated()
}

// ───────────────────────────────────────────────────────────────────────────
// Reading the routing table
// ───────────────────────────────────────────────────────────────────────────

/// Pick the physical default out of `netstat -rn -f inet` output.
///
/// Pure so the shapes that matter — a gateway address, a link-level default,
/// our own tunnel sitting in the table beside it — are testable without a
/// machine in that state.
pub fn parse_physical_default(netstat: &str) -> Option<PhysicalRoute> {
    for line in netstat.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("default") {
            continue;
        }
        // Gateway, flags, interface. Some rows carry an expire column after
        // the interface, which is why the fourth field is taken rather than
        // the last. A short row is skipped rather than ending the scan: the
        // table often holds more than one `default`.
        let (Some(gateway), Some(_flags), Some(interface)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };

        // Another VPN, or our own tunnel if someone ever points 0/0 at it.
        if interface.starts_with("utun") || interface == TUN_NAME {
            continue;
        }
        let gateway = if gateway.parse::<std::net::Ipv4Addr>().is_ok() {
            Some(gateway.to_string())
        } else {
            None
        };
        return Some(PhysicalRoute {
            gateway,
            interface: interface.to_string(),
        });
    }
    None
}

/// The current physical default route.
///
/// `TunFailed` rather than a network code when there is none: a machine with
/// no default route is not a machine whose VPN failed, and the diagnostics
/// screen is where that distinction is drawn.
pub async fn physical_default() -> Result<PhysicalRoute, AppError> {
    let out = Command::new("/usr/sbin/netstat")
        .args(["-rn", "-f", "inet"])
        .output()
        .await
        .map_err(|e| {
            crate::logger::log("error", "tun", &format!("netstat failed: {e}"));
            AppError::new(ErrorCode::TunFailed)
        })?;

    parse_physical_default(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
        crate::logger::log("error", "tun", "no physical default route");
        AppError::new(ErrorCode::NetworkOffline)
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Bringing it up
// ───────────────────────────────────────────────────────────────────────────

async fn resolve_host(host: &str) -> Result<String, AppError> {
    let lookup = format!("{}:443", host);
    let resolved = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        lookup
            .to_socket_addrs()
            .ok()
            .map(|it| it.collect::<Vec<_>>())
    })
    .await
    .ok()
    .flatten();

    let addr = resolved
        .into_iter()
        .flatten()
        .find(|a| a.is_ipv4())
        .ok_or_else(|| {
            // The host name is NOT logged. It is a node address, and the log
            // is a file support asks people to paste into a chat.
            crate::logger::log("error", "tun", "node address did not resolve");
            AppError::new(ErrorCode::NoRoute)
        })?;
    Ok(addr.ip().to_string())
}

async fn run_cmd(program: &str, args: &[&str]) -> Result<(), AppError> {
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| {
            crate::logger::log("error", "tun", &format!("spawn {program}: {e}"));
            AppError::new(ErrorCode::TunFailed)
        })?;

    if !status.success() {
        // Route arguments can carry a node address, so the command is named
        // without them.
        crate::logger::log("error", "tun", &format!("{program} failed: {status}"));
        return Err(AppError::new(ErrorCode::TunFailed));
    }
    Ok(())
}

/// Route one host around the tunnel, through the physical network.
async fn add_host_route(ip: &str, via: &PhysicalRoute) -> Result<(), AppError> {
    let mut args: Vec<String> = vec!["-n".into(), "add".into(), "-host".into(), ip.into()];
    args.extend(via.via());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run_cmd("/sbin/route", &borrowed).await
}

async fn delete_host_route(ip: &str) {
    let _ = run_cmd("/sbin/route", &["-n", "delete", "-host", ip]).await;
}

async fn add_split_defaults() -> Result<(), AppError> {
    run_cmd(
        "/sbin/route",
        &["-n", "add", "-net", "0.0.0.0/1", "-interface", TUN_NAME],
    )
    .await?;
    run_cmd(
        "/sbin/route",
        &["-n", "add", "-net", "128.0.0.0/1", "-interface", TUN_NAME],
    )
    .await
}

pub async fn start(
    state: &SharedTunState,
    app: &tauri::AppHandle,
    server_host: &str,
) -> Result<(), AppError> {
    if !is_root() {
        // The launcher asks for the password before the window appears; being
        // here without root means the person dismissed it or started the
        // binary by hand. One code, one screen, one button.
        crate::logger::log("error", "tun", "not running as root");
        return Err(AppError::new(ErrorCode::PermissionDenied));
    }

    let mut guard = state.lock().await;
    if guard.child.is_some() {
        crate::logger::log("error", "tun", "start called while tun is up");
        return Err(AppError::new(ErrorCode::TunFailed));
    }

    let tun2socks = tun2socks_path(app)?;
    let server_ip = resolve_host(server_host).await?;
    let physical = physical_default().await?;

    crate::logger::log(
        "info",
        "tun",
        &format!("physical route via {}", physical.interface),
    );

    // Delete first: a stale host route from a crashed run points at a gateway
    // that may no longer exist, and `add` over it fails with "file exists".
    delete_host_route(&server_ip).await;
    add_host_route(&server_ip, &physical).await?;

    let child = match spawn_tun2socks(&tun2socks).await {
        Ok(child) => child,
        Err(err) => {
            delete_host_route(&server_ip).await;
            return Err(err);
        }
    };
    guard.child = Some(child);

    if let Err(err) = configure_device().await {
        if let Some(child) = guard.child.take() {
            child.stop().await;
        }
        delete_host_route(&server_ip).await;
        return Err(err);
    }

    guard.server_ip = Some(server_ip);
    guard.physical = Some(physical);

    // DNS last: only a tunnel that carries traffic may be the system's
    // resolver. A failure here is logged, not fatal — the tunnel works, and
    // most networks resolve through it anyway; only a Mac with encrypted DNS
    // bound to Wi-Fi (27.09.2026) needs this to see names at all.
    guard.dns = None;
    match crate::sysdns::TunnelDns::install(TUN_NAME, TUN_ADDR) {
        Ok(dns) => {
            guard.dns = Some(dns);
            crate::logger::log(
                "info",
                "tun",
                &format!("DNS системы: через туннель ({})", crate::sysdns::TUNNEL_DNS),
            );
        }
        Err(e) => crate::logger::log("warn", "tun", &format!("DNS системы не переключён: {e}")),
    }
    Ok(())
}

/// Spawn tun2socks and wait for the device to exist.
async fn spawn_tun2socks(bin: &std::path::Path) -> Result<Engine, AppError> {
    let mut cmd = Command::new(bin);
    cmd.args([
        "-device",
        TUN_NAME,
        "-proxy",
        &format!("socks5://127.0.0.1:{}", SOCKS_PORT),
        // `warn`, not `info`. At `info` tun2socks writes one line per
        // connection, which is a list of the addresses the person visited and
        // was the bulk of an 82.7 MB log file. The logger drops those lines
        // anyway; not producing them saves the IPC, the parsing and the disk.
        //
        // Именно `warn`. Стояло `warning` — такого уровня у tun2socks нет, он
        // принимает debug|info|warn|error|silent и на чужом падает сразу:
        // «[ENGINE] failed to start: unrecognized level». То есть ТУННЕЛЬ НЕ
        // ПОДНИМАЛСЯ ВООБЩЕ, а человек видел «Соединение не запустилось».
        // Найдено живым прогоном 22.09.2026; тесты этого не видели, потому
        // что строку никто не сверял с самим бинарём.
        "-loglevel",
        "warn",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    let mut child = Engine::spawn("tun2socks", &mut cmd, bin).map_err(|e| {
        crate::logger::log("error", "tun", &format!("spawn tun2socks: {e}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    // "info" as a default only: a line that names its own level keeps it.
    pump(child.child().stdout.take());
    pump(child.child().stderr.take());

    let deadline = tokio::time::Instant::now() + DEVICE_TIMEOUT;
    loop {
        if device_exists().await {
            return Ok(child);
        }
        if let Ok(Some(status)) = child.try_wait() {
            crate::logger::log("error", "tun", &format!("tun2socks exited: {status}"));
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
        if tokio::time::Instant::now() >= deadline {
            child.stop().await;
            crate::logger::log("error", "tun", "device did not appear in time");
            return Err(AppError::new(ErrorCode::TunFailed));
        }
        tokio::time::sleep(DEVICE_POLL).await;
    }
}

fn pump<R>(stream: Option<R>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let Some(stream) = stream else { return };
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            crate::logger::log("info", "tun2socks", &line);
        }
    });
}

async fn device_exists() -> bool {
    Command::new("/sbin/ifconfig")
        .arg(TUN_NAME)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Address on the device plus both halves of the default route.
async fn configure_device() -> Result<(), AppError> {
    run_cmd("/sbin/ifconfig", &[TUN_NAME, TUN_ADDR, TUN_ADDR, "up"]).await?;
    take_over_half_defaults().await;
    add_split_defaults().await
}

/// The tunnel is being raised because the person asked for it (Connect, or
/// the engine restart that keeps that connection): ours must be the
/// half-defaults that carry the traffic, so a /1 already in the table is
/// removed first, or `add_split_defaults` would fail on it. It used to happen
/// at every launch and quit too, see `half_default_is_ours`.
///
/// Whether a /1 is there at all is read from what `route get` PRINTS:
/// route(8) exits 0 for "not in table" as well, so its status said "present"
/// on every connect, logged a phantom other VPN and ran a delete that failed
/// as an `[error]`. Only a /1 on another interface is reported — the other
/// VPN keeps saying "connected" while its traffic now goes through ours.
async fn take_over_half_defaults() {
    for half in HALF_DEFAULTS {
        let out = Command::new("/sbin/route")
            .args(["-n", "get", "-net", half])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await;
        let found = match out {
            Ok(o) if o.status.success() => classify_half_default(&String::from_utf8_lossy(&o.stdout)),
            _ => HalfDefault::Absent,
        };
        match found {
            HalfDefault::Absent => {}
            HalfDefault::Ours => {
                let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", half]).await;
            }
            HalfDefault::Foreign => {
                crate::logger::log(
                    "warn",
                    "tun",
                    &format!("{half} was already routed elsewhere (another VPN?); replaced by ours for this session"),
                );
                let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", half]).await;
            }
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Keeping it up — called by the supervisor, which owns the voice
// ───────────────────────────────────────────────────────────────────────────

/// What one pass of route maintenance had to put back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RouteRepair {
    /// The route that keeps engine traffic out of our own tunnel. Losing it
    /// is what turns a working session into a connection storm.
    pub host_route: bool,
    /// One or both halves of the split default.
    pub split_default: bool,
    /// The machine changed how it reaches the internet — Wi-Fi to cellular,
    /// a cable pulled, a wake from sleep on a different network. The engine
    /// config names the old interface and has to be rebuilt.
    pub network_changed: bool,
}

impl RouteRepair {
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

/// Put back whatever the system took away, and report what that was.
///
/// Never touches tun2socks: a dead engine is the supervisor's business, and
/// re-adding routes to a device that no longer exists would only fail.
pub async fn ensure_routes(state: &SharedTunState) -> RouteRepair {
    let mut repair = RouteRepair::default();

    let (server_ip, known_physical) = {
        let guard = state.lock().await;
        (guard.server_ip.clone(), guard.physical.clone())
    };
    let Some(server_ip) = server_ip else {
        return repair;
    };

    let physical = match physical_default().await {
        Ok(p) => p,
        // No default route at all: the machine is offline. Nothing to repair,
        // and the probe layer is the one that says so out loud.
        Err(_) => return repair,
    };

    if known_physical.as_ref() != Some(&physical) {
        repair.network_changed = true;
        state.lock().await.physical = Some(physical.clone());
    }

    if !host_route_ok(&server_ip).await {
        delete_host_route(&server_ip).await;
        if add_host_route(&server_ip, &physical).await.is_ok() {
            repair.host_route = true;
            crate::logger::log("warn", "tun", "host route re-added");
        }
    }

    if !split_defaults_ok().await && add_split_defaults().await.is_ok() {
        repair.split_default = true;
        crate::logger::log("warn", "tun", "split default routes re-added");
    }

    repair
}

/// Does the host route still send the node around the tunnel?
async fn host_route_ok(ip: &str) -> bool {
    let Ok(out) = Command::new("/sbin/route")
        .args(["-n", "get", "-host", ip])
        .output()
        .await
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("interface:") && line.contains(TUN_NAME) {
            // Resolved through our own tunnel: the host route is gone and the
            // engine is about to talk to the node through itself.
            return false;
        }
    }
    text.contains("gateway:") || text.contains("interface:")
}

/// Do both halves of the default still point at our device?
async fn split_defaults_ok() -> bool {
    let Ok(out) = Command::new("/usr/sbin/netstat")
        .args(["-rn", "-f", "inet"])
        .output()
        .await
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_split_defaults(&text)
}

/// Pure half of the check above. `netstat` prints the two halves as `0/1` and
/// `128.0/1`, not in CIDR form, which is easy to get wrong without a fixture.
pub fn parse_split_defaults(netstat: &str) -> bool {
    let low = netstat
        .lines()
        .any(|l| l.starts_with("0/1") && l.contains(TUN_NAME));
    let high = netstat
        .lines()
        .any(|l| l.starts_with("128.0/1") && l.contains(TUN_NAME));
    low && high
}

/// Move the host route to a different node without dropping anything.
///
/// The new route is added BEFORE the old one is removed, so there is no
/// instant where the engine's traffic to a node has no way out but our own
/// tunnel. That instant is exactly how a location change turns into a
/// connection storm.
pub async fn retarget(state: &SharedTunState, server_host: &str) -> Result<(), AppError> {
    let new_ip = resolve_host(server_host).await?;
    let physical = physical_default().await?;

    let previous = {
        let guard = state.lock().await;
        guard.server_ip.clone()
    };
    if previous.as_deref() == Some(new_ip.as_str()) {
        // Same address behind a different label, or the same node again.
        return Ok(());
    }

    add_host_route(&new_ip, &physical).await?;
    {
        let mut guard = state.lock().await;
        guard.server_ip = Some(new_ip);
        guard.physical = Some(physical);
    }
    if let Some(old) = previous {
        delete_host_route(&old).await;
    }
    Ok(())
}

/// Bring tun2socks back after it died, leaving the host route alone.
pub async fn restart_engine(
    state: &SharedTunState,
    app: &tauri::AppHandle,
) -> Result<(), AppError> {
    let tun2socks = tun2socks_path(app)?;

    let mut guard = state.lock().await;
    if let Some(child) = guard.child.take() {
        child.stop().await;
    }
    // A process we lost the handle to still holds the device, and the new one
    // then fails to create it. Only a tun2socks WE started is swept; another
    // VPN client's one is none of our business.
    crate::pidfile::sweep("tun2socks").await;

    let child = spawn_tun2socks(&tun2socks).await?;
    guard.child = Some(child);
    configure_device().await
}

// ───────────────────────────────────────────────────────────────────────────
// Taking it down
// ───────────────────────────────────────────────────────────────────────────

/// The two halves of the default route the tunnel installs.
const HALF_DEFAULTS: [&str; 2] = ["0.0.0.0/1", "128.0.0.0/1"];

/// Does `route -n get -net <half>` describe OUR half-default — the /1 on
/// utun225 — rather than another VPN's?
///
/// `route delete -net 0.0.0.0/1` matches on destination and mask only, so it
/// removes whichever 0/1 is installed. Tunnelblick/OpenVPN (`redirect-gateway
/// def1`) and wg-quick install exactly these two routes; deleting them by
/// name at every launch, connect and quit silently sent the other VPN's
/// traffic out in the clear while it still said "connected".
fn half_default_is_ours(route_get: &str) -> bool {
    classify_half_default(route_get) == HalfDefault::Ours
}

/// What `route -n get -net <half>` printed about a /1 half-default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HalfDefault {
    /// No /1 in the table: "not in table" (on stderr, exit 0), or an answer
    /// about some other route.
    Absent,
    /// The /1 on utun225.
    Ours,
    /// A /1 on another interface — another VPN's `def1`.
    Foreign,
}

fn classify_half_default(route_get: &str) -> HalfDefault {
    let field = |name: &str| {
        route_get.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == name).then(|| value.trim().to_string())
        })
    };
    if field("mask").as_deref() != Some("128.0.0.0") {
        HalfDefault::Absent
    } else if field("interface").as_deref() == Some(TUN_NAME) {
        HalfDefault::Ours
    } else {
        HalfDefault::Foreign
    }
}

async fn delete_half_default_if_ours(half: &str) {
    let out = Command::new("/sbin/route")
        .args(["-n", "get", "-net", half])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;
    // route(8) exits 0 even for "not in table"; what it printed decides.
    let ours = out
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| half_default_is_ours(&String::from_utf8_lossy(&o.stdout)));
    if ours {
        let _ = run_cmd("/sbin/route", &["-n", "delete", "-net", half]).await;
    }
}

fn delete_half_default_if_ours_sync(half: &str) {
    use std::process::Command as SyncCommand;

    let ours = SyncCommand::new("/sbin/route")
        .args(["-n", "get", "-net", half])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| half_default_is_ours(&String::from_utf8_lossy(&o.stdout)));
    if ours {
        let _ = SyncCommand::new("/sbin/route")
            .args(["-n", "delete", "-net", half])
            .status();
    }
}

pub async fn stop(state: &SharedTunState) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    // DNS back to the real network FIRST, while the tunnel still carries
    // queries: the system never spends a moment with a resolver it cannot reach.
    guard.dns = None;
    let server_ip = guard.server_ip.take();

    // Only our own: see `half_default_is_ours`.
    for half in HALF_DEFAULTS {
        delete_half_default_if_ours(half).await;
    }
    if let Some(ref ip) = server_ip {
        delete_host_route(ip).await;
    }
    // Only when it is actually there. This runs before every connect
    // (`Core::engine_down`, not only on a real disconnect), and on a fresh
    // process — nothing raised yet — utun225 does not exist, so asking
    // `ifconfig` to bring a nonexistent device down always answers "exit
    // status: 1". `run_cmd` logs that as a real failure, which meant one
    // `[error]` line on every single connect, existent device or not (found
    // live 27.09.2026). A device that truly fails to go down is still an
    // error below, same as before.
    if device_exists().await {
        let _ = run_cmd("/sbin/ifconfig", &[TUN_NAME, "down"]).await;
    }

    if let Some(child) = guard.child.take() {
        child.stop().await;
    }
    crate::pidfile::sweep("tun2socks").await;

    guard.physical = None;
    forget_route_hint();
    Ok(())
}

// ───────────────────────────────────────────────────────────────────────────
// Surviving our own death
// ───────────────────────────────────────────────────────────────────────────

/// Where the host route of the live session is written down.
///
/// A crash or a `kill -9` leaves a host route pointing at a gateway that may
/// not exist on the next network, and nothing else on the machine knows the
/// address to remove. The file holds exactly that one address, readable only
/// by root, and it is the reason this stays inside `tun.rs`: the rest of the
/// program has no business learning a node address it cannot use.
const ROUTE_HINT_PATH: &str = "/tmp/proxysvpn-route-hint";

/// Write down the host route of the current session.
///
/// Not `fs::write`: this is a fixed name in mode-1777 `/tmp`, written by root,
/// so following a symlink a local user planted there would be a root-owned
/// write to whatever they pointed it at. `write_private_file` unlinks, then
/// creates with `O_EXCL | O_NOFOLLOW` and mode 0600 in one step.
pub async fn persist_route_hint(state: &SharedTunState) {
    let Some(ip) = state.lock().await.server_ip.clone() else {
        return;
    };
    let path = std::path::Path::new(ROUTE_HINT_PATH);
    if let Err(e) = paths::write_private_file(path, format!("{ip}\n").as_bytes()) {
        // A hint we could not write only costs a stale route on the next cold
        // start, which `sync_cleanup` also handles by other means.
        crate::logger::log("warn", "tun", &format!("route hint not written: {e:#}"));
    }
}

pub fn forget_route_hint() {
    let _ = std::fs::remove_file(ROUTE_HINT_PATH);
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid(2) has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The route hint is ours only when it is a plain file of our own effective
/// user that nobody else may read or write — what `write_private_file`
/// creates (0600).
fn route_hint_is_trusted(is_file: bool, uid: u32, mode: u32, euid: u32) -> bool {
    is_file && uid == euid && mode & 0o077 == 0
}

/// Remove routes a previous run left behind. Synchronous on purpose: it runs
/// at startup and on signals, where there is no runtime to await on.
pub fn purge_stale_routes() {
    use std::process::Command as SyncCommand;

    // Only our own: this runs at every launch and quit, while another VPN
    // may be up (`half_default_is_ours`).
    for half in HALF_DEFAULTS {
        delete_half_default_if_ours_sync(half);
    }
    let _ = SyncCommand::new("/sbin/ifconfig")
        .args([TUN_NAME, "down"])
        .status();

    // Only our own file is believed: a symlink in its place, or a file another
    // account left in the world-writable /tmp, was not written by
    // `persist_route_hint`. Read anyway, it would have root delete host routes
    // for whatever addresses another user listed (their VPN's server, the
    // gateway). Removed unread instead.
    let trusted = std::fs::symlink_metadata(ROUTE_HINT_PATH)
        .map(|m| {
            use std::os::unix::fs::MetadataExt;
            route_hint_is_trusted(m.file_type().is_file(), m.uid(), m.mode(), effective_uid())
        })
        .unwrap_or(false);
    if !trusted {
        let _ = std::fs::remove_file(ROUTE_HINT_PATH);
        return;
    }
    if let Ok(contents) = std::fs::read_to_string(ROUTE_HINT_PATH) {
        for ip in contents.lines().map(str::trim).filter(|l| !l.is_empty()) {
            // Validated before it reaches a command line: the file is ours,
            // but a file is a file.
            if ip.parse::<std::net::Ipv4Addr>().is_ok() {
                let _ = SyncCommand::new("/sbin/route")
                    .args(["-n", "delete", "-host", ip])
                    .status();
            }
        }
        let _ = std::fs::remove_file(ROUTE_HINT_PATH);
    }
}

/// Is OUR tun2socks alive?
///
/// Asked of the child handle rather than `pgrep`: a tun2socks belonging to
/// another application is not our tunnel, and counting it would hold the
/// screen green over a dead one.
pub async fn is_running(state: &SharedTunState) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE_WITH_TUNNEL: &str = "\
Routing tables

Internet:
Destination        Gateway            Flags               Netif Expire
0/1                utun225            USc                 utun225
default            192.168.1.1        UGScg                 en0
127                127.0.0.1          UCS                   lo0
128.0/1            utun225            USc                 utun225
198.18.0.1         198.18.0.1         UH                  utun225
";

    /// `route -n get -net 0.0.0.0/1` while our tunnel is up, while another
    /// VPN's def1 route holds it, and for the other half.
    #[test]
    fn only_our_half_default_is_ours_to_delete() {
        let ours = "   route to: 0.0.0.0\ndestination: 0.0.0.0\n       mask: 128.0.0.0\n  interface: utun225\n      flags: <UP,DONE,STATIC>\n";
        let high = "   route to: 128.0.0.0\ndestination: 128.0.0.0\n       mask: 128.0.0.0\n  interface: utun225\n      flags: <UP,DONE,STATIC>\n";
        let tunnelblick = "   route to: 0.0.0.0\ndestination: 0.0.0.0\n       mask: 128.0.0.0\n    gateway: 10.8.0.1\n  interface: utun4\n      flags: <UP,GATEWAY,DONE,STATIC>\n";
        let default = "   route to: default\ndestination: default\n       mask: default\n    gateway: 192.168.1.1\n  interface: en0\n";
        assert!(half_default_is_ours(ours));
        assert!(half_default_is_ours(high));
        assert!(!half_default_is_ours(tunnelblick), "another VPN's def1 route stays");
        assert!(!half_default_is_ours(default));
        assert!(!half_default_is_ours(""));
        // Same name, longer: utun2250 is not utun225.
        assert!(!half_default_is_ours(&ours.replace("utun225", "utun2250")));
    }

    /// What Connect does with each answer: nothing for "not in table" (route
    /// exits 0 for it and prints nothing on stdout), a quiet delete of our
    /// own leftover, and a warning only for a /1 that really is another VPN's.
    #[test]
    fn only_a_half_default_on_another_interface_is_reported_as_another_vpn() {
        let ours = "   route to: 0.0.0.0\ndestination: 0.0.0.0\n       mask: 128.0.0.0\n  interface: utun225\n";
        let wireguard = "   route to: 128.0.0.0\ndestination: 128.0.0.0\n       mask: 128.0.0.0\n  interface: utun3\n";
        let default = "   route to: default\ndestination: default\n       mask: default\n    gateway: 192.168.1.1\n  interface: en0\n";
        assert_eq!(classify_half_default(""), HalfDefault::Absent, "not in table");
        assert_eq!(
            classify_half_default("route: writing to routing socket: not in table\n"),
            HalfDefault::Absent
        );
        assert_eq!(classify_half_default(default), HalfDefault::Absent);
        assert_eq!(classify_half_default(ours), HalfDefault::Ours);
        assert_eq!(classify_half_default(wireguard), HalfDefault::Foreign);
    }

    /// Another account on the Mac can leave a file at the fixed /tmp name;
    /// root must not delete host routes for the addresses it lists.
    #[test]
    fn only_our_own_private_route_hint_is_believed() {
        assert!(route_hint_is_trusted(true, 0, 0o100600, 0));
        assert!(!route_hint_is_trusted(true, 501, 0o100600, 0), "another user's file");
        assert!(!route_hint_is_trusted(true, 0, 0o100644, 0), "readable by others");
        assert!(!route_hint_is_trusted(true, 0, 0o100606, 0), "writable by others");
        assert!(!route_hint_is_trusted(false, 0, 0o120600, 0), "a link");
    }

    #[test]
    fn the_physical_default_is_found_under_our_own_tunnel() {
        // The whole reason this parser exists: `route get default` answers
        // "utun225" here, and the old repair path gave up because of it.
        let route = parse_physical_default(TABLE_WITH_TUNNEL).expect("route");
        assert_eq!(route.gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(route.interface, "en0");
    }

    #[test]
    fn a_link_level_default_has_no_gateway_address() {
        // Cellular interfaces routinely look like this.
        let table = "default            link#14            UCSI            pdp_ip0\n";
        let route = parse_physical_default(table).expect("route");
        assert_eq!(route.gateway, None);
        assert_eq!(route.interface, "pdp_ip0");
        assert_eq!(route.via(), vec!["-interface", "pdp_ip0"]);
    }

    #[test]
    fn a_gateway_default_routes_by_next_hop() {
        let route = PhysicalRoute {
            gateway: Some("10.0.0.1".into()),
            interface: "en0".into(),
        };
        assert_eq!(route.via(), vec!["10.0.0.1"]);
    }

    #[test]
    fn another_vpns_default_is_never_ours_to_use() {
        // Sending the node's host route through somebody else's tunnel would
        // wrap our traffic in theirs, which is not what anyone asked for.
        let table = "default            10.8.0.1           UGSc                utun3\n";
        assert!(parse_physical_default(table).is_none());
    }

    #[test]
    fn an_empty_table_yields_nothing_rather_than_a_guess() {
        assert!(parse_physical_default("").is_none());
        assert!(parse_physical_default("Routing tables\n\nInternet:\n").is_none());
    }

    #[test]
    fn split_defaults_are_recognised_in_netstat_shorthand() {
        assert!(parse_split_defaults(TABLE_WITH_TUNNEL));
    }

    #[test]
    fn one_missing_half_is_not_good_enough() {
        let half = TABLE_WITH_TUNNEL
            .lines()
            .filter(|l| !l.starts_with("128.0/1"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!parse_split_defaults(&half));
    }

    #[test]
    fn routes_pointing_somewhere_else_do_not_count() {
        let table = "0/1                10.0.0.1           USc                   en0\n\
                     128.0/1            10.0.0.1           USc                   en0\n";
        assert!(!parse_split_defaults(table));
    }

    /// The device name in this module and the one the platform layer knows
    /// must be the same string, and on macOS it must stay the one every
    /// installed copy already cleans up after.
    #[test]
    fn device_name_comes_from_the_platform_layer() {
        assert_eq!(TUN_NAME, net::DEVICE);
        assert!(net::is_our_device(TUN_NAME));
        assert_eq!(TUN_NAME, "utun225");
    }

    #[test]
    fn device_address_is_the_shared_one() {
        assert_eq!(TUN_ADDR, net::DEVICE_ADDR.to_string());
    }

    /// The Linux helper refuses any SOCKS port that is not one of this app's
    /// own engine ports, because the port is where every packet on the machine
    /// ends up once the half-defaults are installed. The allow-list lives in
    /// the platform crate, which cannot see these constants, so nothing but
    /// this test keeps the values together.
    #[test]
    fn engine_ports_are_the_ones_the_helper_accepts() {
        use pvpn_platform::helper::proto::ALLOWED_SOCKS_PORTS;
        assert!(
            ALLOWED_SOCKS_PORTS.contains(&SOCKS_PORT),
            "xray's inbound {} is not in {:?}",
            SOCKS_PORT,
            ALLOWED_SOCKS_PORTS
        );
        assert!(
            ALLOWED_SOCKS_PORTS.contains(&crate::hysteria_manager::HY2_SOCKS_PORT),
            "hysteria's inbound is not in {:?}",
            ALLOWED_SOCKS_PORTS
        );
    }

    #[test]
    fn a_clean_pass_reports_nothing() {
        assert!(RouteRepair::default().is_clean());
        assert!(!RouteRepair {
            host_route: true,
            ..Default::default()
        }
        .is_clean());
    }
}
