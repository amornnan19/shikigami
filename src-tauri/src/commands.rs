use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{AppStartupError, DataDir};

#[cfg(feature = "e2e")]
const E2E_REDUCE_MOTION_ENV: &str = "SHIKIGAMI_E2E_REDUCE_MOTION";

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

#[tauri::command]
pub fn reduce_motion_override() -> bool {
    #[cfg(feature = "e2e")]
    {
        std::env::var(E2E_REDUCE_MOTION_ENV).is_ok_and(|value| value == "1")
    }
    #[cfg(not(feature = "e2e"))]
    {
        false
    }
}
