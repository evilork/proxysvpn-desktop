// src-tauri/src/device_id.rs
//
// The identity this installation shows the subscription endpoint: one random
// UUID, made once and kept, plus the User-Agent that travels beside it.
//
// ── Why we send an identity at all ──────────────────────────────────────────
// The service rule is "one link — one device", and the server enforces it
// strictly inside `if (hwid)` (frontend/src/app/api/sub/[token]/route.ts). Happ
// sends `x-hwid` by default; our client did not send it at all, which made our
// own app the most convenient way to run one paid link on any number of
// machines. Sending it puts us in line with every other client, and as a side
// effect the `sub_seen` / `sub_ua` traces the support screens read stop being
// guesses.
//
// ── Why NOT a hardware identifier ───────────────────────────────────────────
// The README promises we collect nothing about the machine, and a serial
// number or a MAC address is exactly the thing a person installs a VPN to stop
// handing out. A random UUID satisfies the rule just as well: the server only
// ever asks "is this the same caller as last time", never "which machine is
// this". So the value carries no information about the computer, and resetting
// it is as easy as deleting one file.
//
// ── Why the id may legitimately be absent ───────────────────────────────────
// An id we cannot PERSIST is worse than no id. A fresh UUID on every launch
// would make the server see a new device every time, and the second launch
// would meet "this link is taken by another device" — the app would lock the
// owner out of his own subscription. So `device_id()` returns `None` when it
// could neither read nor write the file, and the request then goes without the
// header, which the server treats exactly as it treats v2rayN or sing-box.
// That is the status quo, not a weakening of the rule.
//
// ── Why two locations, system-wide first ────────────────────────────────────
// On macOS the app is started by `scripts/launcher.sh`, and that script has two
// paths: `sudo -E` (which keeps the user's `HOME`) and `osascript … with
// administrator privileges` + `launchctl asuser` (which runs with root's
// `HOME`). A file under `$HOME` would therefore land in two different places
// depending on how the app happened to start, and the two would be two
// different devices to the server. `/Library/Application Support` does not
// move with the euid, so that is the primary home for the id; the per-user path
// is only the fallback for a non-root run (`cargo tauri dev`), and it is
// promoted to the shared path as soon as a privileged run can write there.
//
// Module note: this file is declared from `subscription.rs` with `#[path]`
// because `lib.rs` is owned elsewhere. Promote it to a plain `mod device_id;`
// in `lib.rs` when that file is next touched, and drop the declaration there.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// File name in every candidate directory.
const FILE_NAME: &str = "device-id";

/// Bundle identifier, mirrored from `tauri.conf.json`. Only used to name the
/// per-user fallback directory, so a drift here costs a new id at worst — and
/// the shared path, which is the one that matters, does not use it.
const BUNDLE_ID: &str = "com.proxysvpn.desktop";

/// Readable by everyone on purpose.
///
/// The shared file is written by a root run and has to be READABLE by a later
/// non-root run, otherwise that run falls through to the per-user path and
/// becomes a second device. The value is not a credential: it is useless
/// without the subscription token, which lives elsewhere and is the actual
/// secret.
#[cfg(unix)]
const FILE_MODE: u32 = 0o644;

/// Longest id we will read back from disk.
const MAX_ID_LEN: usize = 128;

/// Shortest id we will read back from disk.
const MIN_ID_LEN: usize = 8;

/// The stable identity of this installation, or `None` when it could not be
/// persisted (see the header for why absent beats volatile).
pub fn device_id() -> Option<&'static str> {
    static ID: OnceLock<Option<String>> = OnceLock::new();
    ID.get_or_init(load_or_create).as_deref()
}

/// `ProxysVPN-Desktop/0.1.0 (macOS)`; App Store builds (feature `appstore`)
/// `ProxysVPN/0.3.1 (iOS; appstore)`.
///
/// The server stores this under `sub_ua:<uuid>` and support reads it to answer
/// "which app is this person using" without asking. A generic agent string
/// would make our own client indistinguishable from an unknown one. The App
/// Store tag is the same signal for the service's purchase-text filter as the
/// `x-client-dist` header (subscription.rs), on every request that carries
/// this agent — pairing included.
pub fn user_agent() -> &'static str {
    static UA: OnceLock<String> = OnceLock::new();
    UA.get_or_init(|| {
        format_user_agent(
            env!("CARGO_PKG_VERSION"),
            platform_name(),
            cfg!(feature = "appstore"),
        )
    })
}

/// Both shapes in one place, so each can be tested whichever way the crate
/// was built. The direct shape is byte-for-byte what it always was.
fn format_user_agent(version: &str, platform: &str, appstore: bool) -> String {
    if appstore {
        format!("ProxysVPN/{version} ({platform}; appstore)")
    } else {
        format!("ProxysVPN-Desktop/{version} ({platform})")
    }
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else {
        // Nothing else is shipped today; naming the target beats lying.
        std::env::consts::OS
    }
}

/// Every place the id may live, most stable first.
///
/// Order is load-bearing: reading follows it, so a shared id always wins over a
/// per-user one and the euid the app happened to start with cannot change who
/// we are.
fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();

    // macOS only: the sandbox on iOS makes every path per-app already, and
    // there is no second euid to worry about there.
    #[cfg(target_os = "macos")]
    out.push(PathBuf::from("/Library/Application Support/ProxysVPN").join(FILE_NAME));

    if let Ok(home) = std::env::var("HOME") {
        let mut base = PathBuf::from(home).join("Library/Application Support");
        // On iOS the container is already private to the app, so a second
        // level named after the bundle would only add noise; on macOS the home
        // directory is shared with everything else the person runs.
        if cfg!(target_os = "macos") {
            base = base.join(BUNDLE_ID);
        }
        out.push(base.join(FILE_NAME));
    }

    out
}

fn load_or_create() -> Option<String> {
    let paths = candidate_paths();

    // 1. Anything already stored wins. Never regenerate over a readable id:
    //    the server keeps the binding for a year, and a new id there means the
    //    person is told his own link belongs to someone else.
    for (i, path) in paths.iter().enumerate() {
        if let Some(id) = read_id(path) {
            // Found in a fallback location while a more stable one exists:
            // copy it up so the next run finds it regardless of euid. A failure
            // here changes nothing — we already have the id.
            if i > 0 {
                for better in &paths[..i] {
                    if write_id(better, &id).is_ok() {
                        break;
                    }
                }
            }
            return Some(id);
        }
    }

    // 2. Nothing stored yet. Make one and keep it in the most stable location
    //    that accepts it.
    let fresh = new_uuid_v4()?;
    for path in &paths {
        if write_id(path, &fresh).is_ok() {
            return Some(fresh);
        }
    }

    // 3. Could not persist. Report honestly instead of inventing an id that
    //    would differ on the next launch.
    None
}

fn read_id(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    // Bounded read: a wrong path could point at something huge, and we are
    // looking for 36 characters.
    let mut buf = vec![0u8; MAX_ID_LEN + 1];
    let read = file.read(&mut buf).ok()?;
    let text = std::str::from_utf8(&buf[..read]).ok()?.trim().to_string();
    if is_valid_id(&text) {
        Some(text)
    } else {
        None
    }
}

fn write_id(path: &Path, id: &str) -> Result<(), std::io::Error> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "device id path has no parent")
    })?;
    fs::create_dir_all(dir)?;

    // Write-then-rename: a half-written id read by the next launch would look
    // like a different device.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, format!("{id}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best effort: an unreadable id is still better than no id, so a failed
        // chmod does not fail the write.
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(FILE_MODE));
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Accepts what we write and what a person may have put there by hand.
///
/// Deliberately lenient about the shape and strict about the alphabet: an id
/// edited by support is still a usable id, but a stray log line or an HTML
/// error page must never be mistaken for one.
fn is_valid_id(s: &str) -> bool {
    let len = s.len();
    (MIN_ID_LEN..=MAX_ID_LEN).contains(&len)
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.chars().any(|c| c.is_ascii_hexdigit())
}

/// A version 4 UUID from the system CSPRNG.
///
/// `/dev/urandom` rather than a crate: adding a dependency is a change to
/// `Cargo.toml`, which this change does not own, and on macOS and iOS the
/// device is always present and never blocks.
fn new_uuid_v4() -> Option<String> {
    let mut bytes = [0u8; 16];
    let mut file = fs::File::open("/dev/urandom").ok()?;
    // `read` may return fewer bytes than asked for; 16 from urandom in one go
    // is the normal case, but a short read must not silently produce zeros.
    let mut filled = 0usize;
    while filled < bytes.len() {
        match file.read(&mut bytes[filled..]) {
            Ok(0) => return None,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }

    // RFC 4122: version 4 in the high nibble of byte 6, variant 10 in byte 8.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    Some(format_uuid(&bytes))
}

fn format_uuid(b: &[u8; 16]) -> String {
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("proxysvpn-{tag}-{nanos}"));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn generated_id_is_a_version_4_uuid() {
        let id = new_uuid_v4().expect("urandom available");
        assert_eq!(id.len(), 36, "{id}");
        assert_eq!(id.as_bytes()[14], b'4', "version nibble: {id}");
        let variant = id.as_bytes()[19];
        assert!(
            matches!(variant, b'8' | b'9' | b'a' | b'b'),
            "variant nibble: {id}"
        );
        assert!(is_valid_id(&id));
    }

    #[test]
    fn two_ids_differ() {
        // A generator that repeats itself would silently merge two machines
        // into one device on the server.
        let a = new_uuid_v4().expect("urandom");
        let b = new_uuid_v4().expect("urandom");
        assert_ne!(a, b);
    }

    #[test]
    fn written_id_reads_back_unchanged() {
        let dir = temp_dir("id-roundtrip");
        let path = dir.join(FILE_NAME);
        let id = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
        write_id(&path, id).expect("write");
        assert_eq!(read_id(&path).as_deref(), Some(id));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_creates_missing_directories() {
        let dir = temp_dir("id-mkdir");
        let path = dir.join("a").join("b").join(FILE_NAME);
        write_id(&path, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").expect("write");
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_in_the_file_is_not_an_id() {
        let dir = temp_dir("id-garbage");
        let path = dir.join(FILE_NAME);
        // An HTML error page, a log line and an empty file have all been found
        // where a small state file was expected.
        for junk in ["", "   ", "<html>not an id</html>", "zzzz-zzzz"] {
            fs::write(&path, junk).expect("write junk");
            assert!(read_id(&path).is_none(), "accepted {junk:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = temp_dir("id-huge");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "a".repeat(MAX_ID_LEN + 50)).expect("write");
        assert!(read_id(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = temp_dir("id-missing");
        assert!(read_id(&dir.join("nope")).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_agent_names_the_product_version_and_platform() {
        let ua = user_agent();
        assert!(ua.starts_with("ProxysVPN"), "{ua}");
        assert!(ua.contains(env!("CARGO_PKG_VERSION")), "{ua}");
        assert!(ua.contains(platform_name()), "{ua}");
        assert!(ua.ends_with(')'), "{ua}");
        // No node address, no machine name, no user name ever leaves here.
        assert!(!ua.contains('@'), "{ua}");
    }

    #[cfg(not(feature = "appstore"))]
    #[test]
    fn direct_build_user_agent_is_unchanged() {
        let expected = format!(
            "ProxysVPN-Desktop/{} ({})",
            env!("CARGO_PKG_VERSION"),
            platform_name()
        );
        assert_eq!(user_agent(), expected);
        assert!(!user_agent().contains("appstore"));
    }

    #[cfg(feature = "appstore")]
    #[test]
    fn appstore_build_user_agent_says_so() {
        let ua = user_agent();
        assert!(ua.starts_with("ProxysVPN/"), "{ua}");
        assert!(ua.ends_with("; appstore)"), "{ua}");
    }

    #[test]
    fn both_user_agent_shapes() {
        assert_eq!(
            format_user_agent("1.2.3", "iOS", true),
            "ProxysVPN/1.2.3 (iOS; appstore)"
        );
        assert_eq!(
            format_user_agent("1.2.3", "macOS", false),
            "ProxysVPN-Desktop/1.2.3 (macOS)"
        );
    }
}
