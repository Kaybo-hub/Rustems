// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod cmd;
mod usb;
mod splitter;

use audio::StemEngine;
use usb::DeviceState;
use std::sync::Mutex;
use tauri::Manager;

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
            usb::list_usb_devices,
            usb::connect_usb_device,
            usb::upload_stems,
            usb::list_device_tracks,
            usb::delete_track,
            usb::delete_album,
            usb::disconnect_device,
            usb::get_storage_info,
            splitter::check_splitter,
            splitter::split_stems,
            splitter::export_stems
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
                    let state = app_handle.state::<DeviceState>();
                    let mut guard = state.0.lock().unwrap();
                    if let Some(handle) = guard.take() {
                        crate::usb::do_disconnect(handle);
                    }
                }
                _ => {}
            }
        });
}