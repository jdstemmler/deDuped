mod cache;
mod commands;
mod fileops;
mod hasher;

#[cfg(test)]
mod tests;

use commands::{execute_action, scan_folders};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![scan_folders, execute_action])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
