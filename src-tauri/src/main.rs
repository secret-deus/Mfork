// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = mfork_lib::js_worker_entry() {
        std::process::exit(code);
    }
    mfork_lib::run()
}
