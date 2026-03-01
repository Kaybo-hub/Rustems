// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod cmd;

use audio::StemEngine;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(StemEngine::new()))
        .invoke_handler(tauri::generate_handler![
            cmd::load_song,
            cmd::play,
            cmd::pause,
            cmd::set_stem_volume,
            cmd::print_audio_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}