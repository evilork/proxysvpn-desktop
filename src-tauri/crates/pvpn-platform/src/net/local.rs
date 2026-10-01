// src-tauri/crates/pvpn-platform/src/net/local.rs
//
// The tunnel sequence for the platforms where the app itself is privileged:
// macOS (root through scripts/launcher.sh) and Windows (elevated through the
// application manifest). Linux is not one of them — its GUI is unprivileged and
// this whole sequence runs inside the root helper, so `net::linux` does not use
// this module.
//
// WHY THIS IS SHARED
// macOS and Windows differ in their primitives (route vs netsh, ifconfig vs IP
// Helper) but not in the *order* those primitives must run, and that order is
// the part that is easy to get subtly wrong — specifically, every early return
// has to undo what the function already did. Writing it twice would be two
// chances to forget an undo, so the sequence lives here once and each platform
// supplies the steps through `Privileged`.
//
// The sequence is a transcription of the pre-split tun.rs, step for step, so
// macOS keeps doing exactly what it did for the users who are on it today.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::log;
use crate::net::{engine, PhysicalRoute, TeardownStep, TunPlan, TEARDOWN_ORDER, TUNNEL_ENGINE};
use crate::process::Argv;

/// How long the device may take to appear after tun2socks starts. Ten
/// seconds, not five: a Wintun adapter on a slow Windows machine (an x64
/// tun2socks emulated on ARM, 02.10.2026) took longer than five to come up.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);

/// The steps one privileged platform provides. Implemented by a zero-sized type
/// in `macos.rs` and `windows.rs`; never used as `dyn`, so the async methods
/// cost nothing.
pub trait Privileged {
    /// Name of the TUN device, for log lines.
    const DEVICE: &'static str;

    /// Argv for the engine. Lives in `plan` so golden tests can pin it.
    fn tun2socks_argv(bin: &Path, socks_port: u16) -> Argv;

    /// How packets leave the machine outside the tunnel. Must read the
    /// *physical* default even while our two halves are installed.
    async fn physical_route() -> Result<PhysicalRoute>;

    async fn add_host_route(dest: Ipv4Addr, via: &PhysicalRoute) -> Result<()>;
    async fn delete_host_route(dest: Ipv4Addr);
    /// Does a packet to `dest` still leave outside the tunnel?
    async fn host_route_ok(dest: Ipv4Addr) -> bool;

    async fn add_split_defaults() -> Result<()>;
    async fn delete_split_defaults();
    async fn split_defaults_ok() -> bool;

    /// The device is created by tun2socks, not by us; this waits for it.
    async fn wait_for_device(timeout: Duration) -> Result<()>;
    async fn configure_device() -> Result<()>;
    async fn device_down();

    /// Publish resolvers on the tunnel interface. A no-op on both platforms
    /// that use this module — see each implementation for why.
    async fn configure_dns(servers: &[IpAddr]) -> Result<()>;
    async fn restore_dns();

    async fn kill_stray(stem: &str);

    // --- engine control -----------------------------------------------------
    // Defaulted: both real platforms spawn tun2socks themselves and share one
    // handle in `net::engine`. They exist as trait methods only so the tests
    // below can drive the whole sequence with a fake that starts no process —
    // which is the only way to assert that every failure path rolls back.

    async fn start_engine(bin: &Path, socks_port: u16) -> Result<()> {
        let argv = Self::tun2socks_argv(bin, socks_port);
        engine::spawn(bin, &argv.args).await
    }

    /// `Some(reason)` when the engine is already gone.
    async fn engine_exited() -> Option<String> {
        engine::exited().await.map(|status| status.to_string())
    }

    async fn stop_engine() {
        engine::kill().await
    }

    /// `Some(who)` when a process that is not one of our engines listens
    /// where tun2socks connects (127.0.0.1:`port`). Every packet of the
    /// machine goes there once the split defaults are in, and between the
    /// stop and the start of an xray restart the port is free for anyone to
    /// take. Defaulted to "nobody": macOS keeps exactly what it did; Windows
    /// asks its TCP table (net::windows).
    async fn socks_port_stranger(_port: u16) -> Option<String> {
        None
    }
}

/// The error for a SOCKS port held by someone else. Russian, like the other
/// messages of this layer that reach the person through a failure.
fn stranger_error(port: u16, who: &str) -> anyhow::Error {
    anyhow!("порт {port} занят чужой программой ({who}) — туннель на него не направлен")
}

/// Raise the tunnel.
///
/// Order, and why it cannot be rearranged:
///   1. host route to the node through the physical link — so the engines can
///      still reach it once the default route points inside the tunnel;
///   2. start tun2socks, which creates the TUN device itself;
///   3. address the device;
///   4. install 0.0.0.0/1 + 128.0.0.0/1, which are more specific than the
///      physical default and therefore win without deleting it;
///   5. DNS last, because it is the only step that depends on the tunnel
///      already carrying traffic.
///
/// Every failure after step 1 undoes the steps before it, so a failed connect
/// never leaves a route or a process behind.
pub async fn up<P: Privileged>(plan: &TunPlan) -> Result<()> {
    let physical = P::physical_route().await?;

    // The node's address is deliberately absent from the log: it is not public
    // information and the log is attached to support tickets.
    log::info("tun", &format!("physical exit: {}", physical));
    log::info("tun", &format!("tun2socks: {}", plan.tun2socks.display()));

    P::add_host_route(plan.server_ip, &physical).await?;

    // From here on, `undo` is the only way out.
    let undo = |label: &'static str, e: anyhow::Error| async move {
        log::warn("tun", &format!("{} failed, rolling back: {}", label, e));
        e
    };

    if let Err(e) = P::start_engine(&plan.tun2socks, plan.socks_port).await {
        P::delete_host_route(plan.server_ip).await;
        return Err(undo("start tun2socks", e).await);
    }

    if let Err(e) = P::wait_for_device(DEVICE_TIMEOUT).await {
        // An engine that died on its own explains the missing device far better
        // than "it did not appear in 5s".
        let reason = match P::engine_exited().await {
            Some(status) => anyhow!("tun2socks exited early: {}", status),
            None => e,
        };
        P::stop_engine().await;
        P::delete_host_route(plan.server_ip).await;
        return Err(undo("wait for the device", reason).await);
    }

    if let Err(e) = P::configure_device().await {
        P::stop_engine().await;
        P::delete_host_route(plan.server_ip).await;
        return Err(undo("configure the device", e).await);
    }

    // The last moment before the machine is pointed at the SOCKS port.
    if let Some(who) = P::socks_port_stranger(plan.socks_port).await {
        P::stop_engine().await;
        P::delete_host_route(plan.server_ip).await;
        return Err(undo("check the SOCKS port", stranger_error(plan.socks_port, &who)).await);
    }

    if let Err(e) = P::add_split_defaults().await {
        P::delete_split_defaults().await;
        P::stop_engine().await;
        P::delete_host_route(plan.server_ip).await;
        return Err(undo("install the split defaults", e).await);
    }

    if let Err(e) = P::configure_dns(&plan.dns).await {
        P::restore_dns().await;
        P::delete_split_defaults().await;
        P::stop_engine().await;
        P::delete_host_route(plan.server_ip).await;
        return Err(undo("configure DNS", e).await);
    }

    log::info(
        "tun",
        &format!("device {} up with {}", P::DEVICE, crate::net::DEVICE_ADDR),
    );
    Ok(())
}

/// Supervisor tick: re-install what the OS dropped underneath us.
///
/// Waking from sleep, a Wi-Fi change or a DHCP renewal can wipe our entries
/// while the engine stays alive; without this the app looks connected and
/// nothing flows.
pub async fn ensure<P: Privileged>(plan: &TunPlan) -> Result<()> {
    // Someone else took the port the machine is pointed at — while xray
    // restarted, typically. Lower the tunnel rather than keep feeding them:
    // the GUI sees the engine gone and says protection dropped, and its next
    // `up` is refused for as long as the stranger holds the port.
    if let Some(who) = P::socks_port_stranger(plan.socks_port).await {
        let refusal = stranger_error(plan.socks_port, &who);
        log::warn("watchdog", &format!("{refusal:#}"));
        down::<P>(Some(plan.server_ip)).await?;
        return Err(refusal);
    }

    let mut trouble: Option<anyhow::Error> = None;

    if !P::host_route_ok(plan.server_ip).await {
        match P::physical_route().await {
            Ok(route) => match P::add_host_route(plan.server_ip, &route).await {
                Ok(()) => log::warn("watchdog", &format!("re-added host route via {}", route)),
                Err(e) => trouble = Some(e.context("re-add the host route")),
            },
            Err(e) => trouble = Some(e.context("no physical route to re-add the host route")),
        }
    }

    if !P::split_defaults_ok().await {
        match P::add_split_defaults().await {
            Ok(()) => log::warn("watchdog", "re-added the split-default routes"),
            Err(e) => trouble = Some(e.context("re-add the split-default routes")),
        }
    }

    match trouble {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Move the live tunnel to a different node without lowering it.
///
/// Only the host route changes: the device, its address, both halves of the
/// default route and the engine stay where they are, because the engine behind
/// the SOCKS port is what the caller swaps for the new node. The new route is
/// added BEFORE the old one goes, so there is no instant in which the engine's
/// traffic to a node has no way out but the tunnel itself — that instant is how
/// a location change turns into a connection storm. On failure nothing was
/// removed and the old node stays reachable.
pub async fn retarget<P: Privileged>(old: Option<Ipv4Addr>, plan: &TunPlan) -> Result<()> {
    if old == Some(plan.server_ip) {
        return Ok(());
    }
    let physical = P::physical_route().await?;
    P::add_host_route(plan.server_ip, &physical).await?;
    if let Some(old) = old {
        P::delete_host_route(old).await;
    }
    Ok(())
}

/// Tear the tunnel down, in the order pinned by `TEARDOWN_ORDER`. Idempotent:
/// safe to call when nothing is up, which is what the exit path does.
pub async fn down<P: Privileged>(server_ip: Option<Ipv4Addr>) -> Result<()> {
    for step in TEARDOWN_ORDER {
        match step {
            TeardownStep::SplitDefaults => P::delete_split_defaults().await,
            TeardownStep::HostRoute => {
                if let Some(ip) = server_ip {
                    P::delete_host_route(ip).await;
                }
            }
            TeardownStep::RestoreDns => P::restore_dns().await,
            TeardownStep::DeviceDown => P::device_down().await,
            TeardownStep::KillOwnedEngine => P::stop_engine().await,
            TeardownStep::KillStrayEngines => P::kill_stray(TUNNEL_ENGINE).await,
        }
    }
    Ok(())
}

/// The app owns the engine on these platforms, so its handle is the answer.
pub async fn engine_alive() -> bool {
    engine::alive().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Every call the sequence makes, in order. A plain `Mutex` and not a
    /// per-test fixture because `Privileged` is a static trait; the tests that
    /// use it therefore take `GUARD` and run one at a time.
    static CALLS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    /// Name of the step that should fail, to exercise one rollback path.
    static FAIL_AT: Mutex<Option<&'static str>> = Mutex::new(None);
    /// Who the fake says listens on the SOCKS port, when it is not us.
    static STRANGER: Mutex<Option<&'static str>> = Mutex::new(None);
    /// Serialises the tests that share the two statics above. A tokio mutex and
    /// not a std one: the guard is held across awaits for the whole scenario,
    /// which is exactly what `clippy::await_holding_lock` forbids for std.
    static GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn record(step: &'static str) -> Result<()> {
        CALLS.lock().expect("calls").push(step);
        let fail = *FAIL_AT.lock().expect("fail_at");
        match fail {
            Some(target) if target == step => Err(anyhow!("{} failed on purpose", step)),
            _ => Ok(()),
        }
    }

    async fn begin(fail_at: Option<&'static str>) -> tokio::sync::MutexGuard<'static, ()> {
        let guard = GUARD.lock().await;
        CALLS.lock().expect("calls").clear();
        *FAIL_AT.lock().expect("fail_at") = fail_at;
        *STRANGER.lock().expect("stranger") = None;
        guard
    }

    fn calls() -> Vec<&'static str> {
        CALLS.lock().expect("calls").clone()
    }

    struct Fake;

    impl Privileged for Fake {
        const DEVICE: &'static str = "faketun0";

        fn tun2socks_argv(_bin: &Path, _socks_port: u16) -> Argv {
            Argv::new("tun2socks", ["-device", "faketun0"])
        }

        async fn physical_route() -> Result<PhysicalRoute> {
            record("physical_route")?;
            Ok(PhysicalRoute {
                next_hop: Some("192.168.1.1".to_string()),
                if_name: Some("en0".to_string()),
                if_index: Some(4),
            })
        }

        async fn add_host_route(_dest: Ipv4Addr, _via: &PhysicalRoute) -> Result<()> {
            record("add_host_route")
        }

        async fn delete_host_route(_dest: Ipv4Addr) {
            let _ = record("delete_host_route");
        }

        async fn host_route_ok(_dest: Ipv4Addr) -> bool {
            let _ = record("host_route_ok");
            false
        }

        async fn add_split_defaults() -> Result<()> {
            record("add_split_defaults")
        }

        async fn delete_split_defaults() {
            let _ = record("delete_split_defaults");
        }

        async fn split_defaults_ok() -> bool {
            let _ = record("split_defaults_ok");
            false
        }

        async fn wait_for_device(_timeout: Duration) -> Result<()> {
            record("wait_for_device")
        }

        async fn configure_device() -> Result<()> {
            record("configure_device")
        }

        async fn device_down() {
            let _ = record("device_down");
        }

        async fn configure_dns(_servers: &[IpAddr]) -> Result<()> {
            record("configure_dns")
        }

        async fn restore_dns() {
            let _ = record("restore_dns");
        }

        async fn kill_stray(_stem: &str) {
            let _ = record("kill_stray");
        }

        // Stubbed so no process is ever spawned by a test.
        async fn start_engine(_bin: &Path, _socks_port: u16) -> Result<()> {
            record("start_engine")
        }

        async fn engine_exited() -> Option<String> {
            None
        }

        async fn stop_engine() {
            let _ = record("stop_engine");
        }

        // Not recorded: the step sequences pinned above stay as they were.
        async fn socks_port_stranger(_port: u16) -> Option<String> {
            STRANGER.lock().expect("stranger").map(str::to_string)
        }
    }

    fn plan() -> TunPlan {
        TunPlan {
            server_ip: Ipv4Addr::new(203, 0, 113, 7),
            socks_port: 10808,
            tun2socks: PathBuf::from("/opt/pvpn/tun2socks"),
            dns: vec!["1.1.1.1".parse().expect("ip")],
        }
    }

    /// The host route must exist before the engine starts, and the split
    /// defaults must not exist before the device is addressed. Getting this
    /// order wrong is how a connect blackholes its own control traffic.
    #[tokio::test]
    async fn up_runs_the_steps_in_the_only_safe_order() {
        let _g = begin(None).await;
        up::<Fake>(&plan()).await.expect("fake up succeeds");
        assert_eq!(
            calls(),
            vec![
                "physical_route",
                "add_host_route",
                "start_engine",
                "wait_for_device",
                "configure_device",
                "add_split_defaults",
                "configure_dns",
            ]
        );
    }

    /// Each failure must leave the machine as it was found. The engine and the
    /// host route are the two things that outlive a failed attempt if forgotten.
    #[tokio::test]
    async fn every_failure_in_up_rolls_back_what_it_already_did() {
        for step in [
            "start_engine",
            "wait_for_device",
            "configure_device",
            "add_split_defaults",
            "configure_dns",
        ] {
            let _g = begin(Some(step)).await;
            let err = up::<Fake>(&plan()).await.expect_err("must fail");
            assert!(
                err.to_string().contains(step),
                "{step}: wrong error {err:#}"
            );

            let log = calls();
            assert!(
                log.contains(&"delete_host_route"),
                "{step}: the host route was left behind: {log:?}"
            );
            if step != "start_engine" {
                assert!(
                    log.contains(&"stop_engine"),
                    "{step}: the engine was left running: {log:?}"
                );
            }
            if ["add_split_defaults", "configure_dns"].contains(&step) {
                assert!(
                    log.contains(&"delete_split_defaults"),
                    "{step}: routes were left in the table: {log:?}"
                );
            }
            if step == "configure_dns" {
                assert!(
                    log.contains(&"restore_dns"),
                    "{step}: the resolver was left pointing into a dead tunnel: {log:?}"
                );
            }
        }
    }

    /// A rollback must not re-add anything, and must not reorder: the routes go
    /// before the engine dies, or traffic has nowhere to go for an instant.
    #[tokio::test]
    async fn rollback_removes_routes_before_killing_the_engine() {
        let _g = begin(Some("configure_dns")).await;
        let _ = up::<Fake>(&plan()).await;
        let log = calls();
        let pos = |name: &str| log.iter().position(|c| *c == name).expect(name);
        assert!(pos("restore_dns") < pos("delete_split_defaults"));
        assert!(pos("delete_split_defaults") < pos("stop_engine"));
        assert!(pos("stop_engine") < pos("delete_host_route"));
    }

    /// The supervisor repairs both halves of the state, and reports a failure
    /// rather than swallowing it — a silent repair failure is how "connected but
    /// nothing flows" used to go unnoticed.
    #[tokio::test]
    async fn ensure_repairs_both_the_host_route_and_the_split_defaults() {
        let _g = begin(None).await;
        ensure::<Fake>(&plan()).await.expect("repair succeeds");
        let log = calls();
        assert!(log.contains(&"add_host_route"), "{log:?}");
        assert!(log.contains(&"add_split_defaults"), "{log:?}");
    }

    #[tokio::test]
    async fn ensure_reports_a_failed_repair() {
        let _g = begin(Some("add_split_defaults")).await;
        let err = ensure::<Fake>(&plan()).await.expect_err("must report");
        assert!(err.to_string().contains("split-default"), "{err:#}");
    }

    /// Teardown follows the published order, and runs every step even when the
    /// tunnel was never up — the exit path calls it unconditionally.
    #[tokio::test]
    async fn down_follows_the_published_teardown_order() {
        let _g = begin(None).await;
        down::<Fake>(Some(Ipv4Addr::new(203, 0, 113, 7)))
            .await
            .expect("teardown succeeds");
        assert_eq!(
            calls(),
            vec![
                "delete_split_defaults",
                "delete_host_route",
                "restore_dns",
                "device_down",
                "stop_engine",
                "kill_stray",
            ]
        );
    }

    /// The new host route goes in before the old one goes out, and nothing
    /// else about the tunnel is touched.
    #[tokio::test]
    async fn retarget_adds_the_new_route_before_removing_the_old_one() {
        let _g = begin(None).await;
        retarget::<Fake>(Some(Ipv4Addr::new(198, 51, 100, 9)), &plan())
            .await
            .expect("retarget succeeds");
        assert_eq!(
            calls(),
            vec!["physical_route", "add_host_route", "delete_host_route"]
        );
    }

    /// A failed add must leave the old route alone: the engine still talks to
    /// the old node until the caller decides what to do.
    #[tokio::test]
    async fn a_failed_retarget_keeps_the_old_route() {
        let _g = begin(Some("add_host_route")).await;
        let err = retarget::<Fake>(Some(Ipv4Addr::new(198, 51, 100, 9)), &plan())
            .await
            .expect_err("must fail");
        assert!(err.to_string().contains("add_host_route"), "{err:#}");
        assert!(!calls().contains(&"delete_host_route"), "{:?}", calls());
    }

    #[tokio::test]
    async fn retarget_to_the_same_node_does_nothing() {
        let _g = begin(None).await;
        retarget::<Fake>(Some(plan().server_ip), &plan())
            .await
            .expect("no-op");
        assert!(calls().is_empty(), "{:?}", calls());
    }

    /// A stranger on the SOCKS port before the split defaults go in: the
    /// machine is never pointed at it, and what was raised is undone.
    #[tokio::test]
    async fn up_refuses_a_socks_port_held_by_a_stranger() {
        let _g = begin(None).await;
        *STRANGER.lock().expect("stranger") = Some("pid 4242");
        let err = up::<Fake>(&plan()).await.expect_err("must refuse");
        assert!(format!("{err:#}").contains("pid 4242"), "{err:#}");
        let log = calls();
        assert!(!log.contains(&"add_split_defaults"), "{log:?}");
        assert!(log.contains(&"stop_engine"), "{log:?}");
        assert!(log.contains(&"delete_host_route"), "{log:?}");
    }

    /// A stranger that took the port while xray restarted: the supervisor
    /// lowers the whole tunnel instead of re-asserting routes into it.
    #[tokio::test]
    async fn ensure_lowers_the_tunnel_when_a_stranger_holds_the_socks_port() {
        let _g = begin(None).await;
        *STRANGER.lock().expect("stranger") = Some("pid 4242");
        let err = ensure::<Fake>(&plan()).await.expect_err("must report");
        assert!(format!("{err:#}").contains("pid 4242"), "{err:#}");
        assert_eq!(
            calls(),
            vec![
                "delete_split_defaults",
                "delete_host_route",
                "restore_dns",
                "device_down",
                "stop_engine",
                "kill_stray",
            ],
            "the published teardown, and no repair"
        );
    }

    #[tokio::test]
    async fn down_without_a_known_node_still_tears_everything_else_down() {
        let _g = begin(None).await;
        down::<Fake>(None).await.expect("teardown succeeds");
        let log = calls();
        assert!(!log.contains(&"delete_host_route"), "nothing to delete");
        assert!(log.contains(&"stop_engine"), "{log:?}");
        assert!(log.contains(&"kill_stray"), "{log:?}");
    }
}
