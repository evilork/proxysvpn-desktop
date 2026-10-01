// src-tauri/src/privilege.rs
//! Privilege model, per platform.
//!
//! macOS — the whole process is root: `scripts/launcher.sh` asks for the
//! password once via `osascript`, then re-execs the real binary through
//! `launchctl asuser`, so the GUI keeps its session while running as uid 0.
//!
//! Linux — the GUI stays unprivileged and a small root helper (the same
//! executable started with `--helper` through `pkexec`) owns the TUN device,
//! the routes and DNS. Rationale, in order of weight:
//!
//!   1. Running the GUI as root is not an option on Wayland: a root process
//!      cannot connect to the user's compositor, so the window never appears.
//!      On X11 it needs `xhost +si:localuser:root`. macOS has no equivalent
//!      problem because `launchctl asuser` keeps us inside the GUI session.
//!   2. File capabilities (`setcap cap_net_admin+ep`) are *not* inherited by
//!      children, so xray/tun2socks would still lack them; and capabilities
//!      are lost inside an AppImage mount, so the .deb and the AppImage would
//!      need different privilege models.
//!   3. `pkexec` prompts once per authorization (`auth_admin_keep`) and the
//!      helper is kept alive for the whole app session, so the user sees one
//!      dialog per run — the same feel as macOS.
//!
//! Without our polkit policy file installed, `pkexec` still works through the
//! built-in `org.freedesktop.policykit.exec` action; the dialog is just
//! generic. That is what happens in the AppImage, which cannot install the
//! policy. See `src-tauri/linux/com.proxysvpn.desktop.policy`.

/// True when the current process can configure interfaces and routes itself.
pub fn is_elevated() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: getuid() takes no arguments, cannot fail and has no
        // side effects.
        unsafe { libc::getuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Engine processes we may safely kill at startup and at exit: leftovers from
/// a crash keep the SOCKS ports busy and the next connect would fail.
///
/// macOS keeps exactly the pre-split list so that behaviour does not change.
#[cfg(target_os = "macos")]
const LEFTOVER_ENGINES: &[&str] = &["tun2socks", "xray"];

/// On Linux the engines that belong to the GUI are xray and hysteria;
/// tun2socks is a child of the root helper and the helper reaps it.
#[cfg(target_os = "linux")]
const LEFTOVER_ENGINES: &[&str] = &["xray", "hysteria"];

#[cfg(unix)]
pub fn kill_leftover_engines() {
    use std::process::Command;

    for name in LEFTOVER_ENGINES {
        #[cfg(target_os = "macos")]
        let mut cmd = {
            let mut c = Command::new("/usr/bin/pkill");
            c.args(["-9", "-x", name]);
            c
        };
        // Scope the kill to our own uid: the GUI is unprivileged here and a
        // bare `pkill -x xray` would also try (and fail) on other users'.
        #[cfg(target_os = "linux")]
        let mut cmd = {
            // SAFETY: see is_elevated().
            let uid = unsafe { libc::getuid() }.to_string();
            let mut c = Command::new("pkill");
            c.args(["-9", "-u", &uid, "-x", name]);
            c
        };
        let _ = cmd.status();
    }
}

#[cfg(not(unix))]
pub fn kill_leftover_engines() {
    // Windows: taskkill, implemented on the Windows branch.
}

/// Human-readable reason why we cannot gain privileges, or `None` when we can.
#[cfg(target_os = "linux")]
pub fn elevation_blocker() -> Option<String> {
    if is_elevated() {
        return None;
    }
    if which("pkexec").is_none() {
        return Some(
            "не найден pkexec (пакет polkit) — установите polkit или запустите приложение от root"
                .to_string(),
        );
    }
    None
}

/// Minimal `which`: PATH lookup without pulling a crate in.
#[cfg(target_os = "linux")]
pub fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(target_os = "linux")]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_list_is_not_empty_and_has_no_paths() {
        assert!(!LEFTOVER_ENGINES.is_empty());
        for name in LEFTOVER_ENGINES {
            assert!(!name.contains('/'), "pkill -x expects a bare process name");
        }
    }

    #[test]
    fn is_elevated_matches_uid_on_unix() {
        #[cfg(unix)]
        {
            // SAFETY: see is_elevated().
            let uid = unsafe { libc::getuid() };
            assert_eq!(is_elevated(), uid == 0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn which_finds_sh_and_misses_nonsense() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-9f3a").is_none());
    }
}
