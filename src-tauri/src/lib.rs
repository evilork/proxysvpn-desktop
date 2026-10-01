// src-tauri/src/lib.rs
mod ping;
mod subscription;
mod tun;
mod xray_manager;
mod hysteria_manager;

mod logger;
use pvpn_platform::net;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use subscription::{build_xray_config, ServerInfo};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(target_os = "macos")]
use tauri::menu::Submenu;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, RunEvent, WindowEvent};
use tun::{new_state as new_tun_state, SharedTunState};
use xray_manager::{new_state as new_xray_state, SharedXrayState};
use hysteria_manager::{new_state as new_hysteria_state, SharedHysteriaState, HY2_SOCKS_PORT};
use subscription::{fetch_all_servers, ServerConfig};

struct VpnState {
    xray: SharedXrayState,
    hysteria: SharedHysteriaState,
    tun: SharedTunState,
}

#[derive(serde::Serialize)]
struct ConnectResult {
    ok: bool,
    remark: String,
    host: String,
    port: u16,
}

/// Where the node address of a live tunnel is recorded, so that a run which
/// follows a crash can delete a host route it did not install itself.
/// macOS keeps the pre-split path (/tmp/proxysvpn-desktop.pid).
fn route_hint_path() -> Option<PathBuf> {
    match pvpn_platform::paths::route_hint_file() {
        Ok(path) => Some(path),
        Err(e) => {
            logger::log("warn", "app", &format!("no route hint location: {}", e));
            None
        }
    }
}

/// Blocking teardown for the startup and exit paths, where there is no async
/// runtime to await on. Removes our routes, kills leftover engines, and clears
/// the host route recorded by a previous run.
fn sync_cleanup() {
    let stale: Vec<Ipv4Addr> = route_hint_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|c| net::parse_route_hint(&c))
        .unwrap_or_default();

    net::sync_cleanup(&stale);

    if let Some(path) = route_hint_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn write_route_hint(server_ip: Ipv4Addr) {
    if let Some(path) = route_hint_path() {
        let contents = net::format_route_hint(std::process::id(), server_ip);
        if let Err(e) = std::fs::write(&path, contents) {
            logger::log("warn", "app", &format!("could not write route hint: {}", e));
        }
    }
}

fn remove_route_hint() {
    if let Some(path) = route_hint_path() {
        let _ = std::fs::remove_file(path);
    }
}

#[tauri::command]
async fn vpn_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<VpnState>>,
    sub_url: String,
    server_index: Option<usize>,
) -> Result<ConnectResult, String> {
    let _ = tun::stop(&state.tun).await;
    let _ = xray_manager::stop(&state.xray).await;
    let _ = hysteria_manager::stop(&state.hysteria).await;

    let servers = fetch_all_servers(&sub_url)
        .await
        .map_err(|e| format!("subscription: {}", e))?;
    let idx = server_index.unwrap_or(0).min(servers.len().saturating_sub(1));
    let server = servers[idx].clone();

    let host = server.host().to_string();
    let port = server.port();
    let remark = server.remark().to_string();

    // Выбор движка по протоколу: VLESS -> xray:10808, Hy2 -> hysteria:10809.
    let socks_port = match &server {
        ServerConfig::Vless(cfg) => {
            let xray_cfg = build_xray_config(cfg);
            let (xray_bin, assets_dir) =
                xray_manager::xray_paths(&app).map_err(|e| e.to_string())?;
            xray_manager::start(&state.xray, &xray_bin, &assets_dir, xray_cfg)
                .await
                .map_err(|e| format!("xray start: {}", e))?;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            tun::SOCKS_PORT
        }
        ServerConfig::Hy2(cfg) => {
            hysteria_manager::start(&state.hysteria, &app, cfg)
                .await
                .map_err(|e| format!("hysteria start: {}", e))?;
            HY2_SOCKS_PORT
        }
    };

    if let Err(e) = tun::start(&state.tun, &app, &host, socks_port).await {
        let _ = xray_manager::stop(&state.xray).await;
        let _ = hysteria_manager::stop(&state.hysteria).await;
        return Err(format!("tun start: {}", e));
    }

    if let Some(ip) = tun::get_server_ip(&state.tun).await {
        write_route_hint(ip);
    }

    ping::set_target(host.clone(), port);

    Ok(ConnectResult {
        ok: true,
        remark,
        host,
        port,
    })
}

#[tauri::command]
async fn list_servers(sub_url: String) -> Result<Vec<ServerInfo>, String> {
    let servers = fetch_all_servers(&sub_url)
        .await
        .map_err(|e| format!("subscription: {}", e))?;
    Ok(servers
        .iter()
        .enumerate()
        .map(|(i, c)| ServerInfo {
            index: i,
            remark: c.remark().to_string(),
            host: c.host().to_string(),
            port: c.port(),
            proto: c.proto().to_string(),
        })
        .collect())
}

#[tauri::command]
async fn vpn_disconnect(state: tauri::State<'_, Arc<VpnState>>) -> Result<(), String> {
    ping::clear_target();
    let _ = tun::stop(&state.tun).await;
    let _ = xray_manager::stop(&state.xray).await;
    let _ = hysteria_manager::stop(&state.hysteria).await;
    remove_route_hint();
    Ok(())
}

#[tauri::command]
async fn vpn_status(state: tauri::State<'_, Arc<VpnState>>) -> Result<bool, String> {
    let xray_up = xray_manager::is_running(&state.xray).await;
    let tun_up = tun::is_running(&state.tun).await;
    Ok(xray_up && tun_up)
}

#[tauri::command]
async fn vpn_ping() -> Result<u32, String> {
    ping::tcp_ping_async().await.map_err(|e| e.to_string())
}

/// Leaves the routing table clean when the process is asked to die outside the
/// normal UI path.
///
/// Unix keeps the pre-split behaviour (TERM/INT/HUP). Windows has no signals:
/// Ctrl+C is the only equivalent a GUI process can observe, and a logoff or
/// shutdown kills us without a usable notification. That case is covered
/// instead by the platform layer — every Windows route is written with
/// `store=active`, so it does not survive a reboot, the Wintun adapter dies
/// with tun2socks and takes its own routes with it, and the host route left
/// behind by a logoff is cleared from the route hint on the next start.
#[cfg(unix)]
fn install_signal_handlers() {
    tauri::async_runtime::spawn(async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s, Err(_) => return,
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s, Err(_) => return,
        };
        let mut hup = match signal(SignalKind::hangup()) {
            Ok(s) => s, Err(_) => return,
        };

        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
            _ = hup.recv() => {}
        }
        sync_cleanup();
        std::process::exit(0);
    });
}

#[cfg(windows)]
fn install_signal_handlers() {
    tauri::async_runtime::spawn(async {
        if tokio::signal::ctrl_c().await.is_err() {
            return;
        }
        sync_cleanup();
        std::process::exit(0);
    });
}

/// macOS application menu.
///
/// Only macOS: `hide_others` / `show_all` are Cocoa concepts, and on Windows
/// and Linux `set_menu` would draw a menu bar inside the 480x720 fixed window,
/// which is not part of the design. Copy/paste still work there through the
/// WebView's own accelerators.
#[cfg(target_os = "macos")]
fn build_menu(handle: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let app_submenu = Submenu::with_items(
        handle, "ProxysVPN", true,
        &[
            &PredefinedMenuItem::about(handle, Some("About ProxysVPN"), None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::hide(handle, None)?,
            &PredefinedMenuItem::hide_others(handle, None)?,
            &PredefinedMenuItem::show_all(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::quit(handle, None)?,
        ],
    )?;

    let edit_submenu = Submenu::with_items(
        handle, "Edit", true,
        &[
            &PredefinedMenuItem::undo(handle, None)?,
            &PredefinedMenuItem::redo(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::cut(handle, None)?,
            &PredefinedMenuItem::copy(handle, None)?,
            &PredefinedMenuItem::paste(handle, None)?,
            &PredefinedMenuItem::select_all(handle, None)?,
        ],
    )?;

    let window_submenu = Submenu::with_items(
        handle, "Window", true,
        &[
            &PredefinedMenuItem::minimize(handle, None)?,
            &PredefinedMenuItem::close_window(handle, None)?,
        ],
    )?;

    Menu::with_items(handle, &[&app_submenu, &edit_submenu, &window_submenu])
}

/// Tray icon (macOS menu bar, Windows notification area, Linux app indicator)
/// with Show/Disconnect/Quit actions.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Показать окно", true, None::<&str>)?;
    let disconnect_item = MenuItem::with_id(app, "disconnect", "Отключить VPN", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;

    let tray_menu = Menu::with_items(app, &[
        &show_item,
        &disconnect_item,
        &PredefinedMenuItem::separator(app)?,
        &quit_item,
    ])?;

    // No unwrap: a bundle whose icon list lost the PNG entries would otherwise
    // panic on startup instead of reporting a packaging problem.
    let icon = app
        .default_window_icon()
        .ok_or_else(|| {
            tauri::Error::Anyhow(anyhow::anyhow!(
                "bundle has no default window icon — cannot build the tray"
            ))
        })?
        .clone();

    let _tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("ProxysVPN")
        .icon(icon)
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event: MenuEvent| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "disconnect" => {
                if let Some(state) = app.try_state::<Arc<VpnState>>() {
                    let state = state.inner().clone();
                    tauri::async_runtime::spawn(async move {
                        ping::clear_target();
                        let _ = tun::stop(&state.tun).await;
                        let _ = xray_manager::stop(&state.xray).await;
                        // Hy2 sessions have no xray: without this the hysteria
                        // sidecar kept running and held its SOCKS port.
                        let _ = hysteria_manager::stop(&state.hysteria).await;
                        remove_route_hint();
                    });
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logger::init();
    // Route the platform layer's messages into the same ring buffer the
    // support UI reads; without this they would only reach stdout.
    pvpn_platform::log::set_sink(logger::log);

    sync_cleanup();

    let vpn_state = Arc::new(VpnState {
        xray: new_xray_state(),
        hysteria: new_hysteria_state(),
        tun: new_tun_state(),
    });

    let app = tauri::Builder::default()
        .manage(vpn_state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                let menu = build_menu(app.handle())?;
                app.set_menu(menu)?;
            }
            build_tray(app.handle())?;
            install_signal_handlers();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Close button — hide to the tray, the VPN keeps running.
                // Use tray → Quit (Cmd+Q on macOS) to exit for real.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            vpn_connect,
            list_servers,
            vpn_disconnect,
            vpn_status,
            vpn_ping,
            get_logs,
            clear_logs,
            export_logs,
            get_log_file_path
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_handle, event| {
        if let RunEvent::Exit = event {
            sync_cleanup();
        }
    });
}


// === auto-injected logger commands ===

#[tauri::command]
fn get_logs(limit: Option<usize>) -> Vec<logger::LogLine> {
    logger::snapshot(limit)
}

#[tauri::command]
fn clear_logs() {
    logger::clear();
}

#[tauri::command]
fn export_logs(include_system_info: bool) -> String {
    logger::export_text(include_system_info)
}

#[tauri::command]
fn get_log_file_path() -> Option<String> {
    logger::log_file_path()
}
