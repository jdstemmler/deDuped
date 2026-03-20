//! Application entry point. Registers Tauri plugins and exposes commands to the frontend.

mod actionlog;
mod cache;
mod commands;
mod fileops;
mod hasher;
mod perceptual;

#[cfg(test)]
mod tests;

use commands::{cancel_scan, execute_action, export_report, get_action_log, get_file_preview, scan_folders, undo_last_action};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![scan_folders, execute_action, export_report, get_file_preview, get_action_log, undo_last_action, cancel_scan])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
