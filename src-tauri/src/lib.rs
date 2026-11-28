mod agents;
pub mod api_key;
mod auth;
mod commands;
pub mod db;
mod groups;
mod providers;
pub mod secrets;
mod settings;
mod turns;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tauri::Manager;

pub const DATA_DIR_ENV: &str = "SHIKIGAMI_DATA_DIR";

#[cfg(feature = "e2e")]
const E2E_BACKGROUND_ENV: &str = "SHIKIGAMI_E2E_BACKGROUND";

pub struct DataDir(pub PathBuf);

pub struct AppStartupError(pub String);

fn data_dir_from(
    override_dir: Option<OsString>,
    default_dir: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    match override_dir {
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
        _ => default_dir(),
    }
}

fn resolve_data_dir(app: &tauri::App) -> Result<PathBuf, String> {
    data_dir_from(std::env::var_os(DATA_DIR_ENV), || {
        app.path()
            .app_data_dir()
            .map_err(|error| format!("could not find the data folder: {error}"))
    })
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
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(all(feature = "e2e", target_os = "macos"))]
            if std::env::var_os(E2E_BACKGROUND_ENV).is_some_and(|value| value == "1") {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            let resolved = resolve_data_dir(app);
            let data_dir = resolved.clone().unwrap_or_default();
            let opened = resolved
                .and_then(|dir| open_database(&dir))
                .and_then(|database| turns::sweep_interrupted(&database).map(|_| database));
            if !data_dir.as_os_str().is_empty() {
                turns::sweep_instruction_files(&data_dir.join("tmp"));
            }
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
            app.manage(turns::TurnRegistry::with_temp_dir(data_dir.join("tmp")));
            app.manage(agents::WorkspacesRoot(agents::resolve_workspaces_root(
                app.path().home_dir().ok(),
                std::env::var_os(agents::WORKSPACES_DIR_ENV),
            )));
            app.manage(groups::GroupsConfig {
                root: groups::resolve_groups_root(
                    app.path().home_dir().ok().as_deref(),
                    std::env::var_os(groups::GROUPS_DIR_ENV),
                ),
                home: app.path().home_dir().ok(),
            });
            app.manage(DataDir(data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            auth::get_claude_auth,
            auth::set_claude_auth_mode,
            api_key::api_key_status,
            api_key::save_api_key,
            api_key::remove_api_key,
            agents::list_agents,
            agents::create_agent,
            agents::recreate_workspace,
            agents::set_agent_auth_mode,
            agents::update_agent,
            agents::read_instructions,
            agents::write_instructions,
            agents::reveal_workspace,
            agents::delete_agent,
            groups::list_groups,
            groups::create_group,
            groups::update_group,
            groups::delete_group,
            groups::pick_group_folder,
            groups::suggest_group_folder,
            turns::send_message,
            turns::get_conversation,
            turns::stop_turn,
            turns::list_queue,
            turns::edit_queued,
            turns::remove_queued,
            turns::resume_queue,
            turns::send_group_message,
            turns::get_group_conversation,
            turns::stop_group_turn,
            turns::list_group_queue,
            turns::edit_group_queued,
            turns::remove_group_queued,
            turns::resume_group_queue
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
    use super::{app_context, data_dir_from};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn product_name_is_shikigami() {
        assert_eq!(
            app_context().config().product_name.as_deref(),
            Some("Shikigami")
        );
    }

    #[test]
    fn the_data_folder_override_must_be_absolute_and_the_default_is_the_fallback() {
        let default = || Ok(PathBuf::from("/Users/me/Library/Data"));
        assert_eq!(
            data_dir_from(None, default).unwrap(),
            PathBuf::from("/Users/me/Library/Data")
        );
        assert_eq!(
            data_dir_from(Some(OsString::new()), default).unwrap(),
            PathBuf::from("/Users/me/Library/Data")
        );
        assert!(data_dir_from(Some("relative/dir".into()), default).is_err());
        assert_eq!(
            data_dir_from(Some("/tmp/elsewhere".into()), default).unwrap(),
            PathBuf::from("/tmp/elsewhere")
        );
    }
}
