#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod state;
mod commands;

use state::AppState;
use tauri::{DragDropEvent, Emitter, Manager, WindowEvent};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
struct DragPositionPayload {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Serialize)]
struct DragFilesPayload {
    paths: Vec<String>,
    x: f64,
    y: f64,
}

fn main() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::load_settings,
            commands::save_settings,
            commands::pick_folder,
            commands::get_default_zip_name,
            commands::validate_archive_name,
            commands::cancel_operation,
            commands::set_always_on_top,
            commands::is_running_as_admin,
            commands::restart_as_admin,
            commands::start_zip_batch,
            commands::start_unzip_batch,
            commands::exit_app,
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();

            // Set window icon from default app icon resource
            if let Some(icon) = app.default_window_icon() {
                let _ = window.set_icon(icon.clone());
            }

            // Apply saved Always on Top
            let always_on_top = app.state::<AppState>().settings.lock().unwrap().always_on_top;
            if always_on_top {
                let _ = window.set_always_on_top(true);
            }

            // Windows 11/10 DWM Immersive Dark Mode & Caption Color (#161616)
            #[cfg(target_os = "windows")]
            {
                if let Ok(hwnd) = window.hwnd() {
                    use windows::Win32::Foundation::HWND;
                    use windows::Win32::Graphics::Dwm::{
                        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
                    };
                    use windows::Win32::UI::WindowsAndMessaging::{
                        GetWindowLongW, SetWindowLongW, GWL_STYLE, WS_MAXIMIZEBOX,
                    };
                    let hwnd = HWND(hwnd.0 as _);
                    let dark_mode: i32 = 1;
                    let caption_color: u32 = 0x00161616; // COLORREF 0x00BBGGRR matching #161616
                    unsafe {
                        let current_style = GetWindowLongW(hwnd, GWL_STYLE);
                        let _ = SetWindowLongW(hwnd, GWL_STYLE, current_style & !(WS_MAXIMIZEBOX.0 as i32));

                        let _ = DwmSetWindowAttribute(
                            hwnd,
                            DWMWA_USE_IMMERSIVE_DARK_MODE,
                            &dark_mode as *const _ as _,
                            std::mem::size_of::<i32>() as u32,
                        );
                        let _ = DwmSetWindowAttribute(
                            hwnd,
                            DWMWA_CAPTION_COLOR,
                            &caption_color as *const _ as _,
                            std::mem::size_of::<u32>() as u32,
                        );
                    }
                }
            }

            // Route Windows Drag and Drop events to WebView with logical coordinates
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::DragDrop(ref dd) = event {
                    let scale = window_clone.scale_factor().unwrap_or(1.0);
                    match dd {
                        DragDropEvent::Enter { paths, position } => {
                            let str_paths: Vec<String> = paths
                                .iter()
                                .map(|p: &PathBuf| p.to_string_lossy().to_string())
                                .collect();
                            let _ = window_clone.emit(
                                "window-drag-enter",
                                DragFilesPayload {
                                    paths: str_paths,
                                    x: position.x / scale,
                                    y: position.y / scale,
                                },
                            );
                        }
                        DragDropEvent::Over { position } => {
                            let _ = window_clone.emit(
                                "window-drag-over",
                                DragPositionPayload {
                                    x: position.x / scale,
                                    y: position.y / scale,
                                },
                            );
                        }
                        DragDropEvent::Drop { paths, position } => {
                            let str_paths: Vec<String> = paths
                                .iter()
                                .map(|p: &PathBuf| p.to_string_lossy().to_string())
                                .collect();
                            let _ = window_clone.emit(
                                "window-drag-drop",
                                DragFilesPayload {
                                    paths: str_paths,
                                    x: position.x / scale,
                                    y: position.y / scale,
                                },
                            );
                        }
                        DragDropEvent::Leave => {
                            let _ = window_clone.emit("window-drag-leave", ());
                        }
                        _ => {}
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
