// src-tauri/src/tun/sys/linux/net.rs
//! Privileged Linux primitives: the TUN device, the routing table, DNS.
//!
//! Runs inside the root helper (`crate::helper`), and inline when the whole app
//! happens to be root. Deliberately synchronous: a root process with no async
//! runtime and no shared state is much easier to reason about, and every step
//! here is a short-lived `ip`/`resolvectl` call.
//!
//! Routing shape, same as macOS and Windows: two half-defaults (`0.0.0.0/1` and
//! `128.0.0.0/1`) on the tunnel plus a /32 host route to the node over the
//! physical link. Half-defaults beat the real `0.0.0.0/0` on prefix length
//! without deleting it, so another VPN's default and the ISP's stay intact and
//! come back untouched when we leave. Policy routing (`ip rule` + a private
//! table, or tun2socks' `-fwmark`) would be the alternative; it is only needed
//! when something else already claims `/1`, which no common Linux VPN does.

use anyhow::{anyhow, Context, Result};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::super::linux_logic as logic;
use crate::helper::proto::ValidUp;
use crate::paths;
use logic::{DnsBackend, PhysicalRoute};

pub const TUN_NAME: &str = "proxysvpn0";
/// 198.18.0.0/15 (RFC 2544 benchmark range) — the subnet tun2socks' own docs
/// use, and not routed on the public internet.
const TUN_PREFIX: u8 = 15;
/// tun2socks terminates TCP locally and re-opens it towards the SOCKS proxy, so
/// the device is not an encapsulating tunnel and does not need a reduced MTU.
const TUN_MTU: u32 = 1500;

const RESOLV_CONF: &str = "/etc/resolv.conf";
const PROC_NET_ROUTE: &str = "/proc/net/route";
const DEVICE_WAIT: Duration = Duration::from_millis(100);
const DEVICE_WAIT_TRIES: u32 = 50;

/// A tunnel that is up, with everything needed to re-assert or remove it.
pub struct Tunnel {
    child: Child,
    server_ip: Ipv4Addr,
    dns: Vec<IpAddr>,
    backend: DnsBackend,
}

impl Tunnel {
    /// Which node this tunnel was raised for; the helper checks that an
    /// `Ensure` request is about the tunnel it actually holds.
    pub fn server_ip(&self) -> Ipv4Addr {
        self.server_ip
    }
}

fn log(level: &str, message: &str) {
    crate::helper::emit_log(level, "tun", message);
}

/// Absolute path of a system tool. pkexec resets PATH to a safe default, but
/// distros disagree on `/sbin` vs `/usr/sbin`, so look in both.
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

fn run(program: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("spawn {} {:?}", program.display(), args))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(anyhow!(
        "{} {:?} failed ({}): {}",
        program.display(),
        args,
        out.status,
        stderr
    ))
}

fn run_ok(program: &Path, args: &[&str]) {
    if let Err(e) = run(program, args) {
        log("warn", &format!("ignoring: {}", e));
    }
}

fn ip_tool() -> Result<PathBuf> {
    tool("ip")
}

fn physical_default() -> Result<PhysicalRoute> {
    let text = std::fs::read_to_string(PROC_NET_ROUTE)
        .with_context(|| format!("read {}", PROC_NET_ROUTE))?;
    logic::parse_physical_default(&text).ok_or_else(|| {
        anyhow!("нет физического маршрута по умолчанию — проверьте подключение к сети")
    })
}

fn as_args(owned: &[String]) -> Vec<&str> {
    owned.iter().map(String::as_str).collect()
}

fn add_host_route(ip: &Ipv4Addr, route: &PhysicalRoute) -> Result<()> {
    let ip_bin = ip_tool()?;
    let owned = logic::host_route_args(ip, route);
    run(&ip_bin, &as_args(&owned)).context("pin the node address to the physical link")
}

fn del_host_route(ip: &Ipv4Addr) {
    if let Ok(ip_bin) = ip_tool() {
        run_ok(&ip_bin, &["-4", "route", "del", &format!("{}/32", ip)]);
    }
}

fn add_split_defaults() -> Result<()> {
    let ip_bin = ip_tool()?;
    for net in ["0.0.0.0/1", "128.0.0.0/1"] {
        run(&ip_bin, &["-4", "route", "replace", net, "dev", TUN_NAME])
            .with_context(|| format!("add route {} via {}", net, TUN_NAME))?;
    }
    Ok(())
}

fn split_defaults_ok() -> bool {
    match std::fs::read_to_string(PROC_NET_ROUTE) {
        Ok(text) => {
            let (low, high) = logic::parse_split_defaults(&text, TUN_NAME);
            low && high
        }
        Err(_) => false,
    }
}

/// Does a packet to the node still leave over the physical link?
fn host_route_ok(ip: &Ipv4Addr) -> bool {
    let Ok(ip_bin) = ip_tool() else { return false };
    let target = ip.to_string();
    let out = Command::new(&ip_bin)
        .args(["-4", "route", "get", &target])
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            match logic::parse_ip_route_get_dev(&text) {
                Some(dev) => dev != TUN_NAME,
                None => false,
            }
        }
        _ => false,
    }
}

fn device_exists() -> bool {
    Path::new("/sys/class/net").join(TUN_NAME).exists()
}

fn configure_device() -> Result<()> {
    let ip_bin = ip_tool()?;
    let addr = format!("{}/{}", crate::tun::TUN_ADDR, TUN_PREFIX);
    run(&ip_bin, &["-4", "addr", "replace", &addr, "dev", TUN_NAME])
        .context("assign the tunnel address")?;
    let mtu = TUN_MTU.to_string();
    run(&ip_bin, &["link", "set", "dev", TUN_NAME, "up", "mtu", &mtu])
        .context("bring the tunnel device up")?;
    Ok(())
}

fn spawn_tun2socks(params: &ValidUp) -> Result<Child> {
    let proxy = format!("socks5://127.0.0.1:{}", params.socks_port);
    let device = format!("tun://{}", TUN_NAME);
    let mtu = TUN_MTU.to_string();

    let mut child = Command::new(&params.tun2socks)
        .args([
            "-device",
            &device,
            "-proxy",
            &proxy,
            "-mtu",
            &mtu,
            // `warn`, never `warning`: the long form makes tun2socks exit.
            "-loglevel",
            "warn",
        ])
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
/// helper's main loop must stay free to read stdin.
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
                Ok(line) => crate::helper::emit_log(level, "tun2socks", &line),
                Err(_) => break,
            }
        }
    });
}

fn resolvectl() -> Option<PathBuf> {
    tool("resolvectl").ok()
}

fn current_dns_backend() -> DnsBackend {
    logic::dns_backend(
        resolvectl().is_some(),
        Path::new("/run/systemd/resolve").is_dir(),
    )
}

/// Point the system resolver at servers that are only reachable through the
/// tunnel.
///
/// This is the one step macOS does not need. The macOS resolver keeps using the
/// physical link's servers and those queries are simply proxied, because their
/// addresses fall under `0.0.0.0/1`. On Linux the usual upstream is the LAN
/// router (`192.168.x.1`), which stays on-link and therefore leaks; and
/// systemd-resolved can pin a query to the physical link regardless of routes.
fn dns_up(backend: DnsBackend, servers: &[IpAddr]) -> Result<()> {
    match backend {
        DnsBackend::Resolved => {
            let bin = resolvectl().ok_or_else(|| anyhow!("resolvectl disappeared"))?;
            let mut args = vec!["dns", TUN_NAME];
            let rendered: Vec<String> = servers.iter().map(|ip| ip.to_string()).collect();
            args.extend(rendered.iter().map(String::as_str));
            run(&bin, &args).context("resolvectl dns")?;
            // `~.` makes the tunnel the resolver of last resort for every name.
            run(&bin, &["domain", TUN_NAME, "~."]).context("resolvectl domain")?;
            run_ok(&bin, &["default-route", TUN_NAME, "yes"]);
            run_ok(&bin, &["flush-caches"]);
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
            if let Some(bin) = resolvectl() {
                // The link is about to disappear, which reverts it anyway; doing
                // it explicitly covers the case where the device survives.
                run_ok(&bin, &["revert", TUN_NAME]);
                run_ok(&bin, &["flush-caches"]);
            }
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

fn write_hint(server_ip: &Ipv4Addr) {
    let dir = paths::linux_runtime_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log("warn", &format!("could not create {}: {}", dir.display(), e));
        return;
    }
    let hint = crate::tun::render_hint(std::process::id(), &server_ip.to_string());
    if let Err(e) = std::fs::write(paths::route_hint(), hint) {
        log("warn", &format!("could not write the route hint: {}", e));
    }
}

/// Raise the tunnel. On any failure everything this function changed is undone
/// before the error is returned.
pub fn up(params: &ValidUp) -> Result<Tunnel> {
    let route = physical_default()?;
    log(
        "info",
        &format!("physical exit: {} (gateway {})", route.iface, match route.gateway {
            Some(gw) => gw.to_string(),
            None => "link".to_string(),
        }),
    );

    add_host_route(&params.server_ip, &route)?;

    let mut child = match spawn_tun2socks(params) {
        Ok(child) => child,
        Err(e) => {
            del_host_route(&params.server_ip);
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
            return Err(anyhow!("{} did not appear within 5s", TUN_NAME));
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
        if let Ok(ip_bin) = ip_tool() {
            run_ok(&ip_bin, &["link", "del", TUN_NAME]);
        }
        del_host_route(&params.server_ip);
        return Err(e);
    }

    write_hint(&params.server_ip);
    log("info", &format!("tunnel up on {} ({:?} DNS)", TUN_NAME, backend));

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
    if !host_route_ok(&tunnel.server_ip) {
        let route = physical_default()?;
        add_host_route(&tunnel.server_ip, &route)?;
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
    if let Ok(ip_bin) = ip_tool() {
        run_ok(&ip_bin, &["link", "del", TUN_NAME]);
    }
    del_host_route(&tunnel.server_ip);
    let _ = std::fs::remove_file(paths::route_hint());
    log("info", "tunnel down");
}

/// Crash recovery, run before the helper touches anything: a previous run may
/// have died with our host route, device and resolver file still installed.
pub fn purge_stale_sync() {
    let _ = Command::new("pkill").args(["-9", "-x", "tun2socks"]).status();

    if let Ok(ip_bin) = ip_tool() {
        if device_exists() {
            run_ok(&ip_bin, &["link", "del", TUN_NAME]);
        }
    }

    let hint = paths::route_hint();
    if let Ok(text) = std::fs::read_to_string(&hint) {
        if let Some(ip) = crate::tun::parse_hint_server_ip(&text) {
            if let Ok(addr) = ip.parse::<Ipv4Addr>() {
                del_host_route(&addr);
            }
        }
        let _ = std::fs::remove_file(&hint);
    }

    if let Some(bin) = resolvectl() {
        run_ok(&bin, &["revert", TUN_NAME]);
    }
    if logic::resolv_backup_exists(&paths::linux_runtime_dir()) {
        dns_down(DnsBackend::ResolvConf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_address_and_prefix_stay_inside_the_benchmark_range() {
        let addr: Ipv4Addr = crate::tun::TUN_ADDR.parse().expect("addr");
        assert_eq!(TUN_PREFIX, 15);
        assert_eq!(addr.octets()[0], 198);
    }

    #[test]
    fn iproute2_is_available_on_this_host() {
        assert!(ip_tool().is_ok(), "iproute2 must be installed on a Linux build host");
    }
}
