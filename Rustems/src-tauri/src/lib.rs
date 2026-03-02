// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod cmd;
mod stem_player;

use audio::StemEngine;
use stem_player::DeviceState;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(StemEngine::new()))
        .manage(DeviceState(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            cmd::load_song,
            cmd::play,
            cmd::pause,
            cmd::set_stem_volume,
            cmd::print_audio_status,
            stem_player::list_usb_devices,
            stem_player::connect_usb_device,
            stem_player::set_led_color,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}