mod commands;
pub mod db;

use std::path::{Path, PathBuf};

use tauri::Manager;

pub const DATA_DIR_ENV: &str = "SHIKIGAMI_DATA_DIR";

pub struct DataDir(pub PathBuf);

pub struct AppStartupError(pub String);

fn resolve_data_dir(app: &tauri::App) -> Result<PathBuf, String> {
    match std::env::var_os(DATA_DIR_ENV) {
        Some(dir) if !dir.is_empty() => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                Ok(path)
            } else {
                Err(format!(
                    "{DATA_DIR_ENV} must be an absolute path, but it is {}",
                    path.display()
                ))
            }
        }
        _ => app
            .path()
            .app_data_dir()
            .map_err(|error| format!("could not find the data folder: {error}")),
    }
}

fn open_database(data_dir: &Path) -> Result<db::Database, String> {
    match std::panic::catch_unwind(|| db::Database::open(data_dir)) {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err("the database could not be read".to_string()),
    }
}

fn app_context() -> tauri::Context {
    tauri::generate_context!()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(feature = "e2e")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    builder
        .setup(|app| {
            let resolved = resolve_data_dir(app);
            let data_dir = resolved.clone().unwrap_or_default();
            match resolved.and_then(|dir| open_database(&dir)) {
                Ok(database) => {
                    app.manage(database);
                }
                Err(message) => {
                    app.manage(AppStartupError(message));
                }
            }
            app.manage(DataDir(data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app_status])
        .run(app_context())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::app_context;

    #[test]
    fn product_name_is_shikigami() {
        assert_eq!(
            app_context().config().product_name.as_deref(),
            Some("Shikigami")
        );
    }
}
