#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod components;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(backend::DownloadState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            backend::get_settings,
            backend::install_tools,
            backend::check_tools_updates,
            backend::choose_directory,
            backend::open_directory,
            backend::inspect_video,
            backend::download_video,
            backend::pause_task,
            backend::cancel_task
        ])
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                backend::shutdown(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("无法启动桌面应用");
}
