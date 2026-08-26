mod core;
mod profile;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(core::CoreState::default())
        .invoke_handler(tauri::generate_handler![
            core::core_start,
            core::core_stop,
            core::core_status,
            profile::profiles_list,
            profile::profile_import,
            profile::profile_update,
            profile::profile_delete,
            profile::profile_select
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                core::kill_on_exit(app);
            }
        });
}

