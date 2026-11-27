mod commands;
pub mod db;

use std::path::PathBuf;

use tauri::Manager;

pub const DATA_DIR_ENV: &str = "SHIKIGAMI_DATA_DIR";

pub struct DataDir(pub PathBuf);

pub struct AppStartupError(pub String);

fn resolve_data_dir(app: &tauri::App) -> tauri::Result<PathBuf> {
    match std::env::var_os(DATA_DIR_ENV) {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => app.path().app_data_dir(),
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
            let data_dir = resolve_data_dir(app)?;
            match db::Database::open(&data_dir) {
                Ok(database) => {
                    app.manage(database);
                }
                Err(error) => {
                    app.manage(AppStartupError(error.to_string()));
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
