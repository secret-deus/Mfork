mod core;
mod override_;
mod profile;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(core::CoreState::default())
        .setup(|app| {
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
            override_::override_move
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                core::kill_on_exit(app);
            }
        });
}
