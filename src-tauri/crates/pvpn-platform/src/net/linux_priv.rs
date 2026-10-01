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

/// Removes both halves explicitly.
///
/// An earlier version left this to `ip link del`, on the theory that routes die
/// with their device. They do — *when the delete succeeds*. If the device is
/// busy, rtnetlink errors or another owner holds it, `0.0.0.0/1` and
/// `128.0.0.0/1` stay in the table pointing at a device that is gone, and the
/// machine has no IPv4 after "disconnect" with nothing retrying, because
/// `run_ok` only logs. macOS and Windows have always deleted them by hand.
fn del_split_defaults() {
    for half in [SPLIT_LOW, SPLIT_HIGH] {
        run_ok(&p::split_default_delete(half));
    }
}

fn configure_device() -> Result<()> {
    run(&p::device_set_address()).context("assign the tunnel address")?;
    run(&p::device_up()).context("bring the tunnel device up")?;
    Ok(())
}

fn spawn_tun2socks(params: &ValidUp, tun2socks: &Path) -> Result<Child> {
    let argv = p::tun2socks(tun2socks, params.socks_port);
    let mut cmd = Command::new(tun2socks);
    cmd.args(&argv.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    die_with_parent(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", tun2socks.display()))?;

    pump(child.stdout.take(), "info");
    pump(child.stderr.take(), "warn");
    Ok(child)
}

/// The child gets SIGKILL when the thread that spawned it ends
/// (PR_SET_PDEATHSIG), so a helper that is SIGKILLed or panics takes its
/// root tun2socks with it instead of leaving an orphan until reboot.
///
/// "The thread", not "the process": that is the kernel's rule, and why this
/// is only used from `up`, which the helper runs on its main thread (the
/// request loop in `helper::server`) — that thread ends only with the
/// process. If the helper already died before the child got this far, the
/// signal would never come, so the child checks its parent and refuses to
/// start.
fn die_with_parent(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: getpid(2) has no preconditions.
    let parent = unsafe { libc::getpid() };
    // SAFETY: the closure runs in the forked child before exec and calls
    // only prctl(2) and getppid(2), both async-signal-safe system calls; it
    // allocates nothing and touches no lock.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
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
            // The persistent directory, not the runtime one: our replacement
            // /etc/resolv.conf is on disk and survives a reboot, so the copy of
            // the original has to as well. See paths::linux_persistent_dir.
            logic::resolv_apply(&paths::linux_persistent_dir(), Path::new(RESOLV_CONF), servers)
                .context("rewrite /etc/resolv.conf")
        }
    }
}

/// `dns_up` for the supervisor tick: with the resolv.conf backend, a file
/// that replaced ours mid-session becomes the new original
/// (`linux_logic::resolv_reapply`).
fn dns_reapply(backend: DnsBackend, servers: &[IpAddr]) -> Result<()> {
    match backend {
        DnsBackend::Resolved => dns_up(backend, servers),
        DnsBackend::ResolvConf => {
            logic::resolv_reapply(&paths::linux_persistent_dir(), Path::new(RESOLV_CONF), servers)
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
            // Only over our own file: one NetworkManager wrote since (after
            // a reboot, on a new network) is newer than the backup and stays.
            match logic::resolv_restore(&paths::linux_persistent_dir(), Path::new(RESOLV_CONF)) {
                Ok(logic::ResolvRestore::Restored) => log("info", "/etc/resolv.conf restored"),
                Ok(logic::ResolvRestore::KeptNewer) => log(
                    "info",
                    "/etc/resolv.conf was rewritten after connect; kept it and dropped the old backup",
                ),
                Ok(logic::ResolvRestore::NoBackup) => {}
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

fn write_hint(server_ip: Ipv4Addr, engine_pid: Option<u32>) {
    let dir = paths::linux_runtime_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log("warn", &format!("could not create {}: {}", dir.display(), e));
        return;
    }
    let hint = match engine_pid {
        Some(pid) => crate::net::format_route_hint_with_engine(std::process::id(), server_ip, pid),
        None => crate::net::format_route_hint(std::process::id(), server_ip),
    };
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
/// before the error is returned. `tun2socks` is the helper's own sidecar
/// (`helper::server::own_sidecar`), never a path from the peer.
pub fn up(params: &ValidUp, tun2socks: &Path) -> Result<Tunnel> {
    let route = physical_default()?;
    // The node address is not logged: it is not public information.
    log("info", &format!("physical exit: {}", route.iface));

    // The hint goes down before anything it describes exists, and is updated
    // with the engine's pid the moment there is one: a helper killed half way
    // through `up` (the 5 s device wait, DNS) used to leave a host route and a
    // root tun2socks that no later purge knew about.
    write_hint(params.server_ip, None);
    if let Err(e) = add_host_route(params.server_ip, &route) {
        let _ = std::fs::remove_file(hint_path());
        return Err(e);
    }

    let mut child = match spawn_tun2socks(params, tun2socks) {
        Ok(child) => child,
        Err(e) => {
            del_host_route(params.server_ip);
            let _ = std::fs::remove_file(hint_path());
            return Err(e);
        }
    };
    write_hint(params.server_ip, Some(child.id()));

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
        // Same relative order as the rollback in `net::local` (shared by macOS
        // and Windows), which its fake-backend test pins: DNS first, then the
        // split defaults, then the engine, and the host route last — the engine
        // still needs the node reachable while it shuts down. The device delete
        // sits with the engine because on Linux, unlike utun and Wintun, the
        // device does not disappear with the process.
        dns_down(backend);
        del_split_defaults();
        let _ = child.kill();
        let _ = child.wait();
        run_ok(&p::device_delete());
        del_host_route(params.server_ip);
        let _ = std::fs::remove_file(hint_path());
        return Err(e);
    }

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
        dns_reapply(tunnel.backend, &tunnel.dns)?;
    } else if tunnel.backend == DnsBackend::ResolvConf && !resolv_conf_is_ours() {
        dns_reapply(tunnel.backend, &tunnel.dns)?;
        log("warn", "/etc/resolv.conf was rewritten, reapplied ours and kept the new one for Disconnect");
    }
    Ok(())
}

/// Move the tunnel to another node: the new host route goes in before the old
/// one comes out, so the engine never has to reach a node through the tunnel
/// itself. Everything else — device, split defaults, DNS, tun2socks — stays.
/// On failure the old route is untouched and the tunnel still points at it.
pub fn retarget(tunnel: &mut Tunnel, new_ip: Ipv4Addr) -> Result<()> {
    if tunnel.server_ip == new_ip {
        return Ok(());
    }
    let route = physical_default()?;
    add_host_route(new_ip, &route)?;
    del_host_route(tunnel.server_ip);
    tunnel.server_ip = new_ip;
    write_hint(new_ip, Some(tunnel.child.id()));
    // The addresses are not logged: node addresses are not public information.
    log("info", "host route moved to the new node");
    Ok(())
}

pub fn engine_alive(tunnel: &mut Tunnel) -> bool {
    matches!(tunnel.child.try_wait(), Ok(None))
}

/// Remove everything `up` created. Idempotent.
///
/// Driven by `TEARDOWN_ORDER` rather than written out, for the same reason
/// `net::local::down` is: the order is the part that is easy to get subtly
/// wrong, the rationale for each position lives with the constant, and a host
/// test pins it. Writing it out here is how this backend ended up killing the
/// engine first and never deleting the split defaults at all.
pub fn down(mut tunnel: Tunnel) {
    use crate::net::{TeardownStep, TEARDOWN_ORDER};

    for step in TEARDOWN_ORDER {
        match step {
            TeardownStep::SplitDefaults => del_split_defaults(),
            TeardownStep::HostRoute => del_host_route(tunnel.server_ip),
            TeardownStep::RestoreDns => dns_down(tunnel.backend),
            TeardownStep::DeviceDown => run_ok(&p::device_delete()),
            TeardownStep::KillOwnedEngine => {
                let _ = tunnel.child.kill();
                let _ = tunnel.child.wait();
            }
            // Nothing to sweep: the engine this tunnel owns was killed by
            // handle in the step before. `pkill -u 0 tun2socks` here used to
            // kill any root tun2socks on the machine — another VPN client's
            // privileged service included — on every Disconnect.
            TeardownStep::KillStrayEngines => {}
        }
    }
    let _ = std::fs::remove_file(hint_path());
    log("info", "tunnel down");
}

/// Crash recovery, run before the helper touches anything: a previous run may
/// have died with our host route, device and resolver file still installed.
pub fn purge_stale_sync() {
    // Routes before the device, as in `down`: a crash can leave the halves in
    // the table even when the device is already gone, and then the machine has
    // no IPv4 until something removes them.
    del_split_defaults();
    let hint = hint_path();
    let hint_text = std::fs::read_to_string(&hint).ok();
    // Only the tun2socks the previous helper recorded, and only while that
    // pid still runs our own binary: a name sweep (`pkill -u 0 tun2socks`)
    // also killed another VPN client's root engine at every helper start.
    if let Some(text) = &hint_text {
        for pid in crate::net::parse_engine_pids(text) {
            kill_own_engine(pid);
        }
    }

    if device_exists() {
        run_ok(&p::device_delete());
    }

    if let Some(text) = &hint_text {
        for ip in crate::net::parse_route_hint(text) {
            del_host_route(ip);
        }
        let _ = std::fs::remove_file(&hint);
    }

    // Reverting a link that does not exist is harmless and covers the case
    // where the device survived us.
    run_ok(&p::resolved_revert());
    if logic::resolv_backup_exists(&paths::linux_persistent_dir()) {
        dns_down(DnsBackend::ResolvConf);
    }
}

/// SIGKILL `pid` if, and only if, it still runs the helper's own tun2socks.
fn kill_own_engine(pid: u32) {
    let Ok(raw) = libc::pid_t::try_from(pid) else {
        return;
    };
    let own_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let image = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    let ours = match (image, own_dir) {
        (Some(image), Some(own)) => crate::net::is_own_engine_image(&image, &own),
        _ => false,
    };
    if !ours {
        return;
    }
    // SAFETY: kill(2) takes two integers; `raw` is a single pid above 1
    // (parse_engine_pids), never a group or "everyone".
    if unsafe { libc::kill(raw, libc::SIGKILL) } == 0 {
        log("info", "stopped the tun2socks a previous helper left running");
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

    /// PR_SET_PDEATHSIG is set: the spawning thread ends, the child dies
    /// with it. (The same rule is why `up` must spawn from the main thread.)
    #[test]
    fn a_child_spawned_to_die_with_its_parent_does() {
        let child = std::thread::spawn(|| {
            let mut cmd = Command::new("sleep");
            cmd.arg("30").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            die_with_parent(&mut cmd);
            cmd.spawn().expect("spawn sleep")
        })
        .join()
        .expect("spawning thread");
        let mut child = child;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().expect("try_wait") {
                use std::os::unix::process::ExitStatusExt;
                assert_eq!(status.signal(), Some(libc::SIGKILL));
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the child outlived the thread that spawned it");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_tunnel_address_stays_inside_the_benchmark_range() {
        assert_eq!(crate::net::DEVICE_ADDR.octets()[0], 198);
        assert_eq!(p::PREFIX, 15);
    }
}
