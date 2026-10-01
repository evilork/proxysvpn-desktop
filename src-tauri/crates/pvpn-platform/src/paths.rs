// src-tauri/crates/pvpn-platform/src/paths.rs
// One place that knows where things live on disk.
//
// macOS values are exactly the literals the pre-split code used, so an
// upgrade keeps finding the same log file and cleans up the same route hint.
//
// Windows deliberately uses %LOCALAPPDATA% rather than %PROGRAMDATA% for the
// state directory: the hysteria config holds the node password, and the
// default ACL on %PROGRAMDATA% grants read access to every local user, while
// %LOCALAPPDATA% is already restricted to the owning account. Under UAC the
// elevated process keeps the invoking user's profile, so the path is stable.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const APP_DIR: &str = "ProxysVPN";

// ---------------------------------------------------------------- pure joins
//
// Compiled on every host (not behind cfg) so the unit tests below can check
// all three layouts from one machine.

#[allow(dead_code)]
fn macos_log_file(home: &str) -> PathBuf {
    PathBuf::from(home).join("Library/Logs/ProxysVPN/app.log")
}

#[allow(dead_code)]
fn windows_log_file(local_app_data: &str) -> PathBuf {
    PathBuf::from(local_app_data).join(APP_DIR).join("logs").join("app.log")
}

#[allow(dead_code)]
fn linux_log_file(state_dir: &Path) -> PathBuf {
    state_dir.join("app.log")
}

#[allow(dead_code)]
fn linux_state_dir(xdg_state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match (xdg_state_home, home) {
        (Some(x), _) if !x.is_empty() => Some(PathBuf::from(x).join(APP_DIR)),
        (_, Some(h)) if !h.is_empty() => Some(PathBuf::from(h).join(".local/state").join(APP_DIR)),
        _ => None,
    }
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

// ------------------------------------------------------------- platform view

/// Directory for mutable app state (hysteria config, route hint).
/// Created on demand.
pub fn state_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let dir = PathBuf::from("/tmp");

    #[cfg(target_os = "windows")]
    let dir = {
        let base = env_non_empty("LOCALAPPDATA")
            .ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA is not set — cannot locate app state"))?;
        PathBuf::from(base).join(APP_DIR)
    };

    #[cfg(target_os = "linux")]
    let dir = linux_state_dir(
        env_non_empty("XDG_STATE_HOME").as_deref(),
        env_non_empty("HOME").as_deref(),
    )
    .ok_or_else(|| anyhow::anyhow!("neither XDG_STATE_HOME nor HOME is set"))?;

    if !dir.is_dir() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
    }
    Ok(dir)
}

/// Mirror file for the support log. `None` when the environment gives us
/// nowhere to write — the in-memory ring buffer still works.
pub fn log_file() -> Option<PathBuf> {
    // Unchanged on macOS: $HOME/Library/Logs/ProxysVPN/app.log
    #[cfg(target_os = "macos")]
    let path = macos_log_file(&env_non_empty("HOME")?);

    #[cfg(target_os = "windows")]
    let path = windows_log_file(&env_non_empty("LOCALAPPDATA")?);

    #[cfg(target_os = "linux")]
    let path = linux_log_file(&linux_state_dir(
        env_non_empty("XDG_STATE_HOME").as_deref(),
        env_non_empty("HOME").as_deref(),
    )?);

    Some(path)
}

/// Config handed to the hysteria client. Holds the node password, so it must be
/// written with [`write_private_file`].
pub fn hy2_config_file() -> Result<PathBuf> {
    // Unchanged literal on macOS: a fixed path, because TMPDIR differs under sudo.
    #[cfg(target_os = "macos")]
    let path = PathBuf::from("/tmp/proxysvpn-hy2.yaml");

    #[cfg(not(target_os = "macos"))]
    let path = state_dir()?.join("hy2.yaml");

    Ok(path)
}

/// Records the node IP whose host route we installed, so that a run which
/// follows a crash can remove a route it never created itself.
pub fn route_hint_file() -> Result<PathBuf> {
    // Unchanged literal on macOS (the pre-split PID_FILE).
    #[cfg(target_os = "macos")]
    let path = PathBuf::from("/tmp/proxysvpn-desktop.pid");

    #[cfg(not(target_os = "macos"))]
    let path = state_dir()?.join(ROUTE_HINT_NAME);

    Ok(path)
}

/// Name of the route-hint file, shared by the GUI's state directory and the
/// helper's runtime directory so both sides look for the same thing.
pub const ROUTE_HINT_NAME: &str = "route-hint";

/// Where the Linux root helper keeps its own state.
///
/// `/run` and not the user's state directory: the helper is root and must not
/// write into a directory an unprivileged process controls — a symlink planted
/// there would let any local user aim root's writes at an arbitrary file. /run
/// is tmpfs, so the hint also disappears on reboot, which is exactly right for
/// a breadcrumb about routes that did not survive either.
#[cfg(target_os = "linux")]
pub fn linux_runtime_dir() -> PathBuf {
    PathBuf::from("/run/proxysvpn")
}

/// Writes a file that only its owner may read, refusing to follow a symlink.
///
/// Both callers write a fixed, predictable path, and on macOS they do it **as
/// root** into `/tmp`, which is mode 1777 and empty after a reboot. A plain
/// `fs::write` follows symlinks, so any local user could plant
/// `/tmp/proxysvpn-hy2.yaml -> /etc/sudoers` and have root overwrite that file
/// (and, with the old `chmod 600` afterwards, change its mode too). Keeping the
/// path and refusing the link is the fix that does not break the upgrade
/// cleanup, which still looks for those names.
///
/// The sequence is unlink, then create with `O_EXCL`: if the attacker wins the
/// race and re-creates the link in between, the create fails and we return an
/// error instead of writing through it. 0600 also replaces the old
/// write-then-chmod, which left the hysteria config holding the node password
/// world-readable for the moment in between.
///
/// Windows: `%LOCALAPPDATA%` already inherits an owner-only ACL and has no
/// symlink exposure of this kind, so the write is plain.
pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;

    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e).with_context(|| format!("replace {}", path.display()));
        }
    }

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }

    let mut file = opts
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_log_path_is_unchanged() {
        assert_eq!(
            macos_log_file("/Users/x"),
            PathBuf::from("/Users/x/Library/Logs/ProxysVPN/app.log")
        );
    }

    #[test]
    fn windows_log_path_lives_under_localappdata() {
        assert_eq!(
            windows_log_file(r"C:\Users\x\AppData\Local"),
            PathBuf::from(r"C:\Users\x\AppData\Local")
                .join("ProxysVPN")
                .join("logs")
                .join("app.log")
        );
    }

    #[test]
    fn linux_state_dir_prefers_xdg() {
        assert_eq!(
            linux_state_dir(Some("/home/x/.local/state"), Some("/home/x")),
            Some(PathBuf::from("/home/x/.local/state/ProxysVPN"))
        );
        assert_eq!(
            linux_state_dir(None, Some("/home/x")),
            Some(PathBuf::from("/home/x/.local/state/ProxysVPN"))
        );
        assert_eq!(linux_state_dir(Some(""), Some("")), None);
        assert_eq!(linux_state_dir(None, None), None);
    }

    #[test]
    fn linux_log_lives_in_state_dir() {
        let state = PathBuf::from("/home/x/.local/state/ProxysVPN");
        assert_eq!(linux_log_file(&state), state.join("app.log"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_fixed_paths_are_byte_identical() {
        assert_eq!(
            hy2_config_file().expect("hy2 path"),
            PathBuf::from("/tmp/proxysvpn-hy2.yaml")
        );
        assert_eq!(
            route_hint_file().expect("hint path"),
            PathBuf::from("/tmp/proxysvpn-desktop.pid")
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_write_creates_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("pvpn-secret-{}", std::process::id()));
        std::fs::remove_file(&path).ok();

        write_private_file(&path, b"secret").expect("write");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the node password must not be readable by others");
        assert_eq!(std::fs::read(&path).expect("read"), b"secret");

        // Rewriting must replace the contents, not append to them.
        write_private_file(&path, b"second").expect("rewrite");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");

        std::fs::remove_file(&path).ok();
    }

    /// The reason this helper exists: on macOS these files are written by root
    /// into mode-1777 /tmp under a predictable name, so following a symlink
    /// would hand any local user a root-owned write to a file of their choice.
    #[cfg(unix)]
    #[test]
    fn private_write_refuses_to_follow_a_planted_symlink() {
        let dir = std::env::temp_dir().join(format!("pvpn-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let victim = dir.join("victim");
        let link = dir.join("config.yaml");
        std::fs::write(&victim, b"do not touch").expect("victim");
        std::os::unix::fs::symlink(&victim, &link).expect("symlink");

        // The unlink step removes the link itself, so the write lands on a
        // fresh regular file and the victim keeps its contents.
        write_private_file(&link, b"ours").expect("write");
        assert_eq!(std::fs::read(&victim).expect("read"), b"do not touch");
        assert_eq!(std::fs::read(&link).expect("read"), b"ours");
        assert!(
            !std::fs::symlink_metadata(&link)
                .expect("stat")
                .file_type()
                .is_symlink(),
            "the link must have been replaced by a regular file"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
