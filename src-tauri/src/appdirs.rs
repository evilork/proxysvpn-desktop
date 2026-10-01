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

use std::path::PathBuf;

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

#[cfg(test)]
mod tests {
    use super::*;

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
