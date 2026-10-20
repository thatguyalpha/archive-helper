use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use archive_helper_core::progress::CancellationToken;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppSettings {
    pub delete_zip: bool,
    pub fast_zip: bool,
    pub predict_ext: bool,
    pub keep_mtime_zip: bool,
    pub single_folder_zip: bool,
    pub delete_unzip: bool,
    pub fast_unzip: bool,
    pub keep_top_dir: bool,
    pub keep_mtime_unzip: bool,
    pub always_on_top: bool,
    #[serde(default)]
    pub start_as_admin: bool,
    pub use_binary_units: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            delete_zip: true,
            fast_zip: false,
            predict_ext: true,
            keep_mtime_zip: true,
            single_folder_zip: false,
            delete_unzip: true,
            fast_unzip: false,
            keep_top_dir: true,
            keep_mtime_unzip: true,
            always_on_top: false,
            start_as_admin: false,
            use_binary_units: true,
        }
    }
}

impl AppSettings {
    fn settings_path() -> Option<PathBuf> {
        let app_data = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(app_data).join("ArchiveHelper").join("settings.json"))
    }

    pub fn load() -> Self {
        if let Some(path) = Self::settings_path() {
            if path.exists() {
                if let Ok(data) = fs::read_to_string(&path) {
                    if let Ok(settings) = serde_json::from_str::<AppSettings>(&data) {
                        return settings;
                    }
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::settings_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = fs::write(path, json);
            }
        }
    }
}

pub struct AppState {
    pub settings: Mutex<AppSettings>,
    pub current_cancel_token: Mutex<Option<Arc<CancellationToken>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            settings: Mutex::new(AppSettings::load()),
            current_cancel_token: Mutex::new(None),
        }
    }
}
