mod core;
mod override_;
mod profile;
mod settings;
mod system;
mod tray;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(core::CoreState::default())
        .manage(tray::TrayState::default())
        .setup(|app| {
            tray::create_tray(app.handle())?;
            // the window is created hidden; silent start keeps it in the tray
            if !settings::load(app.handle()).silent_start {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                }
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(600));
                profile::auto_update_tick(&handle);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // closing the main window hides to tray; quit lives in the tray menu
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            core::core_start,
            core::core_stop,
            core::core_status,
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
            tray::tray_sync
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
