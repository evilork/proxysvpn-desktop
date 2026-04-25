// src-tauri/src/lib.rs
mod ping;
mod subscription;
mod tun;
mod xray_manager;

mod logger;
use std::sync::Arc;
use subscription::{build_xray_config, fetch_subscription};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use tauri::{Manager, RunEvent, WindowEvent};
use tun::{new_state as new_tun_state, SharedTunState};
use xray_manager::{new_state as new_xray_state, SharedXrayState};

struct VpnState {
    xray: SharedXrayState,
    tun: SharedTunState,
}

#[derive(serde::Serialize)]
struct ConnectResult {
    ok: bool,
    remark: String,
    host: String,
    port: u16,
}

const PID_FILE: &str = "/tmp/proxysvpn-desktop.pid";

fn sync_cleanup() {
    use std::process::Command;

    let _ = Command::new("/usr/bin/pkill").args(["-9", "-x", "tun2socks"]).status();
    let _ = Command::new("/usr/bin/pkill").args(["-9", "-x", "xray"]).status();
    let _ = Command::new("/sbin/route")
        .args(["-n", "delete", "-net", "0.0.0.0/1"])
        .status();
    let _ = Command::new("/sbin/route")
        .args(["-n", "delete", "-net", "128.0.0.0/1"])
        .status();
    let _ = Command::new("/sbin/ifconfig").args([tun::TUN_NAME, "down"]).status();

    if let Ok(contents) = std::fs::read_to_string(PID_FILE) {
        for line in contents.lines() {
            if let Some(ip) = line.strip_prefix("server_ip=") {
                let _ = Command::new("/sbin/route")
                    .args(["-n", "delete", "-host", ip])
                    .status();
            }
        }
        let _ = std::fs::remove_file(PID_FILE);
    }
}

fn write_pid_file(server_ip: &str) {
    let my_pid = std::process::id();
    let contents = format!("pid={}\nserver_ip={}\n", my_pid, server_ip);
    let _ = std::fs::write(PID_FILE, contents);
}

fn remove_pid_file() {
    let _ = std::fs::remove_file(PID_FILE);
}

#[tauri::command]
async fn vpn_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<VpnState>>,
    sub_url: String,
) -> Result<ConnectResult, String> {
    let _ = tun::stop(&state.tun).await;
    let _ = xray_manager::stop(&state.xray).await;

    let cfg = fetch_subscription(&sub_url)
        .await
        .map_err(|e| format!("subscription: {}", e))?;

    let xray_cfg = build_xray_config(&cfg);
    let (xray_bin, assets_dir) =
        xray_manager::xray_paths(&app).map_err(|e| e.to_string())?;
    xray_manager::start(&state.xray, &xray_bin, &assets_dir, xray_cfg)
        .await
        .map_err(|e| format!("xray start: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    if let Err(e) = tun::start(&state.tun, &app, &cfg.host).await {
        let _ = xray_manager::stop(&state.xray).await;
        return Err(format!("tun start: {}", e));
    }

    if let Some(ip) = tun::get_server_ip(&state.tun).await {
        write_pid_file(&ip);
    }

    ping::set_target(cfg.host.clone(), cfg.port);

    Ok(ConnectResult {
        ok: true,
        remark: cfg.remark,
        host: cfg.host,
        port: cfg.port,
    })
}

#[tauri::command]
async fn vpn_disconnect(state: tauri::State<'_, Arc<VpnState>>) -> Result<(), String> {
    ping::clear_target();
    let _ = tun::stop(&state.tun).await;
    xray_manager::stop(&state.xray).await.map_err(|e| e.to_string())?;
    remove_pid_file();
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

/// Tray icon in the macOS menu bar with Show/Disconnect/Quit actions.
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

    let _tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("ProxysVPN")
        .icon(app.default_window_icon().unwrap().clone())
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
                        remove_pid_file();
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

    sync_cleanup();

    let vpn_state = Arc::new(VpnState {
        xray: new_xray_state(),
        tun: new_tun_state(),
    });

    let app = tauri::Builder::default()
        .manage(vpn_state)
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            build_tray(app.handle())?;
            install_signal_handlers();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Red-X / Cmd+W — hide to tray, VPN keeps running.
                // Use tray → Quit or Cmd+Q to fully exit.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            vpn_connect,
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
