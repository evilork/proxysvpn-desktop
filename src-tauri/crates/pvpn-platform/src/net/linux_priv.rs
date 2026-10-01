// src-tauri/crates/pvpn-platform/src/net/linux_priv.rs
//
// The privileged half of the Linux backend: the TUN device, the routing table
// and DNS. This runs inside the root helper (see crate::helper), and inline when
// the whole app happens to be root — `sudo` on X11, where a root GUI still
// works.
//
// Deliberately synchronous: a root process with no async runtime and no shared
// state is far easier to reason about, and every step here is a short-lived
// `ip` or `resolvectl` call. The argv comes from `net::plan::linux`, which is
// pure and unit-tested on the developer's Mac.
//
// DNS is the one thing macOS and Windows do not need. There, the system
// resolver keeps using the physical link's servers and the queries are proxied
// like any other traffic under 0.0.0.0/1. On Linux the usual upstream is the
// LAN router (192.168.x.1), which stays *on-link* and is therefore reached
// directly, outside the tunnel — a real leak — and systemd-resolved can pin a
// query to the physical link regardless of routes. So here we must take DNS
// over, and give it back on every exit path.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use crate::net::linux_logic::{self as logic, DnsBackend, LinuxDefaultRoute};
use crate::net::plan::{linux as p, SPLIT_HIGH, SPLIT_LOW};
use crate::net::TUNNEL_ENGINE;
use crate::paths;
use crate::process::Argv;

pub use crate::net::plan::linux::DEVICE;

const PROC_NET_ROUTE: &str = "/proc/net/route";
const RESOLV_CONF: &str = "/etc/resolv.conf";
const DEVICE_WAIT: Duration = Duration::from_millis(100);
const DEVICE_WAIT_TRIES: u32 = 50;

/// What the root side acts on: `helper::proto::UpParams` after it has been
/// validated at the privilege boundary. There is deliberately no second
/// "validated" struct here — one type, validated in one place, is the whole
/// point of the boundary.
pub use crate::helper::proto::ValidUp;

/// A tunnel that is up, with everything needed to re-assert or remove it.
pub struct Tunnel {
    child: Child,
    server_ip: Ipv4Addr,
    dns: Vec<IpAddr>,
    backend: DnsBackend,
}

impl Tunnel {
    /// Which node this tunnel was raised for. The helper checks an `Ensure`
    /// request against this, so a repair can never be turned into a way to aim
    /// our routes at a different address.
    pub fn server_ip(&self) -> Ipv4Addr {
        self.server_ip
    }
}

fn log(level: &str, message: &str) {
    crate::helper::emit_log(level, "tun", message);
}

/// Absolute path of a system tool.
///
/// pkexec resets PATH to a safe default, and distros disagree on /sbin vs
/// /usr/sbin, so look in all four places before falling back to PATH.
fn tool(name: &str) -> Result<PathBuf> {
    const DIRS: &[&str] = &["/usr/sbin", "/sbin", "/usr/bin", "/bin"];
    for dir in DIRS {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    crate::privilege::which(name).ok_or_else(|| anyhow!("{} not found (install iproute2?)", name))
}

/// Runs a planned command, resolving its bare program name to an absolute path.
fn run(argv: &Argv) -> Result<()> {
    let program = tool(&argv.program)?;
    let out = Command::new(&program)
        .args(&argv.args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("spawn {}", argv))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(anyhow!("{} failed ({}): {}", argv, out.status, stderr))
}

/// For teardown steps where "already gone" and "removed" are the same outcome.
fn run_ok(argv: &Argv) {
    if let Err(e) = run(argv) {
        log("info", &format!("ignored: {}", e));
    }
}

fn stdout_of(argv: &Argv) -> Option<String> {
    let program = tool(&argv.program).ok()?;
    let out = Command::new(&program)
        .args(&argv.args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

// ------------------------------------------------------------------- reads

/// The machine's physical IPv4 default, read from `/proc/net/route`.
///
/// The kernel's own ABI rather than `ip route show`: no locale to parse, and —
/// unlike `ip route get <addr>` — it cannot be answered by the half-defaults we
/// installed ourselves.
fn physical_default() -> Result<LinuxDefaultRoute> {
    let text = std::fs::read_to_string(PROC_NET_ROUTE)
        .with_context(|| format!("read {}", PROC_NET_ROUTE))?;
    logic::parse_physical_default(&text).ok_or_else(|| {
        anyhow!("нет физического маршрута по умолчанию — проверьте подключение к сети")
    })
}

fn split_defaults_ok() -> bool {
    match std::fs::read_to_string(PROC_NET_ROUTE) {
        Ok(text) => {
            let (low, high) = logic::parse_split_defaults(&text, DEVICE);
            low && high
        }
        Err(_) => false,
    }
}

/// Does a packet to the node still leave over the physical link?
fn host_route_ok(ip: Ipv4Addr) -> bool {
    match stdout_of(&p::host_route_query(ip)) {
        Some(text) => match logic::parse_ip_route_get_dev(&text) {
            Some(dev) => dev != DEVICE,
            None => false,
        },
        None => false,
    }
}

fn device_exists() -> bool {
    Path::new("/sys/class/net").join(DEVICE).exists()
}

// ------------------------------------------------------------------ writes

fn add_host_route(ip: Ipv4Addr, route: &LinuxDefaultRoute) -> Result<()> {
    run(&p::host_route_add(ip, route)).context("pin the node address to the physical link")
}

fn del_host_route(ip: Ipv4Addr) {
    run_ok(&p::host_route_delete(ip));
}

fn add_split_defaults() -> Result<()> {
    for half in [SPLIT_LOW, SPLIT_HIGH] {
        run(&p::split_default_add(half))
            .with_context(|| format!("route {} into {}", half, DEVICE))?;
    }
    Ok(())
}

fn configure_device() -> Result<()> {
    run(&p::device_set_address()).context("assign the tunnel address")?;
    run(&p::device_up()).context("bring the tunnel device up")?;
    Ok(())
}

fn spawn_tun2socks(params: &ValidUp) -> Result<Child> {
    let argv = p::tun2socks(&params.tun2socks, params.socks_port);
    let mut child = Command::new(&params.tun2socks)
        .args(&argv.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", params.tun2socks.display()))?;

    pump(child.stdout.take(), "info");
    pump(child.stderr.take(), "warn");
    Ok(child)
}

/// Forward a sidecar's output into the GUI's log, on its own thread: the
/// helper's main loop must stay free to read stdin, and a full pipe would
/// otherwise block tun2socks itself.
fn pump<R>(stream: Option<R>, level: &'static str)
where
    R: std::io::Read + Send + 'static,
{
    let Some(stream) = stream else { return };
    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stream);
        for line in reader.lines() {
            match line {
                Ok(line) => crate::helper::emit_log(level, TUNNEL_ENGINE, &line),
                Err(_) => break,
            }
        }
    });
}

// --------------------------------------------------------------------- DNS

fn current_dns_backend() -> DnsBackend {
    logic::dns_backend(
        tool("resolvectl").is_ok(),
        Path::new("/run/systemd/resolve").is_dir(),
    )
}

fn dns_up(backend: DnsBackend, servers: &[IpAddr]) -> Result<()> {
    match backend {
        DnsBackend::Resolved => {
            run(&p::resolved_set_servers(servers)).context("resolvectl dns")?;
            run(&p::resolved_set_domain()).context("resolvectl domain")?;
            // Not fatal: without it resolved may still consult the physical
            // link in parallel, which is a leak but not a breakage.
            run_ok(&p::resolved_set_default_route());
            run_ok(&p::resolved_flush());
            Ok(())
        }
        DnsBackend::ResolvConf => {
            logic::resolv_apply(&paths::linux_runtime_dir(), Path::new(RESOLV_CONF), servers)
                .context("rewrite /etc/resolv.conf")
        }
    }
}

fn dns_down(backend: DnsBackend) {
    match backend {
        DnsBackend::Resolved => {
            // The link is about to disappear, which reverts it anyway; doing it
            // explicitly covers the case where the device survives.
            run_ok(&p::resolved_revert());
            run_ok(&p::resolved_flush());
        }
        DnsBackend::ResolvConf => {
            match logic::resolv_restore(&paths::linux_runtime_dir(), Path::new(RESOLV_CONF)) {
                Ok(true) => log("info", "/etc/resolv.conf restored"),
                Ok(false) => {}
                Err(e) => log("warn", &format!("could not restore /etc/resolv.conf: {}", e)),
            }
        }
    }
}

/// Is our resolver file still in place, or did NetworkManager rewrite it?
fn resolv_conf_is_ours() -> bool {
    match std::fs::read_to_string(RESOLV_CONF) {
        Ok(text) => text.contains(logic::RESOLV_MARKER),
        Err(_) => false,
    }
}

// ----------------------------------------------------------------- the hint

fn write_hint(server_ip: Ipv4Addr) {
    let dir = paths::linux_runtime_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log("warn", &format!("could not create {}: {}", dir.display(), e));
        return;
    }
    let hint = crate::net::format_route_hint(std::process::id(), server_ip);
    let path = dir.join(paths::ROUTE_HINT_NAME);
    if let Err(e) = std::fs::write(&path, hint) {
        log("warn", &format!("could not write the route hint: {}", e));
    }
}

fn hint_path() -> PathBuf {
    paths::linux_runtime_dir().join(paths::ROUTE_HINT_NAME)
}

// ------------------------------------------------------------------ the API

/// Raise the tunnel. On any failure everything this function changed is undone
/// before the error is returned.
pub fn up(params: &ValidUp) -> Result<Tunnel> {
    let route = physical_default()?;
    // The node address is not logged: it is not public information.
    log("info", &format!("physical exit: {}", route.iface));

    add_host_route(params.server_ip, &route)?;

    let mut child = match spawn_tun2socks(params) {
        Ok(child) => child,
        Err(e) => {
            del_host_route(params.server_ip);
            return Err(e);
        }
    };

    let backend = current_dns_backend();
    let result = (|| -> Result<()> {
        let mut ready = false;
        for _ in 0..DEVICE_WAIT_TRIES {
            std::thread::sleep(DEVICE_WAIT);
            if device_exists() {
                ready = true;
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(anyhow!("tun2socks exited early: {}", status));
            }
        }
        if !ready {
            return Err(anyhow!("{} did not appear within 5s", DEVICE));
        }
        configure_device()?;
        add_split_defaults()?;
        dns_up(backend, &params.dns)?;
        Ok(())
    })();

    if let Err(e) = result {
        let _ = child.kill();
        let _ = child.wait();
        dns_down(backend);
        run_ok(&p::device_delete());
        del_host_route(params.server_ip);
        return Err(e);
    }

    write_hint(params.server_ip);
    log("info", &format!("tunnel up on {} ({:?} DNS)", DEVICE, backend));

    Ok(Tunnel {
        child,
        server_ip: params.server_ip,
        dns: params.dns.clone(),
        backend,
    })
}

/// Supervisor tick: a network change, a wake from suspend or a DHCP renew drops
/// our routes, and NetworkManager may rewrite `/etc/resolv.conf` behind us.
pub fn ensure(tunnel: &Tunnel) -> Result<()> {
    if !host_route_ok(tunnel.server_ip) {
        let route = physical_default()?;
        add_host_route(tunnel.server_ip, &route)?;
        log("warn", "re-pinned the node host route");
    }

    if !split_defaults_ok() {
        add_split_defaults()?;
        log("warn", "re-added the split-default routes");
        // The device was rebuilt, so per-link resolver settings are gone too.
        dns_up(tunnel.backend, &tunnel.dns)?;
    } else if tunnel.backend == DnsBackend::ResolvConf && !resolv_conf_is_ours() {
        dns_up(tunnel.backend, &tunnel.dns)?;
        log("warn", "/etc/resolv.conf was rewritten, reapplied ours");
    }
    Ok(())
}

pub fn engine_alive(tunnel: &mut Tunnel) -> bool {
    matches!(tunnel.child.try_wait(), Ok(None))
}

/// Remove everything `up` created. Idempotent.
pub fn down(mut tunnel: Tunnel) {
    let _ = tunnel.child.kill();
    let _ = tunnel.child.wait();
    dns_down(tunnel.backend);
    run_ok(&p::device_delete());
    del_host_route(tunnel.server_ip);
    let _ = std::fs::remove_file(hint_path());
    log("info", "tunnel down");
}

/// Crash recovery, run before the helper touches anything: a previous run may
/// have died with our host route, device and resolver file still installed.
pub fn purge_stale_sync() {
    run_ok(&p::kill_stray(TUNNEL_ENGINE));

    if device_exists() {
        run_ok(&p::device_delete());
    }

    let hint = hint_path();
    if let Ok(text) = std::fs::read_to_string(&hint) {
        for ip in crate::net::parse_route_hint(&text) {
            del_host_route(ip);
        }
        let _ = std::fs::remove_file(&hint);
    }

    // Reverting a link that does not exist is harmless and covers the case
    // where the device survived us.
    run_ok(&p::resolved_revert());
    if logic::resolv_backup_exists(&paths::linux_runtime_dir()) {
        dns_down(DnsBackend::ResolvConf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iproute2_is_available_on_this_host() {
        assert!(
            tool("ip").is_ok(),
            "iproute2 must be installed on a Linux build host"
        );
    }

    #[test]
    fn the_tunnel_address_stays_inside_the_benchmark_range() {
        assert_eq!(crate::net::DEVICE_ADDR.octets()[0], 198);
        assert_eq!(p::PREFIX, 15);
    }
}
