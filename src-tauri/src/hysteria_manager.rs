// src-tauri/src/hysteria_manager.rs
//
// The hysteria2 client process. It is no longer the front of the chain: since
// the unified fork layer (see xray_manager.rs) tun2socks always talks to xray,
// and xray reaches a Hysteria2 node through this process's SOCKS listener.
//
//   tun2socks -> xray:10808 --socks--> hysteria:10809 -> node
//
// Two consequences worth naming. Split routing now applies on Hysteria2
// locations, where it simply did not exist before. And this process is a
// dependency of the tunnel rather than the tunnel itself, so the supervisor in
// lib.rs watches it exactly like it watches the other two, and stopping the
// VPN stops it — a hysteria that outlived the app kept port 10809 and was the
// whole of "it will not connect a second time".

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use pvpn_platform::{paths, process as pprocess};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::errors::{AppError, ErrorCode};
use crate::pidfile::Engine;
use crate::subscription::Hy2Config;

/// SOCKS port for hysteria — distinct from xray's 10808 because both now run
/// at the same time, which is the point of the unified layer.
pub const HY2_SOCKS_PORT: u16 = 10809;

// Where the client config goes: `pvpn_platform::paths::hy2_config_file`.
// On macOS that is still the fixed `/tmp/proxysvpn-hy2.yaml` — under `sudo`
// and under `launchctl asuser` the process has two different `TMPDIR`s, and a
// config written to one of them would be invisible to a later run trying to
// clean it up. Windows and Linux keep it in the per-user state directory.

/// How long the SOCKS listener gets to appear before we call the start a
/// failure. Measured, not guessed: on this machine the listener is up in
/// 150-250 ms, and the old blind 600 ms sleep reported success even when
/// hysteria had already exited on a bad config.
const LISTEN_TIMEOUT: Duration = Duration::from_millis(3000);
const LISTEN_POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
pub struct HysteriaState {
    child: Option<Engine>,
}

pub type SharedHysteriaState = Arc<Mutex<HysteriaState>>;

pub fn new_state() -> SharedHysteriaState {
    Arc::new(Mutex::new(HysteriaState::default()))
}

/// The hysteria sidecar, found by the same ordered directory list as every
/// other sidecar (`crate::sidecars`) and named by the platform layer's one
/// rule (`hysteria`, `hysteria-<triple>`, `.exe` on Windows).
pub fn hysteria_path(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    crate::sidecars::find(app, "hysteria", "hysteria")
}

/// Build the hysteria client YAML config.
///
/// Kept as a pure function of the node so the shape can be tested without a
/// binary, a network or a filesystem.
pub fn build_config(cfg: &Hy2Config) -> String {
    let mut yaml = String::new();
    yaml.push_str(&format!("server: {}\n", yaml_scalar(&format!("{}:{}", cfg.host, cfg.port))));
    yaml.push_str(&format!("auth: {}\n", yaml_scalar(&cfg.password)));
    yaml.push_str("tls:\n");
    yaml.push_str(&format!("  sni: {}\n", yaml_scalar(&cfg.sni)));
    if cfg.insecure {
        yaml.push_str("  insecure: true\n");
    }
    if !cfg.pin_sha256.is_empty() {
        yaml.push_str(&format!("  pinSHA256: {}\n", yaml_scalar(&cfg.pin_sha256)));
    }
    yaml.push_str("socks5:\n");
    yaml.push_str(&format!("  listen: 127.0.0.1:{}\n", HY2_SOCKS_PORT));
    yaml.push_str("fastOpen: true\n");
    yaml
}

/// One value of the hand-built YAML.
///
/// Every value comes from a subscription line, and `query_pairs` decodes
/// percent-escapes: a `pinSHA256=%0A...` used to put a newline, and with it any
/// top-level key the line liked, into the config of a process that runs as
/// root (macOS) or administrator (Windows). Ordinary values — host:port, hex,
/// a UUID password — are written bare exactly as before; anything else is
/// written as a JSON string, which YAML reads as a double-quoted scalar, so it
/// can never end the line or start a key.
fn yaml_scalar(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']'))
        && value.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'[');
    if plain {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
    }
}

/// Write the config owner-only and never through a symlink.
///
/// It holds the node's auth password and its address, and on macOS the path
/// is a fixed name in mode-1777 `/tmp` written by root: a plain `fs::write`
/// would follow a link any local user planted there and hand them a root-owned
/// write to a file of their choice, and the old write-then-chmod left the
/// password world-readable for the moment in between. See
/// `pvpn_platform::paths::write_private_file`.
fn write_config(yaml: &str) -> Result<PathBuf, AppError> {
    let path = paths::hy2_config_file().map_err(|e| {
        crate::logger::log("error", "hysteria", &format!("no config location: {e:#}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;
    paths::write_private_file(&path, yaml.as_bytes()).map_err(|e| {
        crate::logger::log("error", "hysteria", &format!("config write failed: {e:#}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;
    Ok(path)
}

/// The config is the node password at rest; nothing reads it once hysteria
/// stopped. On Windows and Linux it lives in the user's profile, where it
/// would otherwise sit until the next connect overwrote it.
fn remove_config() {
    let Ok(path) = paths::hy2_config_file() else {
        return;
    };
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            crate::logger::log("warn", "hysteria", &format!("config not removed: {e}"));
        }
    }
}

pub async fn start(
    state: &SharedHysteriaState,
    app: &tauri::AppHandle,
    cfg: &Hy2Config,
) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    if guard.child.is_some() {
        crate::logger::log("error", "hysteria", "start called while already running");
        return Err(AppError::new(ErrorCode::EngineStartFailed));
    }

    // Asked before spawning, because hysteria answers a taken port by exiting
    // with a message nobody reads, and "port busy" is the one start failure
    // where pressing the button again genuinely helps once the holder is gone.
    if crate::xray_manager::port_in_use(HY2_SOCKS_PORT).await {
        crate::logger::log("error", "hysteria", "socks port is already held");
        return Err(AppError::new(ErrorCode::PortBusy));
    }

    let bin = hysteria_path(app)?;
    let cfg_path = write_config(&build_config(cfg))?;
    let cfg_arg = cfg_path.to_str().ok_or_else(|| {
        crate::logger::log("error", "hysteria", "config path is not valid utf-8");
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    // The node's address is deliberately not written here. `logger::redact`
    // would mask a numeric one anyway, but a host NAME is only masked once
    // `remember_node_host` has been told about it, and a log line is not the
    // place to find out whether that happened.
    crate::logger::log("info", "hysteria", "starting client");

    let mut cmd = Command::new(&bin);
    cmd.args(["client", "-c", cfg_arg])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Without this a console window pops up on Windows for every engine start.
    pprocess::no_window(&mut cmd);

    let mut child = Engine::spawn("hysteria", &mut cmd, &bin).map_err(|e| {
        crate::logger::log("error", "hysteria", &format!("spawn failed: {e}"));
        AppError::new(ErrorCode::EngineStartFailed)
    })?;

    // "info" as the DEFAULT level, not as a verdict: hysteria colours its own
    // level word and logger::log honours it. Labelling this stream `warn` is
    // what used to turn the log window into an orange wall.
    pump(child.child().stdout.take());
    pump(child.child().stderr.take());

    // Success means "the port answers", not "the process was spawned". A bad
    // password or a burnt address makes hysteria exit within a moment, and the
    // old code reported that as a working tunnel.
    match await_listener(child.child()).await {
        Ok(()) => {
            guard.child = Some(child);
            Ok(())
        }
        Err(err) => {
            child.stop().await;
            Err(err)
        }
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
            crate::logger::log("info", "hysteria", &line);
        }
    });
}

/// Wait until the SOCKS listener accepts a connection, or the process dies.
async fn await_listener(child: &mut Child) -> Result<(), AppError> {
    let deadline = tokio::time::Instant::now() + LISTEN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                crate::logger::log("error", "hysteria", &format!("exited at once: {status}"));
                return Err(AppError::new(ErrorCode::EngineStartFailed));
            }
            Ok(None) => {}
            Err(e) => {
                crate::logger::log("error", "hysteria", &format!("try_wait failed: {e}"));
                return Err(AppError::new(ErrorCode::EngineStartFailed));
            }
        }

        if crate::xray_manager::port_in_use(HY2_SOCKS_PORT).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            crate::logger::log("error", "hysteria", "socks listener did not appear in time");
            return Err(AppError::new(ErrorCode::EngineStartFailed));
        }
        tokio::time::sleep(LISTEN_POLL).await;
    }
}

pub async fn stop(state: &SharedHysteriaState) -> Result<(), AppError> {
    let mut guard = state.lock().await;
    if let Some(child) = guard.child.take() {
        child.stop().await;
    }
    // A hysteria we lost the handle to holds 10809 and makes the next connect
    // fail for a reason nobody can see, so the sweep is unconditional — over
    // the hysterias WE started, not every process of that name on the Mac.
    crate::pidfile::sweep("hysteria").await;
    remove_config();
    Ok(())
}

/// Is OUR hysteria alive? Asked of the child handle, so a stray copy on the
/// machine cannot keep the screen green over a dead tunnel.
pub async fn is_running(state: &SharedHysteriaState) -> bool {
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

    fn cfg() -> Hy2Config {
        Hy2Config {
            password: "secret".into(),
            host: "node.example".into(),
            port: 443,
            sni: "sni.example".into(),
            pin_sha256: String::new(),
            insecure: false,
            remark: "Нидерланды".into(),
        }
    }

    #[test]
    fn config_listens_where_xray_will_dial() {
        let yaml = build_config(&cfg());
        assert!(yaml.contains(&format!("listen: 127.0.0.1:{HY2_SOCKS_PORT}")));
        assert!(yaml.contains("server: node.example:443"));
    }

    #[test]
    fn optional_fields_stay_out_when_empty() {
        // An empty pinSHA256 written as a key makes hysteria refuse the file.
        let yaml = build_config(&cfg());
        assert!(!yaml.contains("pinSHA256"));
        assert!(!yaml.contains("insecure"));
    }

    #[test]
    fn pinned_and_insecure_nodes_are_written_out() {
        let mut c = cfg();
        c.insecure = true;
        c.pin_sha256 = "AA:BB".into();
        let yaml = build_config(&c);
        assert!(yaml.contains("insecure: true"));
        assert!(yaml.contains("pinSHA256: AA:BB"));
    }

    /// A percent-encoded newline in a subscription value must not become a
    /// new key in the config of a root/administrator process.
    #[test]
    fn a_value_cannot_inject_yaml_keys() {
        let mut c = cfg();
        c.pin_sha256 = "AA\nsocks5:\n  listen: 0.0.0.0:1080".into();
        c.password = "p\r\nhttp:\n  listen: 0.0.0.0:8080".into();
        let yaml = build_config(&c);
        let keys: Vec<&str> = yaml
            .lines()
            .filter(|line| !line.starts_with(' '))
            .map(|line| line.split(':').next().unwrap_or(""))
            .collect();
        assert_eq!(keys, ["server", "auth", "tls", "socks5", "fastOpen"], "{yaml}");
        assert_eq!(yaml.lines().count(), 8, "one line per key, nothing smuggled in: {yaml}");
        assert!(yaml.contains(r#"auth: "p\r\nhttp:\n  listen: 0.0.0.0:8080""#), "{yaml}");
    }

    #[test]
    fn ordinary_values_are_written_bare_as_before() {
        assert_eq!(yaml_scalar("node.example:443"), "node.example:443");
        assert_eq!(yaml_scalar("[2001:db8::1]:443"), "[2001:db8::1]:443");
        assert_eq!(yaml_scalar("11111111-2222-3333-4444-555555555555"), "11111111-2222-3333-4444-555555555555");
        assert_eq!(yaml_scalar("AA:BB:CC"), "AA:BB:CC");
        assert_eq!(yaml_scalar("a b"), "\"a b\"");
        assert_eq!(yaml_scalar("#comment"), "\"#comment\"");
        assert_eq!(yaml_scalar(""), "\"\"");
    }

    #[test]
    fn the_two_engines_never_share_a_port() {
        assert_ne!(HY2_SOCKS_PORT, crate::xray_manager::FRONT_SOCKS_PORT);
    }

    /// The YAML is hand-built, so its exact shape is worth pinning: hysteria
    /// silently ignores keys it does not understand, which turns a typo into a
    /// connection that "works" without TLS pinning.
    #[test]
    fn config_yaml_is_stable() {
        let mut c = cfg();
        c.insecure = true;
        c.pin_sha256 = "AA:BB:CC".into();
        assert_eq!(
            build_config(&c),
            concat!(
                "server: node.example:443\n",
                "auth: secret\n",
                "tls:\n",
                "  sni: sni.example\n",
                "  insecure: true\n",
                "  pinSHA256: AA:BB:CC\n",
                "socks5:\n",
                "  listen: 127.0.0.1:10809\n",
                "fastOpen: true\n",
            )
        );
    }

    /// The config carries the node password, so it must never land somewhere a
    /// different local user could read or replace it.
    ///
    /// The path differs per platform, and deliberately so: macOS keeps the
    /// fixed literal in /tmp (TMPDIR changes under sudo) and relies on
    /// `write_private_file`, while Windows and Linux put it in a per-user
    /// directory, which is strictly better. So this asserts the invariant that
    /// actually matters rather than one hard-coded string.
    #[test]
    fn config_path_is_app_private() {
        let path = paths::hy2_config_file().expect("config path");
        let text = path.to_string_lossy().to_ascii_lowercase();

        #[cfg(target_os = "macos")]
        assert_eq!(path, PathBuf::from("/tmp/proxysvpn-hy2.yaml"));

        #[cfg(target_os = "windows")]
        {
            assert!(text.contains("proxysvpn"), "{}", text);
            assert!(!text.starts_with(r"c:\windows\temp"), "{}", text);
        }

        // Linux must not fall back to a world-writable directory: a
        // predictable name in /tmp invites a symlink swap by another user.
        #[cfg(target_os = "linux")]
        {
            assert!(text.contains("proxysvpn"), "{}", text);
            assert!(!text.starts_with("/tmp/"), "{}", text);
        }

        assert!(path.is_absolute(), "{}", text);
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("yaml"));
    }
}
