// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Routes CEF helper subprocesses (renderer, GPU, etc.) to the helper entry point
// before the app initializes; only the browser process runs `run()`.
#[tauri::cef_entry_point]
fn main() {
    spreadsheet_app_lib::run()
}
