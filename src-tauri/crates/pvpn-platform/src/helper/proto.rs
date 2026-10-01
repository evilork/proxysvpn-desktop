// src-tauri/crates/pvpn-platform/src/helper/proto.rs
//! Wire protocol between the unprivileged GUI and the root helper on Linux:
//! one JSON object per line, GUI → helper on stdin, helper → GUI on stdout.
//!
//! The helper's stdin is a privilege boundary, so everything that arrives on it
//! is validated here before it reaches a command line. This module is compiled
//! on every host (not just Linux) so that its unit tests run in the macOS
//! build as well — this Mac is the only machine available for the Linux port.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
// Reason for the allow: on macOS/Windows nothing calls into the Linux helper,
// but we still want the parsing, validation and trust checks exercised by
// `cargo test` on the developer's Mac — that is the only machine available.

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

/// One line on the pipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Frame {
    /// GUI → helper.
    Request { id: u64, req: Request },
    /// helper → GUI, answering `id`.
    Response {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default)]
        engine_alive: bool,
    },
    /// helper → GUI, unsolicited; mirrored into the app log.
    Log {
        level: String,
        source: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Handshake: proves the helper is alive and authorized.
    Hello,
    /// Bring the tunnel up. Idempotent: a second `Up` tears the old one down.
    Up(UpParams),
    /// Re-assert routes and DNS (the 5 s supervisor tick).
    Ensure(UpParams),
    /// Tear the tunnel down but keep the helper running, so that the next
    /// connect does not ask for the password again.
    Down,
    /// Is tun2socks still alive?
    Status,
}

/// Everything the helper needs to raise the tunnel. Deliberately *not*
/// including the interface name, address or routes: those are helper-side
/// constants, so a compromised GUI cannot aim the helper at an arbitrary
/// interface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpParams {
    /// Already-resolved IPv4 address of the VPN node (never logged).
    pub server_ip: String,
    /// SOCKS5 port the engine (xray or hysteria) listens on, on loopback.
    pub socks_port: u16,
    /// Absolute path to the tun2socks sidecar.
    pub tun2socks: String,
    /// Resolvers to publish on the tunnel interface.
    pub dns: Vec<String>,
}

/// `UpParams` after validation — the only shape the helper acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidUp {
    pub server_ip: Ipv4Addr,
    pub socks_port: u16,
    pub tun2socks: PathBuf,
    pub dns: Vec<IpAddr>,
}

/// Validate a request that crossed the privilege boundary.
///
/// Rejects anything that could turn into an argument injection or a route to
/// somewhere we did not mean: non-IPv4 node addresses, port 0, relative paths,
/// empty or malformed resolver lists.
pub fn validate_up(params: &UpParams) -> Result<ValidUp, String> {
    let server_ip: Ipv4Addr = params
        .server_ip
        .parse()
        .map_err(|_| format!("server_ip is not an IPv4 address: {:?}", params.server_ip))?;
    if server_ip.is_loopback() || server_ip.is_unspecified() || server_ip.is_multicast() {
        return Err("server_ip must be a routable unicast address".to_string());
    }
    if params.socks_port == 0 {
        return Err("socks_port must be non-zero".to_string());
    }

    // A leading slash, checked by hand rather than with `Path::is_absolute()`.
    // This is a wire protocol whose receiver is always the Linux helper, so
    // "absolute" must mean POSIX-absolute no matter which OS compiled the code;
    // `is_absolute()` asks the *host* and answers false for "/usr/bin/tun2socks"
    // on Windows, which made this validation — and its test — disagree with
    // itself across platforms.
    if !params.tun2socks.starts_with('/') {
        return Err(format!(
            "tun2socks path must be absolute: {:?}",
            params.tun2socks
        ));
    }
    // No `..`: the path is handed to root for exec, and the trust check that
    // follows stats the file, so a traversal must not slip past it.
    let tun2socks = PathBuf::from(&params.tun2socks);
    if tun2socks.components().any(|c| c.as_os_str() == "..") {
        return Err(format!(
            "tun2socks path must not contain '..': {:?}",
            params.tun2socks
        ));
    }

    if params.dns.is_empty() {
        return Err("dns list must not be empty".to_string());
    }
    let mut dns = Vec::with_capacity(params.dns.len());
    for entry in &params.dns {
        let ip: IpAddr = entry
            .parse()
            .map_err(|_| format!("dns entry is not an IP address: {:?}", entry))?;
        if ip.is_loopback() || ip.is_unspecified() {
            return Err(format!("dns entry is not usable inside the tunnel: {}", ip));
        }
        dns.push(ip);
    }

    Ok(ValidUp {
        server_ip,
        socks_port: params.socks_port,
        tun2socks,
        dns,
    })
}

/// Ownership and permission facts about a file, extracted so the trust rule
/// below can be tested without creating root-owned files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    pub uid: u32,
    /// st_mode as reported by stat(2).
    pub mode: u32,
    pub is_regular: bool,
}

/// May the root helper exec this file?
///
/// The helper runs as uid 0 and is told which binary to start by an
/// unprivileged peer, so the invariant we need is: *the sidecar must be no
/// easier to tamper with than the helper executable itself*. Anyone who can
/// rewrite the helper binary already owns the root it is about to gain, so:
///
///   * never group- or world-writable, and never a non-regular file;
///   * root-owned is always fine — the `.deb` case, `/usr/bin`, root:root 0755;
///   * a file sitting in the same directory as the helper binary is fine too —
///     the AppImage case, where the whole read-only mount belongs to whoever
///     built the image rather than to root;
///   * otherwise only in dev mode (`PROXYSVPN_HELPER_DEV=1`, never set in a
///     bundle), and only when the file belongs to the user who authorized us.
pub fn sidecar_is_trusted(
    facts: &FileFacts,
    invoking_uid: Option<u32>,
    dev_mode: bool,
    beside_helper: bool,
) -> Result<(), String> {
    if !facts.is_regular {
        return Err("sidecar is not a regular file".to_string());
    }
    if facts.mode & 0o022 != 0 {
        return Err(format!(
            "sidecar is group/world writable (mode {:o})",
            facts.mode & 0o7777
        ));
    }
    if facts.uid == 0 || beside_helper {
        return Ok(());
    }
    match (dev_mode, invoking_uid) {
        (true, Some(uid)) if uid == facts.uid => Ok(()),
        _ => Err(format!(
            "sidecar is owned by uid {} and does not sit next to the helper",
            facts.uid
        )),
    }
}

/// uid of the user who authorized us, as exported by pkexec.
pub fn invoking_uid_from_env() -> Option<u32> {
    std::env::var("PKEXEC_UID").ok()?.parse().ok()
}

/// Serialize a frame as a single line, newline included.
pub fn encode(frame: &Frame) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(frame)?;
    line.push('\n');
    Ok(line)
}

/// Parse one line from the pipe.
pub fn decode(line: &str) -> Result<Frame, serde_json::Error> {
    serde_json::from_str(line.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UpParams {
        UpParams {
            server_ip: "203.0.113.7".to_string(),
            socks_port: 10808,
            tun2socks: "/usr/bin/tun2socks".to_string(),
            dns: vec!["1.1.1.1".to_string(), "1.0.0.1".to_string()],
        }
    }

    #[test]
    fn frames_round_trip_through_json() {
        let frames = vec![
            Frame::Request { id: 1, req: Request::Hello },
            Frame::Request { id: 2, req: Request::Up(sample()) },
            Frame::Request { id: 3, req: Request::Ensure(sample()) },
            Frame::Request { id: 4, req: Request::Down },
            Frame::Request { id: 5, req: Request::Status },
            Frame::Response { id: 5, ok: true, error: None, engine_alive: true },
            Frame::Response {
                id: 6,
                ok: false,
                error: Some("boom".to_string()),
                engine_alive: false,
            },
            Frame::Log {
                level: "warn".to_string(),
                source: "helper".to_string(),
                message: "hi".to_string(),
            },
        ];
        for frame in frames {
            let line = encode(&frame).expect("encode");
            assert!(line.ends_with('\n'), "frames must be newline delimited");
            assert_eq!(line.matches('\n').count(), 1, "no embedded newlines");
            assert_eq!(decode(&line).expect("decode"), frame);
        }
    }

    #[test]
    fn garbage_lines_are_rejected_not_panicking() {
        assert!(decode("").is_err());
        assert!(decode("{").is_err());
        assert!(decode("{\"nope\":1}").is_err());
        assert!(decode("[1,2,3]").is_err());
    }

    #[test]
    fn valid_params_pass() {
        let ok = validate_up(&sample()).expect("sample must validate");
        assert_eq!(ok.server_ip, Ipv4Addr::new(203, 0, 113, 7));
        assert_eq!(ok.socks_port, 10808);
        assert_eq!(ok.dns.len(), 2);
    }

    #[test]
    fn rejects_bad_server_ip() {
        for bad in ["", "not-an-ip", "::1", "2001:db8::1", "127.0.0.1", "0.0.0.0", "224.0.0.1"] {
            let mut p = sample();
            p.server_ip = bad.to_string();
            assert!(validate_up(&p).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn rejects_zero_port_and_relative_path() {
        let mut p = sample();
        p.socks_port = 0;
        assert!(validate_up(&p).is_err());

        for bad in [
            "tun2socks",
            "./tun2socks",
            "../../usr/bin/tun2socks",
            "/usr/bin/../../tmp/evil",
            "",
            r"C:\tun2socks.exe",
        ] {
            let mut p = sample();
            p.tun2socks = bad.to_string();
            assert!(validate_up(&p).is_err(), "{bad:?} must be rejected");
        }
    }

    /// The verdict must not depend on which OS compiled this code: the helper
    /// that acts on it always runs on Linux. `Path::is_absolute()` would answer
    /// false for a POSIX path on a Windows host and break this invariant.
    #[test]
    fn absolute_means_posix_absolute_on_every_host() {
        let ok = validate_up(&sample()).expect("a POSIX path validates everywhere");
        assert_eq!(
            ok.tun2socks.to_string_lossy(),
            "/usr/bin/tun2socks",
            "the path must survive validation unchanged"
        );
    }

    #[test]
    fn rejects_bad_dns_lists() {
        let mut p = sample();
        p.dns = vec![];
        assert!(validate_up(&p).is_err());

        let mut p = sample();
        p.dns = vec!["1.1.1.1".to_string(), "nope".to_string()];
        assert!(validate_up(&p).is_err());

        let mut p = sample();
        p.dns = vec!["127.0.0.53".to_string()];
        assert!(validate_up(&p).is_err(), "a loopback resolver would not reach the tunnel");
    }

    #[test]
    fn root_owned_sidecar_is_trusted() {
        let facts = FileFacts { uid: 0, mode: 0o100755, is_regular: true };
        assert!(sidecar_is_trusted(&facts, Some(1000), false, false).is_ok());
    }

    #[test]
    fn world_writable_sidecar_is_refused_even_for_root_and_even_beside_us() {
        let world = FileFacts { uid: 0, mode: 0o100777, is_regular: true };
        assert!(sidecar_is_trusted(&world, Some(1000), false, false).is_err());
        assert!(sidecar_is_trusted(&world, Some(1000), true, true).is_err());
        let group = FileFacts { uid: 0, mode: 0o100775, is_regular: true };
        assert!(sidecar_is_trusted(&group, Some(1000), false, false).is_err());
    }

    #[test]
    fn appimage_case_sidecar_beside_the_helper_is_trusted() {
        // Inside an AppImage every file belongs to whoever built the image.
        let facts = FileFacts { uid: 1001, mode: 0o100755, is_regular: true };
        assert!(sidecar_is_trusted(&facts, Some(1000), false, true).is_ok());
    }

    #[test]
    fn user_owned_sidecar_elsewhere_needs_dev_mode_and_matching_uid() {
        let facts = FileFacts { uid: 1000, mode: 0o100755, is_regular: true };
        assert!(sidecar_is_trusted(&facts, Some(1000), false, false).is_err());
        assert!(sidecar_is_trusted(&facts, Some(1000), true, false).is_ok());
        assert!(sidecar_is_trusted(&facts, Some(1001), true, false).is_err());
        assert!(sidecar_is_trusted(&facts, None, true, false).is_err());
    }

    #[test]
    fn directories_are_refused() {
        let facts = FileFacts { uid: 0, mode: 0o040755, is_regular: false };
        assert!(sidecar_is_trusted(&facts, None, false, true).is_err());
    }
}
