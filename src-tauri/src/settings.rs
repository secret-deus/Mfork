//! GUI app settings persisted in app-config.json.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TunSettings {
    pub enable: bool,
    /// gvisor | system | mixed
    pub stack: String,
    pub auto_route: bool,
    pub strict_route: bool,
    pub dns_hijack: bool,
    pub mtu: u32,
}

impl Default for TunSettings {
    fn default() -> Self {
        Self {
            enable: false,
            stack: "mixed".into(),
            auto_route: true,
            strict_route: true,
            dns_hijack: true,
            mtu: 1500,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsSettings {
    /// when false the profile's own dns block is left untouched
    pub enable: bool,
    /// fake-ip | redir-host
    pub enhanced_mode: String,
    pub nameserver: Vec<String>,
    pub fallback: Vec<String>,
}

impl Default for DnsSettings {
    fn default() -> Self {
        Self {
            enable: false,
            enhanced_mode: "fake-ip".into(),
            nameserver: vec!["https://223.5.5.5/dns-query".into(), "https://doh.pub/dns-query".into()],
            fallback: vec!["https://1.1.1.1/dns-query".into(), "https://8.8.8.8/dns-query".into()],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub system_proxy: bool,
    pub proxy_bypass: String,
    pub silent_start: bool,
    pub tun: TunSettings,
    pub dns: DnsSettings,
}

pub fn default_bypass() -> String {
    if cfg!(windows) {
        "localhost;127.*;192.168.*;10.*;172.16.*;<local>".into()
    } else {
        "localhost,127.0.0.1/8,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12".into()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            system_proxy: false,
            proxy_bypass: default_bypass(),
            silent_start: false,
            tun: TunSettings::default(),
            dns: DnsSettings::default(),
        }
    }
}

fn path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("app-config.json"))
}

pub fn load(app: &AppHandle) -> AppSettings {
    path(app)
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, s: &AppSettings) -> Result<(), String> {
    let p = path(app).ok_or("no data dir")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(p, serde_json::to_string_pretty(s).unwrap()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn settings_get(app: AppHandle) -> AppSettings {
    load(&app)
}

/// Persist settings; re-apply the OS proxy so a changed bypass list (or a
/// toggled system_proxy flag) takes effect immediately.
#[tauri::command]
pub fn settings_set(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    save(&app, &settings)?;
    let r = crate::system::sync_system_proxy(&app);
    crate::tray::refresh(&app);
    r
}
