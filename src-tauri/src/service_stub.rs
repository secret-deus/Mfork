//! Mobile stand-in for `service.rs`: there is no elevated helper on
//! Android — TUN comes from the VPN service and the core runs unprivileged
//! as our own child process (docs/ANDROID.md, decision 2). Every entry
//! point reports "not installed" so the shared code paths in core.rs and
//! updater.rs take their non-service branch.

use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

const NOT_HERE: &str = "此平台不需要提权服务";

pub fn ping(_app: &AppHandle) -> bool {
    false
}

pub fn core_running(_app: &AppHandle) -> bool {
    false
}

pub fn core_running_status(_app: &AppHandle) -> Option<bool> {
    Some(false)
}

pub fn start_core(_app: &AppHandle, _config: &str, _dir: &str) -> Result<(u32, Option<String>, bool), String> {
    Err(NOT_HERE.into())
}

pub fn stop_core(_app: &AppHandle) {}

pub fn installed(_app: &AppHandle) -> bool {
    false
}

pub fn repin(_app: &AppHandle, _core: &PathBuf) -> Result<(), String> {
    Err(NOT_HERE.into())
}

#[derive(Serialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub reachable: bool,
    pub core_running: bool,
}

#[tauri::command]
pub fn service_status(_app: AppHandle) -> ServiceStatus {
    ServiceStatus { installed: false, reachable: false, core_running: false }
}

#[tauri::command]
pub fn service_install(_app: AppHandle) -> Result<(), String> {
    Err(NOT_HERE.into())
}

#[tauri::command]
pub fn service_uninstall(_app: AppHandle) -> Result<(), String> {
    Err(NOT_HERE.into())
}
