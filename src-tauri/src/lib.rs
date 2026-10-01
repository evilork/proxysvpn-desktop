// src-tauri/src/lib.rs
mod paths;
mod ping;
mod privilege;
mod subscription;
mod tun;
mod xray_manager;
mod hysteria_manager;

mod logger;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use subscription::{build_xray_config, ServerInfo};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
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

/// Set once the tray icon exists. Without a tray there is nowhere to restore
/// the window from, so closing it must then really quit (matters on Linux
/// desktops without an AppIndicator host).
static TRAY_READY: AtomicBool = AtomicBool::new(false);

/// Crash recovery: leftover engines from a previous run keep the SOCKS ports
/// busy, and leftover routes break the network until they are removed. Runs at
/// startup, on signals and on exit, so it must be synchronous.
fn sync_cleanup() {
    privilege::kill_leftover_engines();
    tun::purge_stale();
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

/// The application menu is a macOS convention; on Linux it would render as an
/// in-window menu bar, which this 480x720 window does not want.
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

/// Tray icon (macOS menu bar, Linux AppIndicator) with Show/Disconnect/Quit.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let icon = app
        .default_window_icon()
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("no default window icon")))?
        .clone();
    let show_item = MenuItem::with_id(app, "show", "Показать окно", true, None::<&str>)?;
    let disconnect_item = MenuItem::with_id(app, "disconnect", "Отключить VPN", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;

    let tray_menu = Menu::with_items(app, &[
        &show_item,
        &disconnect_item,
        &PredefinedMenuItem::separator(app)?,
        &quit_item,
    ])?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("ProxysVPN")
        .icon(icon)
        .menu(&tray_menu)
        .menu_on_left_click(false)
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
                        let _ = hysteria_manager::stop(&state.hysteria).await;
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

    let _tray = tray;
    TRAY_READY.store(true, Ordering::Relaxed);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logger::init();

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
            if let Err(e) = build_tray(app.handle()) {
                logger::log("warn", "app", &format!("tray unavailable: {}", e));
            }
            #[cfg(unix)]
            install_signal_handlers();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Red-X / Cmd+W — hide to tray, VPN keeps running.
                // Use tray → Quit or Cmd+Q to fully exit. Without a tray the
                // window would be unreachable, so then let the close through.
                let recoverable = cfg!(target_os = "macos") || TRAY_READY.load(Ordering::Relaxed);
                if recoverable {
                    api.prevent_close();
                    let _ = window.hide();
                }
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
