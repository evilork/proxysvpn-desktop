// src-tauri/src/consent.rs
//
// Whether this installation has already shown the data notice.
//
// ── Why the core keeps this, not the window ─────────────────────────────────
// Guideline 5.4 wants the declaration of what data a VPN app uses on a screen
// shown BEFORE the service is used. The window asks the core which first-run
// steps are pending (`onboarding_state`), and the core is also what opens the
// first connection to the service (pairing, the subscription) — so the core is
// the one place that can say "not before the notice" and mean it. The window's
// localStorage would also be the wrong home: a reset WebView cache would bring
// the notice back while the subscription link, which lives beside this file,
// stays.
//
// ── Why a version in the file ───────────────────────────────────────────────
// A notice accepted once is not accepted forever: when what the app does with
// data changes, the notice changes, and everybody has to see it again. Bumping
// `DATA_NOTICE_VERSION` does that. A file that says less, or that cannot be
// read as a record at all (truncated by a crash, edited by hand), counts as
// not accepted — showing a notice twice is harmless, skipping it is not.
//
// ── Why tmp + rename ────────────────────────────────────────────────────────
// A crash in the middle of a plain write leaves half a file, which reads as
// "not accepted" and shows the notice again — harmless but sloppy. The rename
// is atomic within a directory, so the record is either the old one or the
// complete new one.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Raise to show the data notice again to every installation.
pub const DATA_NOTICE_VERSION: u32 = 1;

/// The record's name inside each directory the subscription link may live in.
pub const FILE_NAME: &str = "consent-v1";

/// A record is a few dozen bytes; anything much larger is not ours.
const MAX_RECORD_BYTES: u64 = 4096;

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    /// Version of the notice that was shown.
    v: u32,
    /// When "Continue" was pressed, unix milliseconds. Kept for support ("since
    /// when does this install run"), never sent anywhere.
    at: u64,
}

/// True when none of `paths` holds a record for the current notice.
///
/// Paths are read in order and the first valid record wins; an unreadable or
/// outdated one is skipped rather than trusted, so a stale file in the first
/// location cannot hide a good one in the second.
pub fn is_pending(paths: &[PathBuf]) -> bool {
    !paths
        .iter()
        .any(|path| read_version(path).is_some_and(|v| v >= DATA_NOTICE_VERSION))
}

/// Writes the record for the current notice into the first of `paths` that
/// accepts it. Returns the error of the last attempt when none did.
pub fn record(paths: &[PathBuf], now_ms: u64) -> io::Result<()> {
    let body = serde_json::to_vec(&Record {
        v: DATA_NOTICE_VERSION,
        at: now_ms,
    })
    .map_err(io::Error::other)?;
    let mut last: Option<io::Error> = None;
    for path in paths {
        match write_atomic(path, &body) {
            Ok(()) => return Ok(()),
            Err(err) => last = Some(err),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no location to write to")))
}

fn read_version(path: &Path) -> Option<u32> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    // `take` bounds the read even if something replaced the file with a huge
    // one; a record that long fails to parse and counts as absent.
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return None;
    }
    serde_json::from_slice::<Record>(&bytes).ok().map(|record| record.v)
}

fn write_atomic(path: &Path, body: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "record path has no directory"))?;
    fs::create_dir_all(dir)?;
    // Per process, so two instances starting at once cannot interleave writes
    // into one temporary file; the rename decides which complete record stays.
    let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let result = write_then_rename(&tmp, path, body);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_then_rename(tmp: &Path, path: &Path, body: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(tmp)?;
    // `mode` applies only when the file is created; a temporary file left by a
    // crashed run keeps whatever it had, so the mode is set again explicitly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(body)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("proxysvpn-consent-{tag}-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn nothing_on_disk_means_the_notice_is_pending() {
        let dir = temp_dir("absent");
        assert!(is_pending(&[dir.join(FILE_NAME)]));
        assert!(is_pending(&[]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_record_for_the_current_notice_clears_it() {
        let dir = temp_dir("record");
        let path = dir.join("nested").join(FILE_NAME);
        record(std::slice::from_ref(&path), 1_700_000_000_000).expect("record");
        assert!(!is_pending(std::slice::from_ref(&path)));

        let text = fs::read_to_string(&path).expect("read back");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(value["v"], DATA_NOTICE_VERSION);
        assert_eq!(value["at"], 1_700_000_000_000u64);

        // No temporary file is left next to the record.
        let leftovers: Vec<_> = fs::read_dir(path.parent().expect("dir"))
            .expect("list")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_record_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join(FILE_NAME);
        record(std::slice::from_ref(&path), 1).expect("record");
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_older_notice_version_shows_the_notice_again() {
        let dir = temp_dir("old");
        let path = dir.join(FILE_NAME);
        fs::write(&path, br#"{"v":0,"at":1}"#).expect("write");
        assert!(is_pending(&[path]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_record_shows_the_notice_again() {
        let dir = temp_dir("damaged");
        let path = dir.join(FILE_NAME);
        for body in [
            &b"{\"v\":1,\"at\":"[..],
            b"",
            b"not json",
            b"{\"v\":\"1\",\"at\":1}",
            b"{\"v\":-1,\"at\":1}",
            b"{\"at\":1}",
        ] {
            fs::write(&path, body).expect("write");
            assert!(is_pending(std::slice::from_ref(&path)), "{:?}", String::from_utf8_lossy(body));
        }
        let huge = format!("{{\"v\":1,\"at\":1,\"pad\":\"{}\"}}", "x".repeat(8192));
        fs::write(&path, huge).expect("write");
        assert!(is_pending(&[path]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_good_record_in_a_later_location_still_counts() {
        let dir = temp_dir("second");
        let first = dir.join("a").join(FILE_NAME);
        let second = dir.join("b").join(FILE_NAME);
        fs::create_dir_all(first.parent().expect("dir")).expect("mkdir");
        fs::write(&first, b"garbage").expect("write");
        record(std::slice::from_ref(&second), 5).expect("record");
        assert!(!is_pending(&[first, second]));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unwritable_location_falls_through_to_the_next() {
        let dir = temp_dir("fallthrough");
        // A regular file where the directory should be: create_dir_all fails.
        let blocker = dir.join("blocked");
        fs::write(&blocker, b"").expect("write");
        let bad = blocker.join(FILE_NAME);
        let good = dir.join("ok").join(FILE_NAME);
        record(&[bad.clone(), good.clone()], 9).expect("record");
        assert!(!bad.exists());
        assert!(!is_pending(&[good]));
        assert!(record(&[bad], 9).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
