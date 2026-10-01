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
use std::path::{Path, PathBuf};

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
    /// Move the live tunnel to another node: pin the new node's address to the
    /// physical link, then drop the old one. The device, the split defaults,
    /// DNS and tun2socks stay as they are. Only valid while a tunnel is up, and
    /// it can aim nothing `Up` could not: the same validation applies.
    Retarget(UpParams),
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
///
/// Nor the tun2socks binary. It used to travel here as a path, and the helper
/// checked that path and then exec'd it by name again later. Only the last
/// component was protected against symlinks, so a peer could pass a path
/// through a directory it controlled, let the check see /usr/bin and then
/// retarget the directory before the exec: root ran a file of its choosing,
/// with no password inside polkit's `auth_admin_keep` window. The helper now
/// finds the sidecar next to its own executable ([`sidecar_dirs`]), and
/// `deny_unknown_fields` refuses a request that still tries to name one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UpParams {
    /// Already-resolved IPv4 address of the VPN node (never logged).
    pub server_ip: String,
    /// SOCKS5 port the engine (xray or hysteria) listens on, on loopback.
    pub socks_port: u16,
    /// Resolvers to publish on the tunnel interface.
    pub dns: Vec<String>,
}

/// Stem of the sidecar the helper runs. Resolved with
/// `triple::find_sidecar`, so `tun2socks` in a bundle and
/// `tun2socks-<triple>` in a dev checkout.
pub const SIDECAR_NAME: &str = "tun2socks";

/// Directories the helper looks in for tun2socks, in order: the directory of
/// its own executable (on the .deb that is root-owned `/usr/bin`, inside an
/// AppImage the read-only mount), and only in a debug `--dev` run the repo's
/// `binaries` directory. Nothing in this list comes from the peer.
pub fn sidecar_dirs(helper_exe: &Path, dev_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    if let Some(own) = helper_exe.parent() {
        dirs.push(own.to_path_buf());
    }
    if let Some(dev) = dev_dir {
        dirs.push(dev.to_path_buf());
    }
    dirs
}

/// The only SOCKS ports a shipped build ever asks for: xray's inbound
/// (`tun::SOCKS_PORT`) and hysteria's (`hysteria_manager::HY2_SOCKS_PORT`).
///
/// This is a privilege boundary, not a convenience: the port decides where
/// every packet on the machine goes once the half-defaults are installed, and
/// the helper must not aim them at a listener it was simply told about. The GUI
/// crate pins its own two constants against this list in a test.
pub const ALLOWED_SOCKS_PORTS: [u16; 2] = [10808, 10809];

/// `UpParams` after validation — the only shape the helper acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidUp {
    pub server_ip: Ipv4Addr,
    pub socks_port: u16,
    pub dns: Vec<IpAddr>,
}

/// Validate a request that crossed the privilege boundary.
///
/// Rejects anything that could turn into an argument injection or a route to
/// somewhere we did not mean: non-IPv4 node addresses, ports that are not our
/// engines', empty or malformed resolver lists.
pub fn validate_up(params: &UpParams) -> Result<ValidUp, String> {
    let server_ip: Ipv4Addr = params
        .server_ip
        .parse()
        .map_err(|_| format!("server_ip is not an IPv4 address: {:?}", params.server_ip))?;
    if server_ip.is_loopback() || server_ip.is_unspecified() || server_ip.is_multicast() {
        return Err("server_ip must be a routable unicast address".to_string());
    }
    if !ALLOWED_SOCKS_PORTS.contains(&params.socks_port) {
        return Err(format!(
            "socks_port {} is not one of this app's engine ports {:?}",
            params.socks_port, ALLOWED_SOCKS_PORTS
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
/// The helper runs as uid 0 and finds the binary itself ([`sidecar_dirs`]),
/// but the invariant still holds as a second line: *the sidecar must be no
/// easier to tamper with than the helper executable itself*. Anyone who can
/// rewrite the helper binary already owns the root it is about to gain, so:
///
///   * never group- or world-writable, and never a non-regular file;
///   * root-owned is always fine — the `.deb` case, `/usr/bin`, root:root 0755;
///   * a file sitting in the same directory as the helper binary is fine too —
///     the AppImage case, where the whole read-only mount belongs to whoever
///     built the image rather than to root;
///   * otherwise only in dev mode, and only when the file belongs to the user
///     who authorized us. See [`dev_mode_allowed`] for why dev mode cannot be
///     reached in a shipped build.
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

/// May the sidecar ownership rule be relaxed for `cargo run`?
///
/// Only in a debug build, and that is the whole point. The flag that asks for
/// it is an argument of the *helper*, and the helper's argv is chosen by an
/// unprivileged peer: the GUI only passes `--dev` when `PROXYSVPN_HELPER_DEV=1`
/// is set in its own environment, but nothing stops another local process from
/// running `pkexec /usr/bin/proxysvpn-desktop --helper --dev` itself. Inside
/// polkit's `auth_admin_keep` window that needs no password, and the relaxed
/// rule would then let that process have root exec a binary it owns.
///
/// Gating on the build kind removes the branch from every shipped artifact
/// instead of relying on an environment variable a bundle "never" has.
pub fn dev_mode_allowed(flag_present: bool, debug_build: bool) -> bool {
    flag_present && debug_build
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

/// Longest request the helper will assemble. The biggest legitimate frame is an
/// `Up` with a path and a handful of resolvers — comfortably under a kilobyte —
/// so this only bounds how much memory an unprivileged peer can make a root
/// process allocate by sending one endless line.
pub const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// One read from the request pipe.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    Request(String),
    /// Over the limit. The rest of it was discarded, so the *next* frame still
    /// parses instead of being read as the tail of this one.
    TooLong,
    Eof,
}

/// `BufRead::read_line` with a ceiling, and without losing framing when the
/// ceiling is hit.
pub fn read_bounded_line<R: std::io::BufRead>(reader: &mut R) -> std::io::Result<Line> {
    use std::io::{BufRead, Read};

    let mut buf = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_REQUEST_BYTES)
        .read_until(b'\n', &mut buf)?;
    if read == 0 {
        return Ok(Line::Eof);
    }
    if !buf.ends_with(b"\n") {
        // Either the limit was hit mid-line, or the peer closed the pipe
        // without a final newline. Discarding to the next newline is right in
        // both cases; at EOF it reads nothing and returns at once.
        skip_to_newline(reader)?;
        return Ok(Line::TooLong);
    }
    Ok(Line::Request(String::from_utf8_lossy(&buf).into_owned()))
}

fn skip_to_newline<R: std::io::BufRead>(reader: &mut R) -> std::io::Result<()> {
    use std::io::{BufRead, Read};

    loop {
        let mut sink = Vec::new();
        let n = reader
            .by_ref()
            .take(MAX_REQUEST_BYTES)
            .read_until(b'\n', &mut sink)?;
        if n == 0 || sink.ends_with(b"\n") {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UpParams {
        UpParams {
            server_ip: "203.0.113.7".to_string(),
            socks_port: 10808,
            dns: vec!["1.1.1.1".to_string(), "1.0.0.1".to_string()],
        }
    }

    /// The peer must not be able to name the binary root runs: a path it
    /// supplies can be checked through one directory and exec'd through
    /// another. A request that still carries one is refused outright.
    #[test]
    fn an_up_request_that_names_a_binary_is_refused() {
        let line = r#"{"request":{"id":1,"req":{"up":{"server_ip":"203.0.113.7","socks_port":10808,"tun2socks":"/home/u/d/tun2socks","dns":["1.1.1.1"]}}}}"#;
        assert!(decode(line).is_err(), "a tun2socks path from the peer must not parse");
        let clean = r#"{"request":{"id":1,"req":{"up":{"server_ip":"203.0.113.7","socks_port":10808,"dns":["1.1.1.1"]}}}}"#;
        assert!(decode(clean).is_ok());
    }

    #[test]
    fn the_helper_looks_for_tun2socks_only_beside_itself_outside_dev() {
        assert_eq!(
            sidecar_dirs(Path::new("/usr/bin/proxysvpn-desktop"), None),
            vec![PathBuf::from("/usr/bin")]
        );
        assert_eq!(
            sidecar_dirs(Path::new("/usr/bin/proxysvpn-desktop"), Some(Path::new("/src/binaries"))),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/src/binaries")]
        );
    }

    #[test]
    fn frames_round_trip_through_json() {
        let frames = vec![
            Frame::Request { id: 1, req: Request::Hello },
            Frame::Request { id: 2, req: Request::Up(sample()) },
            Frame::Request { id: 3, req: Request::Ensure(sample()) },
            Frame::Request { id: 6, req: Request::Retarget(sample()) },
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

    /// The port decides where every packet goes once the half-defaults are in
    /// place, so the helper accepts only the two ports this app's own engines
    /// listen on — never one it was merely told about.
    #[test]
    fn rejects_any_port_that_is_not_an_engine_port() {
        for bad in [0u16, 1, 1080, 8080, 10807, 10810, 65535] {
            let mut p = sample();
            p.socks_port = bad;
            assert!(validate_up(&p).is_err(), "port {bad} must be rejected");
        }
        for good in ALLOWED_SOCKS_PORTS {
            let mut p = sample();
            p.socks_port = good;
            assert!(validate_up(&p).is_ok(), "port {good} must be accepted");
        }
    }

    /// The `--dev` flag is an argument of the helper, and the helper's argv is
    /// chosen by an unprivileged peer. A release build must therefore ignore it
    /// outright rather than trust that no bundle sets the environment variable.
    #[test]
    fn dev_mode_needs_a_debug_build_not_just_the_flag() {
        assert!(dev_mode_allowed(true, true));
        assert!(!dev_mode_allowed(true, false), "release must ignore --dev");
        assert!(!dev_mode_allowed(false, true));
        assert!(!dev_mode_allowed(false, false));
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
    fn bounded_reader_returns_whole_lines_then_eof() {
        let mut src: &[u8] = b"{\"a\":1}\n{\"b\":2}\n";
        assert_eq!(
            read_bounded_line(&mut src).expect("read"),
            Line::Request("{\"a\":1}\n".to_string())
        );
        assert_eq!(
            read_bounded_line(&mut src).expect("read"),
            Line::Request("{\"b\":2}\n".to_string())
        );
        assert_eq!(read_bounded_line(&mut src).expect("read"), Line::Eof);
    }

    /// An endless line must not be assembled in a root process's memory, and
    /// the frame that follows it must still be readable — otherwise one
    /// oversized request would desynchronise the pipe for good.
    #[test]
    fn bounded_reader_drops_an_oversized_line_and_resynchronises() {
        let mut data = vec![b'x'; (MAX_REQUEST_BYTES as usize) + 10];
        data.push(b'\n');
        data.extend_from_slice(b"{\"next\":true}\n");
        let mut src: &[u8] = &data;

        assert_eq!(read_bounded_line(&mut src).expect("read"), Line::TooLong);
        assert_eq!(
            read_bounded_line(&mut src).expect("read"),
            Line::Request("{\"next\":true}\n".to_string())
        );
        assert_eq!(read_bounded_line(&mut src).expect("read"), Line::Eof);
    }

    /// A peer that closes the pipe mid-line must not leave the fragment to be
    /// parsed as a request.
    #[test]
    fn bounded_reader_refuses_an_unterminated_tail() {
        let mut src: &[u8] = b"{\"partial\":";
        assert_eq!(read_bounded_line(&mut src).expect("read"), Line::TooLong);
        assert_eq!(read_bounded_line(&mut src).expect("read"), Line::Eof);
    }

    #[test]
    fn directories_are_refused() {
        let facts = FileFacts { uid: 0, mode: 0o040755, is_regular: false };
        assert!(sidecar_is_trusted(&facts, None, false, true).is_err());
    }
}
