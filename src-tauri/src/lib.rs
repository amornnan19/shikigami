mod agents;
pub mod api_key;
mod commands;
pub mod db;
mod providers;
pub mod secrets;
mod settings;
mod turns;

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
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let resolved = resolve_data_dir(app);
            let data_dir = resolved.clone().unwrap_or_default();
            let opened = resolved
                .and_then(|dir| open_database(&dir))
                .and_then(|database| turns::sweep_interrupted(&database).map(|_| database));
            match opened {
                Ok(database) => {
                    app.manage(database);
                }
                Err(message) => {
                    app.manage(AppStartupError(message));
                }
            }
            app.manage(secrets::select_store());
            app.manage(api_key::StatusCache::default());
            app.manage(turns::TurnRegistry::default());
            app.manage(agents::WorkspacesRoot(agents::resolve_workspaces_root(
                app.path().home_dir().ok(),
            )));
            app.manage(DataDir(data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            providers::list_providers,
            api_key::api_key_status,
            api_key::save_api_key,
            api_key::remove_api_key,
            api_key::set_api_key_enabled,
            agents::list_agents,
            agents::create_agent,
            turns::send_message,
            turns::get_conversation
        ])
        .build(app_context())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                turns::stop_all_turns(app);
            }
        });
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
