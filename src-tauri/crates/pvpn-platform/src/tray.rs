// src-tauri/crates/pvpn-platform/src/tray.rs
//
// Will a tray icon actually be seen?
//
// The window hides into the tray on close, and the VPN keeps running. That is
// only safe where the tray icon is visible: hidden with no icon, the window
// can never be brought back and the tunnel never switched off — short of a
// second launch, which the single-instance plugin turns into "show the window".
//
// macOS and Windows always draw the icon. Linux draws it only when a
// StatusNotifier host runs: KDE, Cinnamon, XFCE and others have one, stock
// GNOME (Debian, Fedora) does not, and libappindicator builds the icon there
// without complaint and shows nothing. So on Linux the answer is asked of the
// session bus: does anyone own org.kde.StatusNotifierWatcher right now? It is
// asked at the moment of closing, not once at start, because a tray extension
// can be switched on while the app runs.

/// True when a tray icon built now would be visible.
pub fn host_present() -> bool {
    #[cfg(target_os = "linux")]
    {
        status_notifier_watcher_present()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// The well-known name of the StatusNotifier watcher. KDE's spelling is the
/// one every implementation registers, GNOME extensions and XFCE included.
#[cfg(target_os = "linux")]
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Any failure — no session bus, a refused call — answers "no host": the
/// caller then keeps the window reachable instead of hiding it.
#[cfg(target_os = "linux")]
fn status_notifier_watcher_present() -> bool {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        crate::log::warn("tray", "no session bus: treating the tray as absent");
        return false;
    };
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&connection) else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(WATCHER) else {
        return false;
    };
    bus.name_has_owner(name).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn macos_and_windows_always_have_a_tray() {
        assert!(super::host_present());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_watcher_name_is_a_valid_bus_name() {
        assert!(zbus::names::BusName::try_from(super::WATCHER).is_ok());
    }
}
