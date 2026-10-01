// src-tauri/crates/pvpn-platform/src/net/linux.rs
//
// Linux backend, GUI side. Nothing here is privileged.
//
// Every operation is a line of JSON sent to the root helper (crate::helper),
// which is this very executable restarted through pkexec with `--helper`. The
// helper is spawned once and kept for the whole app session, so the user sees a
// single authorization dialog — the same feel as the macOS launcher. It is our
// child, so when we die its stdin reaches EOF and it tears the tunnel down by
// itself; that is the crash path, and it needs no cooperation from us.
//
// Why a helper instead of an elevated GUI, as on macOS and Windows:
//
//   1. A root process cannot connect to a Wayland compositor, so an elevated
//      GUI simply never shows a window. On X11 it would need
//      `xhost +si:localuser:root`. macOS has no equivalent problem because
//      `launchctl asuser` keeps the root process inside the GUI session.
//   2. File capabilities (`setcap cap_net_admin+ep`) are not inherited by child
//      processes, so tun2socks would still lack them, and capabilities are lost
//      inside an AppImage mount — the .deb and the AppImage would need two
//      different privilege models. They share this one: the AppImage runs a
//      root-owned copy of the same helper (helper/install.rs), because root
//      cannot execute from its mount.
//   3. It is also simply better: the webview, which renders remote content,
//      never runs as root. That is stricter than what macOS and Windows do
//      today, and the contract is shaped so they can follow.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::helper::install::{self, Launch};
use crate::helper::proto::{self, Frame, Request, UpParams};
use crate::helper::{DEV_ENV, HELPER_DEV_FLAG, HELPER_FLAG};
use crate::net::linux_logic as logic;
use crate::net::{IfCounters, PhysicalRoute, TunPlan};
use crate::privilege;

pub use crate::net::plan::linux::DEVICE;

/// The user may need a while to type the password into the polkit dialog.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(180);
/// Ordinary requests: the helper only runs `ip`/`resolvectl`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

struct Link {
    child: Child,
    stdin: ChildStdin,
    out: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    /// The start of pkexec's stderr, for telling "no polkit agent" apart from
    /// a refusal when the handshake fails (`privilege::pkexec_unavailable`).
    stderr: std::sync::Arc<std::sync::Mutex<String>>,
    /// This start runs the AppImage's setup before the helper
    /// (helper/install.rs), so a failed handshake may be the setup's.
    setup: bool,
}

/// How much of pkexec's stderr is kept for that diagnosis.
const STDERR_KEEP: usize = 4096;

static LINK: OnceLock<Mutex<Option<Link>>> = OnceLock::new();

/// Last known answer to "is tun2socks alive". Read when the pipe is busy, so a
/// status poll never waits behind the authorization dialog.
static ENGINE_UP: AtomicBool = AtomicBool::new(false);

fn link_slot() -> &'static Mutex<Option<Link>> {
    LINK.get_or_init(|| Mutex::new(None))
}

fn up_params(plan: &TunPlan) -> UpParams {
    UpParams {
        server_ip: plan.server_ip.to_string(),
        socks_port: plan.socks_port,
        dns: plan.dns_strings(),
    }
}

/// Send one request and wait for its answer, mirroring the helper's log frames
/// into the app log on the way.
async fn request(link: &mut Link, req: Request, timeout: Duration) -> Result<bool> {
    link.next_id += 1;
    let id = link.next_id;
    let line = proto::encode(&Frame::Request { id, req }).context("encode helper request")?;
    link.stdin
        .write_all(line.as_bytes())
        .await
        .context("write to helper")?;
    link.stdin.flush().await.context("flush helper pipe")?;

    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(anyhow!("helper did not answer within {:?}", timeout));
        }
        let read = tokio::time::timeout(left, link.out.next_line()).await;
        let raw = match read {
            Err(_) => return Err(anyhow!("helper did not answer within {:?}", timeout)),
            Ok(Err(e)) => return Err(anyhow!("helper pipe error: {}", e)),
            Ok(Ok(None)) => return Err(anyhow!("helper closed the pipe")),
            Ok(Ok(Some(line))) => line,
        };
        if raw.trim().is_empty() {
            continue;
        }
        match proto::decode(&raw) {
            Ok(Frame::Log {
                level,
                source,
                message,
            }) => crate::log::log(&level, &source, &message),
            Ok(Frame::Response {
                id: rid,
                ok,
                error,
                engine_alive,
            }) if rid == id => {
                return if ok {
                    Ok(engine_alive)
                } else {
                    Err(anyhow!(
                        "{}",
                        error.unwrap_or_else(|| "helper reported a failure".to_string())
                    ))
                };
            }
            // A stale answer to a request that already timed out.
            Ok(Frame::Response { .. }) | Ok(Frame::Request { .. }) => {}
            Err(e) => crate::log::warn("helper", &format!("unparsable frame from helper: {}", e)),
        }
    }
}

/// What to start for the helper, and what to tidy up once it has answered.
struct Spawn {
    cmd: Command,
    /// The AppImage's staged files (helper/install.rs), removed after the
    /// handshake whether it worked or not.
    staged: Option<std::path::PathBuf>,
    setup: bool,
}

/// Is this process the AppImage's own executable? See `install::launch_kind`.
fn launch_kind(exe: &std::path::Path) -> Launch {
    let appdir = std::env::var_os("APPDIR")
        .filter(|dir| !dir.is_empty())
        .and_then(|dir| std::fs::canonicalize(dir).ok());
    install::launch_kind(std::env::var_os("APPIMAGE").as_deref(), appdir.as_deref(), exe)
}

fn spawn_command() -> Result<Spawn> {
    let exe = std::env::current_exe().context("locate our own executable")?;
    // Debug builds only. The helper ignores the flag in a release build anyway
    // (helper/proto.rs::dev_mode_allowed), but a release argv should not even
    // contain it.
    let dev = cfg!(debug_assertions)
        && std::env::var(DEV_ENV).map(|v| v == "1").unwrap_or(false);

    if privilege::is_elevated() {
        // Already root (the user ran us with sudo on X11): no dialog needed,
        // just fork the helper. Root mounted the AppImage in that case, so
        // even the AppImage's own executable is reachable.
        let mut cmd = Command::new(&exe);
        cmd.arg(HELPER_FLAG);
        if dev {
            cmd.arg(HELPER_DEV_FLAG);
        }
        return Ok(Spawn { cmd, staged: None, setup: false });
    }

    if launch_kind(&exe) == Launch::AppImage {
        let pkexec = privilege::which("pkexec")
            .ok_or_else(|| anyhow::Error::new(privilege::ElevationUnavailable::NoPkexec))?;
        // Typed, so the window shows "could not prepare its files" rather
        // than a refusal the person never gave.
        let plan = install::plan_appimage_spawn(&exe, install::running_on_steamos()).map_err(anyhow::Error::new)?;
        crate::log::info(
            "helper",
            if plan.setup {
                "AppImage: copying the helper into /home/.proxysvpn, then starting it (one password window)"
            } else {
                "AppImage: starting the helper copied into /home/.proxysvpn"
            },
        );
        let mut cmd = Command::new(pkexec);
        cmd.arg(&plan.program).args(&plan.args);
        return Ok(Spawn { cmd, staged: plan.staged, setup: plan.setup });
    }

    // Typed, not text: the GUI tells "this machine cannot elevate" apart
    // from "the person said no" by this type (ELEVATION_UNAVAILABLE).
    if let Some(reason) = privilege::elevation_blocker() {
        return Err(anyhow::Error::new(reason));
    }
    let pkexec = privilege::which("pkexec")
        .ok_or_else(|| anyhow::Error::new(privilege::ElevationUnavailable::NoPkexec))?;
    let mut cmd = Command::new(pkexec);
    cmd.arg(&exe).arg(HELPER_FLAG);
    if dev {
        cmd.arg(HELPER_DEV_FLAG);
    }
    Ok(Spawn { cmd, staged: None, setup: false })
}

async fn spawn_helper() -> Result<Link> {
    let Spawn { cmd, staged, setup } = spawn_command()?;
    let linked = start_helper(cmd, setup).await;
    if let Some(dir) = staged {
        // Root has copied the files by now, or never will.
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            crate::log::warn("helper", &format!("could not remove {}: {e}", dir.display()));
        }
    }
    linked
}

async fn start_helper(mut cmd: Command, setup: bool) -> Result<Link> {
    use std::process::Stdio;

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn the privileged helper")?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("helper stdin not captured"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("helper stdout not captured"))?;
    let stderr_seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(err) = child.stderr.take() {
        // pkexec reports authorization problems on stderr.
        let seen = std::sync::Arc::clone(&stderr_seen);
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut kept) = seen.lock() {
                    if kept.len() < STDERR_KEEP {
                        kept.push_str(&line);
                        kept.push('\n');
                    }
                }
                crate::log::warn("helper", &line);
            }
        });
    }

    let mut link = Link {
        child,
        stdin,
        out: BufReader::new(stdout).lines(),
        next_id: 0,
        stderr: stderr_seen,
        setup,
    };

    match request(&mut link, Request::Hello, HANDSHAKE_TIMEOUT).await {
        Ok(_) => Ok(link),
        Err(e) => Err(describe_handshake_failure(&mut link, e).await),
    }
}

/// Turn a dead pkexec child into something a user can act on.
async fn describe_handshake_failure(link: &mut Link, cause: anyhow::Error) -> anyhow::Error {
    use privilege::HandshakeFailure;

    let code = match link.child.try_wait() {
        Ok(Some(status)) => status.code(),
        _ => None,
    };
    if code.is_some() {
        // pkexec(1) exits 127 both for "not authorized" and for "could not
        // ask at all", and the AppImage's setup names itself there too: only
        // stderr tells them apart. Give its reader a moment to catch up.
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let stderr = link.stderr.lock().map(|s| s.clone()).unwrap_or_default();
    let failure = privilege::handshake_failure(code, &stderr, link.setup);
    // SteamOS: the deck user has no password until one is set, and Gaming
    // Mode has no window to type it into. Say which, instead of "denied".
    if install::running_on_steamos() {
        if let Some(advice) = privilege::steamos_advice_for(failure) {
            return anyhow::Error::new(advice);
        }
    }
    match failure {
        // pkexec(1): 126 — the dialog was dismissed.
        HandshakeFailure::Dismissed => {
            anyhow!("запрос прав отменён — без пароля администратора туннель не поднять")
        }
        // Typed, so the window says "this system cannot ask" instead of
        // "press Retry and allow it in the system dialog".
        HandshakeFailure::NoAgent => anyhow::Error::new(privilege::ElevationUnavailable::NoAgent),
        HandshakeFailure::NotAuthorized => {
            anyhow!("polkit отказал в правах — запустите приложение из рабочего стола или от root")
        }
        HandshakeFailure::SetupFailed(code) => anyhow::Error::new(privilege::HelperSetupFailed(format!(
            "the setup exited with {code}: {}",
            stderr.lines().filter(|line| line.contains(install::SETUP_NAME)).collect::<Vec<_>>().join("; ")
        ))),
        HandshakeFailure::Other => cause.context("helper handshake failed"),
    }
}

/// Make sure a live helper is on the other end of the pipe.
async fn ensure_link() -> Result<()> {
    let mut slot = link_slot().lock().await;
    if let Some(link) = slot.as_mut() {
        match link.child.try_wait() {
            Ok(None) => return Ok(()),
            _ => {
                crate::log::warn("helper", "helper is gone, starting a new one");
                *slot = None;
            }
        }
    }
    *slot = Some(spawn_helper().await?);
    Ok(())
}

async fn send(req: Request) -> Result<bool> {
    ensure_link().await?;
    let mut slot = link_slot().lock().await;
    let link = slot
        .as_mut()
        .ok_or_else(|| anyhow!("helper link disappeared"))?;
    request(link, req, REQUEST_TIMEOUT).await
}

// ----------------------------------------------------------------- contract

/// Start the helper and let the user authorize it *before* anything else
/// happens, so a dismissed dialog is a clean "not connected" rather than a
/// half-built tunnel.
pub async fn preflight() -> Result<()> {
    ensure_link().await
}

pub async fn up(plan: &TunPlan) -> Result<()> {
    let result = send(Request::Up(up_params(plan))).await.map(|_| ());
    ENGINE_UP.store(result.is_ok(), Ordering::Relaxed);
    result
}

pub async fn ensure(plan: &TunPlan) -> Result<()> {
    send(Request::Ensure(up_params(plan))).await.map(|_| ())
}

/// Move the live tunnel to another node. The helper knows which node it holds,
/// so `_old` is not sent: the two sides must not disagree about it.
pub async fn retarget(_old: Option<Ipv4Addr>, plan: &TunPlan) -> Result<()> {
    send(Request::Retarget(up_params(plan))).await.map(|_| ())
}

// ------------------------------------------------------- read-only facts
//
// Answered by the GUI itself: `/proc/net/route`, `/proc/net/ipv6_route` and
// `/sys/class/net` are world-readable, so none of this needs the helper, a
// round trip over its pipe or the password.

const PROC_NET_ROUTE: &str = "/proc/net/route";
const PROC_NET_IPV6_ROUTE: &str = "/proc/net/ipv6_route";

/// How the machine reaches the internet outside the tunnel. Read from the
/// kernel's table rather than `ip route get`, which would answer with our own
/// half-defaults once they are up.
pub async fn physical_route() -> Result<PhysicalRoute> {
    let text = std::fs::read_to_string(PROC_NET_ROUTE)
        .with_context(|| format!("read {}", PROC_NET_ROUTE))?;
    logic::parse_physical_default(&text)
        .map(|route| PhysicalRoute::from(&route))
        .ok_or_else(|| anyhow!("нет физического маршрута по умолчанию — проверьте подключение к сети"))
}

/// Byte counters of the tunnel device, `None` while it does not exist.
pub fn device_counters() -> Option<IfCounters> {
    if !logic::is_plain_iface_name(DEVICE) {
        return None;
    }
    let dir = std::path::Path::new("/sys/class/net")
        .join(DEVICE)
        .join("statistics");
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|text| logic::parse_counter(&text))
    };
    Some(IfCounters {
        rx_bytes: read("rx_bytes")?,
        tx_bytes: read("tx_bytes")?,
    })
}

/// Does the machine have a way out at all — an IPv4 or IPv6 default route on
/// something that is not a tunnel? Answers "your network is off" apart from
/// "our service is broken" without sending a packet.
pub fn has_usable_link() -> bool {
    let v4 = std::fs::read_to_string(PROC_NET_ROUTE)
        .map(|text| logic::parse_physical_default(&text).is_some())
        .unwrap_or(false);
    v4 || std::fs::read_to_string(PROC_NET_IPV6_ROUTE)
        .map(|text| logic::parse_ipv6_physical_default(&text))
        .unwrap_or(false)
}

/// Tear the tunnel down but keep the helper alive: a reconnect must not ask for
/// the password again.
pub async fn down(_server_ip: Option<Ipv4Addr>) -> Result<()> {
    ENGINE_UP.store(false, Ordering::Relaxed);
    let mut slot = link_slot().lock().await;
    let Some(link) = slot.as_mut() else {
        return Ok(());
    };
    if matches!(link.child.try_wait(), Ok(None)) {
        request(link, Request::Down, REQUEST_TIMEOUT)
            .await
            .map(|_| ())
    } else {
        // The helper died; it already took the tunnel with it.
        *slot = None;
        Ok(())
    }
}

pub async fn engine_alive() -> bool {
    // Never block here: the UI polls this, and a request in flight may be the
    // pkexec dialog waiting for a password (up to three minutes).
    let Ok(mut slot) = link_slot().try_lock() else {
        return ENGINE_UP.load(Ordering::Relaxed);
    };
    let Some(link) = slot.as_mut() else {
        ENGINE_UP.store(false, Ordering::Relaxed);
        return false;
    };
    if !matches!(link.child.try_wait(), Ok(None)) {
        ENGINE_UP.store(false, Ordering::Relaxed);
        return false;
    }
    let alive = match request(link, Request::Status, REQUEST_TIMEOUT).await {
        Ok(alive) => alive,
        Err(e) => {
            crate::log::warn("helper", &format!("status request failed: {}", e));
            false
        }
    };
    ENGINE_UP.store(alive, Ordering::Relaxed);
    alive
}

/// Synchronous crash recovery at startup.
///
/// The unprivileged GUI cannot undo routes, so normally there is nothing to do
/// here: the helper purges on its own startup, before it touches anything. When
/// the app itself was started as root (sudo on X11) we do the purge inline.
///
/// `stale_hosts` is ignored on purpose — the helper writes its hint under /run,
/// reads it back from there, and the two sides must not disagree about who owns
/// the cleanup.
pub fn purge_stale(_stale_hosts: &[Ipv4Addr]) {
    if privilege::is_elevated() {
        crate::net::linux_priv::purge_stale_sync();
    }
}
