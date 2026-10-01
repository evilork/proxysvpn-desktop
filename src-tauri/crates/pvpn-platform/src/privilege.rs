// src-tauri/crates/pvpn-platform/src/privilege.rs
// "Do we have the rights to create a TUN device and edit the route table?"
//
// macOS: the whole process already runs as root — scripts/launcher.sh asks for
// the password once via osascript and re-execs the real binary through
// `launchctl asuser`, so the GUI stays in the user's session.
//
// Windows: the exe carries an application manifest with
// requestedExecutionLevel="requireAdministrator" (see src-tauri/build.rs and
// windows-app.manifest), so Windows shows one UAC prompt at launch and the
// process — including every sidecar it spawns — is elevated. That mirrors the
// macOS model exactly, which is why it was chosen over a separate privileged
// helper service: one prompt, one process tree, no IPC surface, and the
// existing supervisor/teardown logic keeps working untouched.
//
// Linux: the GUI stays unprivileged and a small root helper (this same
// executable started with `--helper` through pkexec) owns the device, the routes
// and DNS. That is not a preference, it is forced: a root process cannot connect
// to a Wayland compositor, so an elevated GUI never shows a window. See
// `net::linux` for the full reasoning, and note that this is the *stricter*
// model — the webview never runs as root there.
//
// The cost on macOS and Windows is real and documented in docs/WINDOWS.md: the
// WebView runs elevated too. It is now at least confined by a content security
// policy (tauri.conf.json) and by a capability set that does not include
// `shell`, but confinement is not the same as not being root. Linux already
// demonstrates the fix, and `net`'s contract is deliberately the same narrow
// three-operation shape on all three platforms, so moving Windows behind the
// same IPC boundary does not touch a single caller.

/// True when the process may reconfigure interfaces and routes.
pub fn is_elevated() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: getuid() takes no arguments, cannot fail and has no
        // side effects.
        unsafe { libc::getuid() == 0 }
    }

    #[cfg(windows)]
    {
        windows_is_elevated().unwrap_or(false)
    }
}

#[cfg(windows)]
fn windows_is_elevated() -> anyhow::Result<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    // SAFETY: `token` is a valid out-pointer; the pseudo-handle from
    // GetCurrentProcess needs no release.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }?;

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION and we pass its exact size.
    let query = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };

    // Close the token on both paths before propagating the query result.
    // SAFETY: `token` came from OpenProcessToken and is closed exactly once.
    let closed = unsafe { CloseHandle(token) };
    query?;
    closed?;

    Ok(elevation.TokenIsElevated != 0)
}

/// Why this machine cannot give the tunnel the rights it needs.
///
/// Not the person's refusal: "press Retry and allow it in the system dialog"
/// would not help, because no dialog can succeed. The GUI finds this type in
/// an error chain (`is_elevation_unavailable`) and shows its own code for it
/// instead of the permission one. The texts are Russian, like every other
/// message of this layer that reaches the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationUnavailable {
    /// pkexec (polkit) is not installed: there is no system dialog to show.
    NoPkexec,
    /// This executable lives on a FUSE mount — an AppImage. The kernel lets
    /// nobody but the user who mounted it execute from there, root included,
    /// so pkexec would ask for the password and then fail to start the helper.
    FuseMount,
}

impl std::fmt::Display for ElevationUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoPkexec => {
                "не найден pkexec (polkit) — установите пакет pkexec или policykit-1 либо запустите приложение от root"
            }
            Self::FuseMount => {
                "приложение запущено из AppImage, а root не может запустить файл из её монтирования — установите пакет .deb"
            }
        })
    }
}

impl std::error::Error for ElevationUnavailable {}

/// Does `err`, anywhere in its chain, say that this machine cannot elevate?
pub fn is_elevation_unavailable(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<ElevationUnavailable>())
}

/// Why we cannot gain privileges, or `None` when we can.
///
/// Linux only: it is the one platform where elevation happens at connect time
/// rather than at launch, so the reason has to be reportable mid-session.
#[cfg(target_os = "linux")]
pub fn elevation_blocker() -> Option<ElevationUnavailable> {
    if is_elevated() {
        return None;
    }
    if which("pkexec").is_none() {
        return Some(ElevationUnavailable::NoPkexec);
    }
    let on_fuse = std::env::current_exe()
        .map(|exe| on_fuse_mount(&exe))
        .unwrap_or(false);
    if on_fuse {
        return Some(ElevationUnavailable::FuseMount);
    }
    None
}

/// `FUSE_SUPER_MAGIC` from linux/magic.h: what statfs(2) reports for a FUSE
/// file system, which is what an AppImage mounts itself as.
#[cfg(target_os = "linux")]
const FUSE_SUPER_MAGIC: libc::__fsword_t = 0x6573_5546;

/// Is `path` on a FUSE file system? Asked of the kernel rather than guessed
/// from `$APPIMAGE` or a `/tmp/.mount_` prefix: an AppImage extracted with
/// --appimage-extract-and-run is a plain directory root can execute from, and
/// the environment variable is inherited by whatever an AppImage starts.
#[cfg(target_os = "linux")]
fn on_fuse_mount(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: an all-zero `statfs` is a valid value of this plain C struct;
    // statfs(2) only writes into it.
    let mut info: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a NUL-terminated string and `info` a live, writable
    // struct, both valid for the duration of the call.
    if unsafe { libc::statfs(c_path.as_ptr(), &mut info) } != 0 {
        return false;
    }
    info.f_type == FUSE_SUPER_MAGIC
}

/// Minimal `which`: a PATH lookup without pulling in a crate.
///
/// Also used by the helper to resolve `ip`/`resolvectl`, which is why it rejects
/// anything that is not an executable regular file — a directory named `ip` on
/// PATH would otherwise be returned and every route call would fail obscurely.
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

/// Message shown when the app is started without the rights it needs.
/// Russian, because it is surfaced in the UI.
pub fn missing_privileges_message() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        // Unchanged pre-split wording.
        "приложение не запущено от root — перезапустите через ProxysVPN Launcher"
    }
    #[cfg(target_os = "windows")]
    {
        "приложение запущено без прав администратора — закройте и запустите ProxysVPN снова, подтвердив запрос Windows"
    }
    #[cfg(target_os = "linux")]
    {
        "приложение запущено без прав администратора — запустите ProxysVPN через pkexec"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_not_empty() {
        assert!(!missing_privileges_message().is_empty());
    }

    /// The GUI tells "this machine cannot elevate" apart from "the person
    /// said no" by the type in the chain, however much context the platform
    /// layer wraps around it on the way up.
    #[test]
    fn an_unavailable_elevation_is_found_through_context() {
        for reason in [ElevationUnavailable::NoPkexec, ElevationUnavailable::FuseMount] {
            let err = anyhow::Error::new(reason)
                .context("spawn the privileged helper")
                .context("preflight");
            assert!(is_elevation_unavailable(&err), "{reason:?}");
            assert!(!reason.to_string().is_empty());
        }
        let refused = anyhow::anyhow!("запрос прав отменён");
        assert!(!is_elevation_unavailable(&refused));
    }

    /// The AppImage text sends the person to the package that works, and the
    /// pkexec one names both package spellings the .deb depends on.
    #[test]
    fn each_reason_names_its_way_out() {
        assert!(ElevationUnavailable::FuseMount.to_string().contains(".deb"));
        let pkexec = ElevationUnavailable::NoPkexec.to_string();
        assert!(pkexec.contains("pkexec") && pkexec.contains("policykit-1"), "{pkexec}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn which_finds_sh_and_misses_nonsense() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-9f3a").is_none());
    }

    /// A directory on PATH named like the tool must not be mistaken for it.
    #[cfg(target_os = "linux")]
    #[test]
    fn which_refuses_a_directory() {
        assert!(!is_executable(std::path::Path::new("/usr/bin")));
        assert!(!is_executable(std::path::Path::new("/definitely/not/here")));
    }

    #[cfg(unix)]
    #[test]
    fn elevation_matches_uid() {
        // SAFETY: see is_elevated.
        let uid = unsafe { libc::getuid() };
        assert_eq!(is_elevated(), uid == 0);
    }
}
