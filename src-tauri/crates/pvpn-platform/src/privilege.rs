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
// The cost is real and documented in the PR: the WebView runs elevated too,
// and tauri.conf.json still has `"csp": null`. A split helper
// (unprivileged GUI + small elevated service) is the next step for both
// Windows and Linux, and `net`'s contract is narrow enough to move behind an
// IPC boundary later without touching callers.

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

    #[cfg(unix)]
    #[test]
    fn elevation_matches_uid() {
        // SAFETY: see is_elevated.
        let uid = unsafe { libc::getuid() };
        assert_eq!(is_elevated(), uid == 0);
    }
}
