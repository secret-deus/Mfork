//! Mobile stand-in for `tray.rs`: no system tray. The status notification
//! on Android is owned by the VPN service (docs/ANDROID.md), not by Tauri.

use tauri::AppHandle;

pub fn show_window(_app: &AppHandle) {}

pub fn set_locale(_app: &AppHandle, _locale: &str) {}

pub fn refresh(_app: &AppHandle) {}

#[tauri::command]
pub fn tray_sync(_app: AppHandle, _mode: Option<String>) {}
