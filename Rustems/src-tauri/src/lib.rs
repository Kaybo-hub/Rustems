// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod cmd;
mod stem_player;

use audio::StemEngine;
use stem_player::DeviceState;
use std::sync::Mutex;
use tauri::Manager;

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
            stem_player::upload_stems,
            stem_player::list_device_tracks,
            stem_player::delete_track,
            stem_player::delete_album,
            stem_player::disconnect_device,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                    let state = app_handle.state::<DeviceState>();
                    let mut guard = state.0.lock().unwrap();
                    if let Some(handle) = guard.take() {
                        crate::stem_player::do_disconnect(handle);
                    }
                }
                _ => {}
            }
        });
}