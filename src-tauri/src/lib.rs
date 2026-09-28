mod backup;
mod config_limits;
#[cfg(desktop)]
mod js_worker;
mod core;
mod override_;
mod profile;
mod settings;
mod updater;
#[cfg(target_os = "android")]
mod vpn;

// Desktop integrations (elevated service, OS proxy + autostart, tray) have
// inert mobile stand-ins so the shared modules compile unchanged; see
// docs/ANDROID.md.
#[cfg(desktop)]
mod service;
#[cfg(mobile)]
#[path = "service_stub.rs"]
mod service;
#[cfg(desktop)]
mod system;
#[cfg(mobile)]
#[path = "system_stub.rs"]
mod system;
#[cfg(desktop)]
mod tray;
#[cfg(mobile)]
#[path = "tray_stub.rs"]
mod tray;

#[cfg(desktop)]
use tauri::Manager;

/// Checked by the binary before any Tauri/plugin initialization.
pub fn js_worker_entry() -> Option<i32> {
    #[cfg(desktop)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(js_worker::WORKER_FLAG)) {
        return Some(js_worker::run());
    }
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(target_os = "android")]
    {
        builder = builder.plugin(vpn::init());
    }
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                tray::show_window(app);
            }))
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init())
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))
            .manage(tray::TrayState::default())
            .on_window_event(|window, event| {
                // closing the main window hides to tray; quit lives in the tray menu
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            });
    }
    builder
        .manage(core::CoreState::default())
        .setup(|app| {
            backup::recover_pending(app.handle()).map_err(std::io::Error::other)?;
            #[cfg(desktop)]
            {
                tray::create_tray(app.handle())?;
                // the window is created hidden; silent start keeps it in the tray
                if !settings::load(app.handle()).silent_start {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                    }
                }
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(600));
                profile::auto_update_tick(&handle);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core::core_start,
            core::core_stop,
            core::core_status,
            core::core_log_tail,
            core::vpn_prepare,
            core::net_diagnose,
            profile::profiles_list,
            profile::profile_import,
            profile::profile_import_local,
            profile::profile_update,
            profile::profile_delete,
            profile::profile_select,
            profile::profile_set_interval,
            profile::config_regenerate,
            override_::overrides_list,
            override_::override_create,
            override_::override_read,
            override_::override_write,
            override_::override_delete,
            override_::override_toggle,
            override_::override_move,
            settings::settings_get,
            settings::settings_set,
            system::system_proxy_set,
            system::autostart_get,
            system::autostart_set,
            service::service_status,
            service::service_install,
            service::service_uninstall,
            tray::tray_sync,
            updater::core_version_info,
            updater::core_check_update,
            updater::core_upgrade,
            updater::core_revert,
            backup::backup_export_local,
            backup::backup_import_local,
            backup::webdav_check,
            backup::webdav_list,
            backup::webdav_backup,
            backup::webdav_restore,
            backup::webdav_delete
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                system::clear_if_enabled(app);
                core::kill_on_exit(app);
            }
        });
}
