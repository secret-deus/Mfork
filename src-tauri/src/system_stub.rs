//! Mobile stand-in for `system.rs`: Android/iOS have no user-settable OS
//! proxy and no launch-at-login. Traffic capture goes through the VPN
//! service instead (see docs/ANDROID.md), so these are inert.

use tauri::AppHandle;

pub fn sync_system_proxy(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

pub fn clear_system_proxy() {}

pub fn clear_if_enabled(_app: &AppHandle) {}

pub fn set_system_proxy_inner(_app: &AppHandle, _enable: bool) -> Result<(), String> {
    Err("此平台没有系统代理设置".into())
}

#[tauri::command]
pub fn system_proxy_set(app: AppHandle, enable: bool) -> Result<(), String> {
    set_system_proxy_inner(&app, enable)
}

#[tauri::command]
pub fn autostart_get(_app: AppHandle) -> bool {
    false
}

#[tauri::command]
pub fn autostart_set(_app: AppHandle, _enable: bool) -> Result<(), String> {
    Err("此平台不支持开机自启".into())
}
