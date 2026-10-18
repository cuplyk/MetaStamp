// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::load_files,
            commands::apply_changes,
            commands::capabilities,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MetaStamp");
}
