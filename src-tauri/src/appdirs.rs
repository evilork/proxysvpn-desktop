// src-tauri/src/appdirs.rs
//
// Where the app keeps its own small files: the subscription link, the data
// notice record, the device id, the tunnel and notification preferences, the
// per-network memory, the manifest cache and the engine pid list.
//
// Every one of those stores reads the directories below in order and writes to
// the first one that accepts the write, so the order is load-bearing:
//
//   macOS   — /Library/Application Support/ProxysVPN first, then
//             ~/Library/Application Support/com.proxysvpn.desktop. The launcher
//             starts the app as root with two different `HOME`s depending on
//             the path it takes, and the shared system folder is the one both
//             of them see. Byte-for-byte the list every store used to build by
//             hand.
//   iOS     — ~/Library/Application Support inside the app's own container,
//             which is already private to the app.
//   Windows — %LOCALAPPDATA%\ProxysVPN, from the platform layer: per-user, with
//             an owner-only ACL, and the same folder under UAC because the
//             elevated process keeps the invoking user's profile.
//   Linux   — $XDG_DATA_HOME/ProxysVPN (or ~/.local/share/ProxysVPN), owner
//             only. The GUI is unprivileged there, so this is the user's own.

use std::path::{Path, PathBuf};

/// The directories app data may live in, most preferred first. Empty when the
/// environment names none, which every store treats as "nothing stored yet".
pub fn state_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();

    #[cfg(target_os = "macos")]
    {
        out.push(PathBuf::from("/Library/Application Support/ProxysVPN"));
        if let Ok(home) = std::env::var("HOME") {
            out.push(
                PathBuf::from(home)
                    .join("Library/Application Support")
                    .join("com.proxysvpn.desktop"),
            );
        }
    }

    #[cfg(target_os = "ios")]
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join("Library/Application Support"));
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    match pvpn_platform::paths::data_dir() {
        Ok(dir) => out.push(dir),
        // Logged rather than fatal: the stores fall back to "nothing stored",
        // which is a first run, not a crash.
        Err(e) => crate::logger::log("warn", "app", &format!("no data directory: {e:#}")),
    }

    out
}

/// Write one of the app's small files so that only its owner can read it.
///
/// A temp file created owner-only (0600 on Unix) in the same folder, then
/// renamed over the target: there is no moment when the content exists with
/// looser permissions (the old write-then-chmod left a new subscription link
/// at 0644 for a moment), an existing world-readable file is replaced by a
/// private one rather than kept, and a symlink planted at the name is
/// replaced, not written through. On macOS the folder is the shared
/// /Library/Application Support/ProxysVPN, readable by every account on the
/// Mac; the link holds the account credential and the tunnel preferences the
/// person's own "always direct / always via VPN" lists.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no name"))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let result = options
        .open(&tmp)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only the unix tests below use it.
    #[cfg(unix)]
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pvpn-appdirs-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    /// The link and the tunnel preferences live in a folder every account
    /// on a Mac can read: they must be owner-only from the first byte, and a
    /// file left world-readable by an older version must not stay so.
    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only_even_over_an_old_readable_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        let path = dir.join("tunnel-prefs.json");
        std::fs::write(&path, b"old").expect("old file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        write_private(&path, b"new").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"new");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_the_name_is_replaced_not_written_through() {
        let dir = scratch("link");
        let victim = dir.join("victim");
        std::fs::write(&victim, b"keep").expect("victim");
        let path = dir.join("sub-link");
        std::os::unix::fs::symlink(&victim, &path).expect("link");

        write_private(&path, b"https://proxysvpn.com/api/sub/x\n").expect("write");
        assert_eq!(std::fs::read(&victim).expect("victim"), b"keep");
        assert!(!std::fs::symlink_metadata(&path).expect("meta").file_type().is_symlink());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The macOS list is the one every store built by hand before this module
    /// existed; an upgrade must keep finding the same files.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keeps_the_shared_folder_first_and_the_per_user_one_second() {
        let dirs = state_dirs();
        assert_eq!(
            dirs.first(),
            Some(&PathBuf::from("/Library/Application Support/ProxysVPN"))
        );
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(
                dirs.get(1),
                Some(
                    &PathBuf::from(home)
                        .join("Library/Application Support")
                        .join("com.proxysvpn.desktop")
                )
            );
        }
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn windows_and_linux_use_the_platform_data_dir() {
        let dirs = state_dirs();
        assert_eq!(dirs.len(), 1, "{dirs:?}");
        assert!(dirs[0].to_string_lossy().contains("ProxysVPN"), "{dirs:?}");
        assert!(dirs[0].is_absolute(), "{dirs:?}");
    }
}
