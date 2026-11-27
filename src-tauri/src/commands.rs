use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{AppStartupError, DataDir};

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum AppStatus {
    Ok,
    Error { message: String, data_dir: String },
}

#[tauri::command]
pub fn app_status(app: AppHandle) -> AppStatus {
    match app.try_state::<AppStartupError>() {
        None => AppStatus::Ok,
        Some(error) => AppStatus::Error {
            message: error.0.clone(),
            data_dir: app
                .try_state::<DataDir>()
                .map(|dir| dir.0.display().to_string())
                .unwrap_or_default(),
        },
    }
}
