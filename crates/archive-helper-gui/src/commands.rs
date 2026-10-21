use crate::state::{AppSettings, AppState};
use archive_helper_core::batch::{BatchCoordinator, BatchTask, BatchTaskResult};
use archive_helper_core::extract::UnzipOptions;
use archive_helper_core::fs::validate_archive_name as core_validate_archive_name;
use archive_helper_core::progress::{CancellationToken, OperationStage};
use archive_helper_core::scanner::compute_selection_root;
use archive_helper_core::zip::ZipOptions;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Serialize)]
pub struct QueueItemInit {
    pub id: String,
    pub name: String,
    pub path: String,
    pub total_bytes: u64,
    pub total_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchInitPayload {
    pub is_zip: bool,
    pub total_items: usize,
    pub grand_total_bytes: u64,
    pub grand_total_files: usize,
    pub items: Vec<QueueItemInit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemCompletedPayload {
    pub id: String,
    pub success: bool,
    pub bytes: u64,
    pub files: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressPayload {
    pub stage: String,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub files_processed: usize,
    pub total_files: usize,
    pub current_item_name: Option<String>,
    pub current_file_name: Option<String>,
    pub bytes_per_sec: f64,
    pub item_index: usize,
    pub total_items: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletedPayload {
    pub success: bool,
    pub is_zip: bool,
    pub total_bytes: u64,
    pub total_files: usize,
    pub items_completed: usize,
    pub total_items: usize,
    pub error_message: Option<String>,
}

fn quick_scan_zip(path: &Path) -> (u64, usize) {
    archive_helper_core::inspect_zip(path).unwrap_or((0, 0))
}

fn quick_scan_folder(path: &Path) -> (u64, usize) {
    if path.is_file() {
        if archive_helper_core::is_archive_helper_temp_file(path) {
            return (0, 0);
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        return (size, 1);
    }
    if let Ok(res) = archive_helper_core::scan_sources(&[path.to_path_buf()], false) {
        return (res.total_bytes, res.total_files);
    }
    (0, 0)
}

fn get_worker_count(fast: bool) -> usize {
    if !fast {
        1 // Strictly one-at-a-time for HDDs / normal sequential mode
    } else {
        // Automatically scale with CPU threads (up to 8 for optimal NVMe queue saturation)
        std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4)
    }
}

/// Natural alphanumeric comparison (e.g. "Pack 2" < "Pack 10", "682489" < "882538")
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    while let (Some(&ca), Some(&cb)) = (a_chars.peek(), b_chars.peek()) {
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let mut num_a: u64 = 0;
            while let Some(&d) = a_chars.peek() {
                if d.is_ascii_digit() {
                    num_a = num_a
                        .saturating_mul(10)
                        .saturating_add((d as u8 - b'0') as u64);
                    a_chars.next();
                } else {
                    break;
                }
            }
            let mut num_b: u64 = 0;
            while let Some(&d) = b_chars.peek() {
                if d.is_ascii_digit() {
                    num_b = num_b
                        .saturating_mul(10)
                        .saturating_add((d as u8 - b'0') as u64);
                    b_chars.next();
                } else {
                    break;
                }
            }
            if num_a != num_b {
                return num_a.cmp(&num_b);
            }
        } else {
            let ca_lower: Vec<char> = ca.to_lowercase().collect();
            let cb_lower: Vec<char> = cb.to_lowercase().collect();
            if ca_lower != cb_lower {
                return ca_lower.cmp(&cb_lower);
            }
            a_chars.next();
            b_chars.next();
        }
    }
    a.len().cmp(&b.len())
}

fn sort_paths_naturally(items: &mut [String]) {
    items.sort_by(|a, b| {
        let name_a = Path::new(a)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let name_b = Path::new(b)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        natural_cmp(&name_a, &name_b)
    });
}

#[tauri::command]
pub fn load_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: AppSettings) {
    settings.save();
    *state.settings.lock().unwrap() = settings;
}

#[tauri::command]
pub async fn pick_folder() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Choose Destination Folder")
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string())
    })
    .await
    .unwrap_or(None)
}

#[tauri::command]
pub fn get_default_zip_name(paths: Vec<String>) -> String {
    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    if let Some(root) = compute_selection_root(&path_bufs) {
        if let Some(name) = root.file_name() {
            let s = name.to_string_lossy().to_string();
            if core_validate_archive_name(&s).is_ok() {
                return s;
            }
        }
    }
    "Archive".to_string()
}

#[tauri::command]
pub fn validate_archive_name(name: String) -> bool {
    core_validate_archive_name(&name).is_ok()
}

#[tauri::command]
pub fn cancel_operation(state: State<'_, AppState>) {
    if let Some(token) = state.current_cancel_token.lock().unwrap().as_ref() {
        token.cancel();
    }
}

#[tauri::command]
pub fn set_always_on_top(app: AppHandle, state: State<'_, AppState>, enabled: bool) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(enabled);
        let mut settings = state.settings.lock().unwrap();
        settings.always_on_top = enabled;
        settings.save();
    }
}

#[tauri::command]
pub fn is_running_as_admin() -> bool {
    #[cfg(target_os = "windows")]
    {
        unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[tauri::command]
pub fn restart_as_admin(_app: AppHandle) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        if let Ok(current_exe) = std::env::current_exe() {
            let exe_wide: Vec<u16> = current_exe
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let op_wide: Vec<u16> = std::ffi::OsStr::new("runas")
                .encode_wide()
                .chain(Some(0))
                .collect();
            unsafe {
                let res = windows::Win32::UI::Shell::ShellExecuteW(
                    None,
                    windows::core::PCWSTR(op_wide.as_ptr()),
                    windows::core::PCWSTR(exe_wide.as_ptr()),
                    None,
                    None,
                    windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
                );
                if res.0 as usize > 32 {
                    std::process::exit(0);
                }
            }
        }
    }
}

#[tauri::command]
pub fn exit_app(app: AppHandle, state: State<'_, AppState>) {
    if let Some(token) = state.current_cancel_token.lock().unwrap().as_ref() {
        token.cancel();
    }
    app.exit(0);
}

#[tauri::command]
pub async fn start_zip_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    items: Vec<String>,
    destination: Option<String>,
    delete_original: bool,
    fast: bool,
    predict_ext: bool,
    keep_mtime: bool,
    single_zip_name: Option<String>,
) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }

    let cancel_token = std::sync::Arc::new(CancellationToken::new());
    *state.current_cancel_token.lock().unwrap() = Some(cancel_token.clone());

    let app_handle = app.clone();
    let dest_dir = destination.map(PathBuf::from);

    tokio::task::spawn_blocking(move || {
        let mut items = items;
        sort_paths_naturally(&mut items);

        let options = ZipOptions {
            delete_original,
            predict_ext,
            keep_mtime,
            destination_dir: dest_dir,
            buffer_size: if fast { 2 * 1024 * 1024 } else { 1024 * 1024 },
        };

        let init_items: Vec<QueueItemInit> = if let Some(ref custom_name) = single_zip_name {
            let mut total_b = 0u64;
            let mut total_f = 0usize;
            for p in &items {
                let (b, f) = quick_scan_folder(Path::new(p));
                total_b += b;
                total_f += f;
            }
            vec![QueueItemInit {
                id: "single".to_string(),
                name: format!("{}.zip", custom_name),
                path: custom_name.clone(),
                total_bytes: total_b,
                total_files: total_f,
            }]
        } else {
            items
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let path = PathBuf::from(p);
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let (b, f) = quick_scan_folder(&path);
                    QueueItemInit {
                        id: format!("item_{}", i),
                        name,
                        path: p.clone(),
                        total_bytes: b,
                        total_files: f,
                    }
                })
                .collect()
        };

        let mut grand_total_bytes = 0u64;
        let mut grand_total_files = 0usize;
        for it in &init_items {
            grand_total_bytes += it.total_bytes;
            grand_total_files += it.total_files;
        }

        let _ = app_handle.emit(
            "op-init",
            BatchInitPayload {
                is_zip: true,
                total_items: init_items.len(),
                grand_total_bytes,
                grand_total_files,
                items: init_items,
            },
        );

        let tasks: Vec<BatchTask> = if let Some(custom_name) = single_zip_name {
            vec![BatchTask::Zip {
                id: "single".to_string(),
                sources: items.iter().map(PathBuf::from).collect(),
                custom_name: Some(custom_name),
                options,
            }]
        } else {
            items
                .into_iter()
                .enumerate()
                .map(|(i, p)| BatchTask::Zip {
                    id: format!("item_{}", i),
                    sources: vec![PathBuf::from(p)],
                    custom_name: None,
                    options: options.clone(),
                })
                .collect()
        };

        let total_items = tasks.len();
        let coordinator = BatchCoordinator::new(get_worker_count(fast));
        let (progress_rx, result_rx) =
            coordinator.execute_batch(tasks, Some((*cancel_token).clone()));

        let completed_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let completed_count_clone = completed_count.clone();

        // Stream progress events to UI
        let app_handle_progress = app_handle.clone();
        let progress_thread = std::thread::spawn(move || {
            while let Ok(progress) = progress_rx.recv() {
                let stage_str = match progress.stage {
                    OperationStage::Scanning => "Scanning",
                    OperationStage::Archiving => "Archiving",
                    OperationStage::Extracting => "Extracting",
                    OperationStage::Finalizing => "Finalizing",
                    OperationStage::CleaningUp => "CleaningUp",
                    OperationStage::Completed => "Completed",
                    OperationStage::Failed => "Failed",
                    OperationStage::Cancelled => "Cancelled",
                };

                let _ = app_handle_progress.emit(
                    "op-progress",
                    ProgressPayload {
                        stage: stage_str.to_string(),
                        bytes_processed: progress.bytes_processed,
                        total_bytes: progress.total_bytes,
                        files_processed: progress.files_processed,
                        total_files: progress.total_files,
                        current_item_name: Some(progress.item_name),
                        current_file_name: progress.current_file,
                        bytes_per_sec: progress.bytes_per_sec,
                        item_index: completed_count_clone
                            .load(std::sync::atomic::Ordering::Relaxed),
                        total_items,
                    },
                );
            }
        });

        // Collect results
        let mut total_bytes = 0u64;
        let mut total_files = 0usize;
        let mut items_succeeded = 0usize;
        let mut first_error = None;

        while let Ok((id, result)) = result_rx.recv() {
            match result {
                BatchTaskResult::Zip(Ok(res)) => {
                    total_bytes += res.bytes_written;
                    total_files += res.files_archived;
                    items_succeeded += 1;
                    completed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = app_handle.emit(
                        "item-completed",
                        ItemCompletedPayload {
                            id,
                            success: true,
                            bytes: res.bytes_written,
                            files: res.files_archived,
                            error: None,
                        },
                    );
                }
                BatchTaskResult::Zip(Err(e)) => {
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                    }
                    completed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = app_handle.emit(
                        "item-completed",
                        ItemCompletedPayload {
                            id,
                            success: false,
                            bytes: 0,
                            files: 0,
                            error: Some(e.to_string()),
                        },
                    );
                }
                _ => {}
            }
        }

        let _ = progress_thread.join();

        let _ = app_handle.emit(
            "op-completed",
            CompletedPayload {
                success: first_error.is_none(),
                is_zip: true,
                total_bytes,
                total_files,
                items_completed: items_succeeded,
                total_items,
                error_message: first_error,
            },
        );
    });

    Ok(())
}

#[tauri::command]
pub async fn start_unzip_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    items: Vec<String>,
    destination: Option<String>,
    delete_original: bool,
    fast: bool,
    keep_top_dir: bool,
    keep_mtime: bool,
) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }

    let cancel_token = std::sync::Arc::new(CancellationToken::new());
    *state.current_cancel_token.lock().unwrap() = Some(cancel_token.clone());

    let app_handle = app.clone();
    let dest_dir = destination.map(PathBuf::from);

    tokio::task::spawn_blocking(move || {
        let mut items = items;
        sort_paths_naturally(&mut items);

        let options = UnzipOptions {
            delete_original,
            keep_top_dir,
            destination_dir: dest_dir,
            keep_mtime,
            max_threads: if fast { None } else { Some(1) },
        };

        let init_items: Vec<QueueItemInit> = items
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let path = PathBuf::from(p);
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let (b, f) = quick_scan_zip(&path);
                QueueItemInit {
                    id: format!("item_{}", i),
                    name,
                    path: p.clone(),
                    total_bytes: b,
                    total_files: f,
                }
            })
            .collect();

        let mut grand_total_bytes = 0u64;
        let mut grand_total_files = 0usize;
        for it in &init_items {
            grand_total_bytes += it.total_bytes;
            grand_total_files += it.total_files;
        }

        let _ = app_handle.emit(
            "op-init",
            BatchInitPayload {
                is_zip: false,
                total_items: init_items.len(),
                grand_total_bytes,
                grand_total_files,
                items: init_items,
            },
        );

        let tasks: Vec<BatchTask> = items
            .into_iter()
            .enumerate()
            .map(|(i, p)| BatchTask::Unzip {
                id: format!("item_{}", i),
                archive_path: PathBuf::from(p),
                options: options.clone(),
            })
            .collect();

        let total_items = tasks.len();
        let coordinator = BatchCoordinator::new(get_worker_count(fast));
        let (progress_rx, result_rx) =
            coordinator.execute_batch(tasks, Some((*cancel_token).clone()));

        let completed_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let completed_count_clone = completed_count.clone();

        // Stream progress events to UI
        let app_handle_progress = app_handle.clone();
        let progress_thread = std::thread::spawn(move || {
            while let Ok(progress) = progress_rx.recv() {
                let stage_str = match progress.stage {
                    OperationStage::Scanning => "Scanning",
                    OperationStage::Archiving => "Archiving",
                    OperationStage::Extracting => "Extracting",
                    OperationStage::Finalizing => "Finalizing",
                    OperationStage::CleaningUp => "CleaningUp",
                    OperationStage::Completed => "Completed",
                    OperationStage::Failed => "Failed",
                    OperationStage::Cancelled => "Cancelled",
                };

                let _ = app_handle_progress.emit(
                    "op-progress",
                    ProgressPayload {
                        stage: stage_str.to_string(),
                        bytes_processed: progress.bytes_processed,
                        total_bytes: progress.total_bytes,
                        files_processed: progress.files_processed,
                        total_files: progress.total_files,
                        current_item_name: Some(progress.item_name),
                        current_file_name: progress.current_file,
                        bytes_per_sec: progress.bytes_per_sec,
                        item_index: completed_count_clone
                            .load(std::sync::atomic::Ordering::Relaxed),
                        total_items,
                    },
                );
            }
        });

        // Collect results
        let mut total_bytes = 0u64;
        let mut total_files = 0usize;
        let mut items_succeeded = 0usize;
        let mut first_error = None;

        while let Ok((id, result)) = result_rx.recv() {
            match result {
                BatchTaskResult::Unzip(Ok(res)) => {
                    total_bytes += res.bytes_extracted;
                    total_files += res.files_extracted;
                    items_succeeded += 1;
                    completed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = app_handle.emit(
                        "item-completed",
                        ItemCompletedPayload {
                            id,
                            success: true,
                            bytes: res.bytes_extracted,
                            files: res.files_extracted,
                            error: None,
                        },
                    );
                }
                BatchTaskResult::Unzip(Err(e)) => {
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                    }
                    completed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = app_handle.emit(
                        "item-completed",
                        ItemCompletedPayload {
                            id,
                            success: false,
                            bytes: 0,
                            files: 0,
                            error: Some(e.to_string()),
                        },
                    );
                }
                _ => {}
            }
        }

        let _ = progress_thread.join();

        let _ = app_handle.emit(
            "op-completed",
            CompletedPayload {
                success: first_error.is_none(),
                is_zip: false,
                total_bytes,
                total_files,
                items_completed: items_succeeded,
                total_items,
                error_message: first_error,
            },
        );
    });

    Ok(())
}
