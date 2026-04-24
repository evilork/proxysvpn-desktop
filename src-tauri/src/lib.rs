// src-tauri/src/lib.rs
mod ping;
mod subscription;
mod system_proxy;
mod xray_manager;

use subscription::{build_xray_config, fetch_subscription};
use xray_manager::{new_state, SharedXrayState};

#[derive(serde::Serialize)]
struct ConnectResult {
    ok: bool,
    remark: String,
    host: String,
    port: u16,
    socks_port: u16,
    http_port: u16,
}

#[tauri::command]
async fn vpn_connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedXrayState>,
    sub_url: String,
) -> Result<ConnectResult, String> {
    let _ = system_proxy::disable_socks().await;
    xray_manager::stop(&state).await.map_err(|e| e.to_string())?;

    let cfg = fetch_subscription(&sub_url)
        .await
        .map_err(|e| format!("subscription: {}", e))?;

    let xray_cfg = build_xray_config(&cfg);
    let (xray_bin, assets_dir) = xray_manager::xray_paths(&app).map_err(|e| e.to_string())?;

    xray_manager::start(&state, &xray_bin, &assets_dir, xray_cfg)
        .await
        .map_err(|e| format!("xray start: {}", e))?;

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    system_proxy::enable_socks("127.0.0.1", 10808)
        .await
        .map_err(|e| format!("system proxy: {}", e))?;

    // Configure the ping target to point at the VPN server itself.
    ping::set_target(cfg.host.clone(), cfg.port);

    Ok(ConnectResult {
        ok: true,
        remark: cfg.remark,
        host: cfg.host,
        port: cfg.port,
        socks_port: 10808,
        http_port: 10809,
    })
}

#[tauri::command]
async fn vpn_disconnect(state: tauri::State<'_, SharedXrayState>) -> Result<(), String> {
    let _ = system_proxy::disable_socks().await;
    ping::clear_target();
    xray_manager::stop(&state).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn vpn_status(state: tauri::State<'_, SharedXrayState>) -> Result<bool, String> {
    Ok(xray_manager::is_running(&state).await)
}

#[tauri::command]
async fn vpn_ping() -> Result<u32, String> {
    ping::tcp_ping_async().await.map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(new_state())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            vpn_connect,
            vpn_disconnect,
            vpn_status,
            vpn_ping
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
