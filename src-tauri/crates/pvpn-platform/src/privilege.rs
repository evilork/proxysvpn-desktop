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
    /// This executable lives on a FUSE mount that is not an AppImage we
    /// recognise (an AppImage is: helper/install.rs copies its helper out).
    /// The kernel lets nobody but the user who mounted it execute from there,
    /// root included, so pkexec would ask for the password and then fail to
    /// start the helper.
    FuseMount,
    /// pkexec is installed but the session runs no polkit authentication
    /// agent (i3, sway, Openbox and other minimal sessions): there is no
    /// window in which the person could allow anything, so "Retry" can only
    /// fail the same way.
    NoAgent,
}

/// What a pkexec that exited before the helper answered says about this
/// machine: `Some` when no dialog could have succeeded.
///
/// pkexec(1) exits 127 both for "not authorized" and for "could not ask at
/// all", so the exit code alone is not enough: only its own stderr line
/// ("No authentication agent found") tells the second case apart.
pub fn pkexec_unavailable(exit: Option<i32>, stderr: &str) -> Option<ElevationUnavailable> {
    (exit == Some(127) && stderr.contains("No authentication agent")).then_some(ElevationUnavailable::NoAgent)
}

impl std::fmt::Display for ElevationUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoPkexec => {
                "не найден pkexec (polkit) — установите пакет pkexec или policykit-1 либо запустите приложение от root"
            }
            Self::FuseMount => {
                "приложение запущено из FUSE-монтирования, откуда root не может запустить файл — установите пакет .deb или запустите сам файл AppImage"
            }
            Self::NoAgent => {
                "в сеансе нет агента авторизации polkit, окну пароля негде появиться — запустите агент (например, polkit-gnome или lxpolkit) или войдите в полноценный рабочий стол"
            }
        })
    }
}

impl std::error::Error for ElevationUnavailable {}

/// Does `err`, anywhere in its chain, say that this machine cannot elevate?
pub fn is_elevation_unavailable(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<ElevationUnavailable>())
}

/// The AppImage's root-owned copy of the helper (helper/install.rs) could not
/// be made or cannot be trusted: a folder on the way that others may change,
/// a staged file that did not hash to its sum, a full disk. The person's
/// password was not the problem, so it is not a refusal either; the GUI shows
/// the "could not prepare its files" screen, and the log carries the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperSetupFailed(pub String);

impl std::fmt::Display for HelperSetupFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "помощник не скопирован в папку root: {}", self.0)
    }
}

impl std::error::Error for HelperSetupFailed {}

/// Does `err`, anywhere in its chain, carry a failed helper setup?
pub fn is_helper_setup_failure(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<HelperSetupFailed>())
}

/// What SteamOS needs from the person when its system dialog could not let
/// the helper start. Not `ElevationUnavailable`: both have a way out the
/// person can take, and the window says which (src/errors.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamOsAdvice {
    /// The dialog was dismissed, or the password was not accepted. On a Steam
    /// Deck the likely reason is that the deck user has no password: SteamOS
    /// ships it without one, and polkit cannot accept "nothing".
    SetPassword,
    /// No polkit agent: Gaming Mode, where no password window can appear.
    /// The one-time setup has to happen in Desktop Mode (helper/install.rs).
    UseDesktopMode,
}

impl std::fmt::Display for SteamOsAdvice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SetPassword => {
                "SteamOS не выдал права: пароль не введён или не принят; у пользователя deck пароля нет, пока его не задать (режим рабочего стола → Konsole → passwd)"
            }
            Self::UseDesktopMode => {
                "в игровом режиме SteamOS нет окна для пароля — подключитесь один раз из режима рабочего стола, после этого игровой режим подключается без пароля"
            }
        })
    }
}

impl std::error::Error for SteamOsAdvice {}

/// The SteamOS advice anywhere in `err`'s chain.
pub fn steamos_advice(err: &anyhow::Error) -> Option<SteamOsAdvice> {
    err.chain().find_map(|cause| cause.downcast_ref::<SteamOsAdvice>().copied())
}

/// What to tell a SteamOS user about a failed handshake, if anything
/// SteamOS-specific applies. A failed setup keeps its own screen.
pub fn steamos_advice_for(failure: HandshakeFailure) -> Option<SteamOsAdvice> {
    match failure {
        HandshakeFailure::Dismissed | HandshakeFailure::NotAuthorized => Some(SteamOsAdvice::SetPassword),
        HandshakeFailure::NoAgent => Some(SteamOsAdvice::UseDesktopMode),
        HandshakeFailure::SetupFailed(_) | HandshakeFailure::Other => None,
    }
}

/// Why a pkexec child died before the helper answered its first request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeFailure {
    /// 126: the dialog was dismissed.
    Dismissed,
    /// 127 with any other line: not authorized (a wrong password three
    /// times), or pkexec could not run the program.
    NotAuthorized,
    /// 127 with pkexec's "No authentication agent found".
    NoAgent,
    /// The AppImage's setup script stopped with this exit code; its own
    /// stderr line, in the log, says why.
    SetupFailed(i32),
    /// Still running, killed by a signal, or a code that means none of the
    /// above.
    Other,
}

/// Read a dead pkexec child: its exit code, the start of its stderr, and
/// whether it was running the AppImage's setup script.
///
/// The setup is checked first and by its name on stderr, not by the code: a
/// shell that cannot exec the installed helper (a /home mounted noexec) exits
/// 126 or 127 itself, and must not be read as the person's answer. Both the
/// shell's own messages and the script's start with `$0`, which is
/// `install::SETUP_NAME`.
pub fn handshake_failure(exit: Option<i32>, stderr: &str, setup: bool) -> HandshakeFailure {
    let Some(code) = exit else {
        return HandshakeFailure::Other;
    };
    if setup && stderr.contains(crate::helper::install::SETUP_NAME) {
        return HandshakeFailure::SetupFailed(code);
    }
    match code {
        126 => HandshakeFailure::Dismissed,
        127 if pkexec_unavailable(exit, stderr).is_some() => HandshakeFailure::NoAgent,
        127 => HandshakeFailure::NotAuthorized,
        // The script's own codes, should its line not have reached us.
        _ if setup => HandshakeFailure::SetupFailed(code),
        _ => HandshakeFailure::Other,
    }
}

/// Why we cannot gain privileges, or `None` when we can.
///
/// Linux only: it is the one platform where elevation happens at connect time
/// rather than at launch, so the reason has to be reportable mid-session.
/// Asked for the .deb and every other start that runs our own executable as
/// the helper; an AppImage never gets here (net/linux.rs recognises it first
/// and starts the root-owned copy, helper/install.rs), so the FUSE check below
/// is left for a FUSE mount we do not know.
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
/// from a `/tmp/.mount_` prefix: whatever the mount is, root cannot execute
/// from it.
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
        for reason in [
            ElevationUnavailable::NoPkexec,
            ElevationUnavailable::FuseMount,
            ElevationUnavailable::NoAgent,
        ] {
            let err = anyhow::Error::new(reason)
                .context("spawn the privileged helper")
                .context("preflight");
            assert!(is_elevation_unavailable(&err), "{reason:?}");
            assert!(!reason.to_string().is_empty());
        }
        let refused = anyhow::anyhow!("запрос прав отменён");
        assert!(!is_elevation_unavailable(&refused));
    }

    /// Exit 127 with pkexec's own "no agent" line is a machine that cannot
    /// ask; 127 without it (a refusal, a wrong password) and 126 (the dialog
    /// was dismissed) are the person's answer.
    #[test]
    fn only_a_missing_agent_makes_pkexec_unavailable() {
        let no_agent = "Error executing command as another user: No authentication agent found.";
        assert_eq!(pkexec_unavailable(Some(127), no_agent), Some(ElevationUnavailable::NoAgent));
        assert_eq!(pkexec_unavailable(Some(127), "Error executing command as another user: Not authorized"), None);
        assert_eq!(pkexec_unavailable(Some(126), no_agent), None);
        assert_eq!(pkexec_unavailable(None, no_agent), None);
        assert!(ElevationUnavailable::NoAgent.to_string().contains("polkit"));
    }

    /// A setup that failed is told apart from the person's answer by its
    /// name on stderr, whatever its code; without it, 126 and 127 are
    /// pkexec's own.
    #[test]
    fn a_failed_setup_is_not_read_as_a_refusal() {
        let shell = "proxysvpn-setup: line 25: /home/.proxysvpn/bin/proxysvpn-helper: Permission denied";
        assert_eq!(handshake_failure(Some(126), shell, true), HandshakeFailure::SetupFailed(126));
        let script = "proxysvpn-setup: tun2socks does not match its checksum";
        assert_eq!(handshake_failure(Some(65), script, true), HandshakeFailure::SetupFailed(65));
        assert_eq!(handshake_failure(Some(73), "", true), HandshakeFailure::SetupFailed(73));

        assert_eq!(handshake_failure(Some(126), "", true), HandshakeFailure::Dismissed);
        let no_agent = "Error executing command as another user: No authentication agent found.";
        assert_eq!(handshake_failure(Some(127), no_agent, true), HandshakeFailure::NoAgent);
        assert_eq!(handshake_failure(Some(127), "Not authorized", true), HandshakeFailure::NotAuthorized);

        // The installed helper, or the .deb's: no setup to blame.
        assert_eq!(handshake_failure(Some(126), "", false), HandshakeFailure::Dismissed);
        assert_eq!(handshake_failure(Some(127), no_agent, false), HandshakeFailure::NoAgent);
        assert_eq!(handshake_failure(Some(65), script, false), HandshakeFailure::Other);
        assert_eq!(handshake_failure(None, no_agent, true), HandshakeFailure::Other);
    }

    /// On SteamOS a refusal sends the person to `passwd` and a missing agent
    /// to Desktop Mode; the advice survives the context the layers add.
    #[test]
    fn steamos_turns_refusals_into_what_to_do() {
        assert_eq!(steamos_advice_for(HandshakeFailure::Dismissed), Some(SteamOsAdvice::SetPassword));
        assert_eq!(steamos_advice_for(HandshakeFailure::NotAuthorized), Some(SteamOsAdvice::SetPassword));
        assert_eq!(steamos_advice_for(HandshakeFailure::NoAgent), Some(SteamOsAdvice::UseDesktopMode));
        assert_eq!(steamos_advice_for(HandshakeFailure::SetupFailed(65)), None);
        assert_eq!(steamos_advice_for(HandshakeFailure::Other), None);

        for advice in [SteamOsAdvice::SetPassword, SteamOsAdvice::UseDesktopMode] {
            let err = anyhow::Error::new(advice).context("spawn the privileged helper").context("preflight");
            assert_eq!(steamos_advice(&err), Some(advice));
            assert!(!is_elevation_unavailable(&err));
        }
        assert!(SteamOsAdvice::SetPassword.to_string().contains("passwd"));
        assert!(SteamOsAdvice::UseDesktopMode.to_string().contains("рабочего стола"));
        assert_eq!(steamos_advice(&anyhow::anyhow!("запрос прав отменён")), None);
    }

    #[test]
    fn a_failed_setup_is_found_through_context_and_is_not_unavailability() {
        let err = anyhow::Error::new(HelperSetupFailed("/home is writable by others".to_string()))
            .context("spawn the privileged helper");
        assert!(is_helper_setup_failure(&err));
        assert!(!is_elevation_unavailable(&err));
        assert!(!is_helper_setup_failure(&anyhow::anyhow!("запрос прав отменён")));
    }

    /// The FUSE text sends the person to what works, and the pkexec one
    /// names both package spellings the .deb depends on.
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
