//! System integration: OS proxy (registry / networksetup via sysproxy)
//! and launch-at-login (tauri-plugin-autostart).

use sysproxy::Sysproxy;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::core::CoreState;

/// Make the OS proxy match settings + core state: point it at the current
/// mixed port when enabled, disable it otherwise. When the core is not
/// running yet, an enabled proxy is applied on the next core start.
pub fn sync_system_proxy(app: &AppHandle) -> Result<(), String> {
    let s = crate::settings::load(app);
    if s.system_proxy {
        let api = app.state::<CoreState>().api.lock().unwrap().clone();
        let Some(api) = api else { return Ok(()) };
        Sysproxy {
            enable: true,
            host: "127.0.0.1".into(),
            port: api.mixed_port,
            bypass: s.proxy_bypass.clone(),
        }
        .set_system_proxy()
        .map_err(|e| format!("设置系统代理失败: {e}"))
    } else {
        clear_system_proxy();
        Ok(())
    }
}

/// Turn the OS proxy off (keeps host/port so the user's manual config survives).
pub fn clear_system_proxy() {
    if let Ok(mut cur) = Sysproxy::get_system_proxy() {
        if cur.enable {
            cur.enable = false;
            let _ = cur.set_system_proxy();
        }
    }
}

/// Clear the OS proxy only when we are the ones who set it.
pub fn clear_if_enabled(app: &AppHandle) {
    if crate::settings::load(app).system_proxy {
        clear_system_proxy();
    }
}

pub fn set_system_proxy_inner(app: &AppHandle, enable: bool) -> Result<(), String> {
    let _guard = crate::settings::data_lock();
    let mut s = crate::settings::load(app);
    s.system_proxy = enable;
    crate::settings::save(app, &s)?;
    let r = sync_system_proxy(app);
    crate::tray::refresh(app);
    let _ = app.emit("sys-proxy-changed", enable);
    r
}

#[tauri::command]
pub fn system_proxy_set(app: AppHandle, enable: bool) -> Result<(), String> {
    set_system_proxy_inner(&app, enable)
}

#[tauri::command]
pub fn autostart_get(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn autostart_set(app: AppHandle, enable: bool) -> Result<(), String> {
    let m = app.autolaunch();
    if enable { m.enable() } else { m.disable() }.map_err(|e| e.to_string())
}
