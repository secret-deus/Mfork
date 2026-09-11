//! Android VPN bridge: talks to the Kotlin `VpnPlugin` (gen/android) which
//! owns the `VpnService`. The service establishes the TUN interface and
//! hands its file descriptor back; core.rs then spawns mihomo with
//! `tun.file-descriptor` pointing at it (docs/ANDROID.md, decisions 2–4).

use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

pub struct Vpn<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("mfork-vpn")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.mfork.desktop", "VpnPlugin")?;
            app.manage(Vpn(handle));
            Ok(())
        })
        .build()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EstablishArgs<'a> {
    mtu: u32,
    ipv6: bool,
    session: &'a str,
}

#[derive(Deserialize)]
struct Granted {
    granted: bool,
}

#[derive(Deserialize)]
struct Established {
    fd: i32,
}

fn handle(app: &AppHandle) -> Result<tauri::State<'_, Vpn<tauri::Wry>>, String> {
    app.try_state::<Vpn<tauri::Wry>>()
        .ok_or_else(|| "VPN 插件未加载".to_string())
}

/// Show the system consent dialog if needed; true once granted. Blocks
/// until the user answers, so call it off the main thread.
pub fn prepare(app: &AppHandle) -> Result<bool, String> {
    handle(app)?
        .0
        .run_mobile_plugin::<Granted>("prepare", ())
        .map(|g| g.granted)
        .map_err(|e| format!("VPN 授权失败: {e}"))
}

/// Bring the TUN interface up and return its fd (owned by the service;
/// stays valid until `stop`).
pub fn establish(app: &AppHandle, mtu: u32, ipv6: bool, session: &str) -> Result<i32, String> {
    handle(app)?
        .0
        .run_mobile_plugin::<Established>("establish", EstablishArgs { mtu, ipv6, session })
        .map(|e| e.fd)
        .map_err(|e| format!("建立 VPN 失败: {e}"))
}

#[derive(Serialize)]
struct AttachArgs<'a> {
    port: u16,
    secret: &'a str,
}

/// Tell the service where the core's API is so the notification can show
/// live traffic. Best effort.
pub fn attach_core(app: &AppHandle, port: u16, secret: &str) {
    if let Ok(h) = handle(app) {
        let _ = h
            .0
            .run_mobile_plugin::<serde_json::Value>("attachCore", AttachArgs { port, secret });
    }
}

pub fn stop(app: &AppHandle) {
    if let Ok(h) = handle(app) {
        let _ = h.0.run_mobile_plugin::<serde_json::Value>("stop", ());
    }
}
