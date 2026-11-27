mod validate;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::Database;
use crate::providers::CLAUDE_CODE_ID;
use crate::secrets::{SecretError, SecretStore, SecretString, SharedStore};
use crate::settings;

const ACCOUNT: &str = "anthropic-api-key";
const ENABLED_SETTING: &str = "claude-code.use-api-key";
const LAST_FOUR: usize = 4;

#[derive(Debug, Serialize)]
pub struct ApiKeyError {
    pub code: &'static str,
    pub message: String,
}

impl ApiKeyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn key_invalid() -> Self {
        Self::new(
            "key-invalid",
            "That does not look like an Anthropic API key (it should start with sk-ant-).",
        )
    }

    fn unsupported() -> Self {
        Self::new(
            "unsupported",
            "API keys are only supported for Claude Code.",
        )
    }

    fn database(error: impl std::fmt::Display) -> Self {
        Self::new(
            "database-failed",
            format!("Could not save the API key setting: {error}"),
        )
    }

    fn unavailable() -> Self {
        Self::new("unavailable", "The app database is not available.")
    }
}

impl From<SecretError> for ApiKeyError {
    fn from(error: SecretError) -> Self {
        Self::new("keychain-failed", error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub saved: bool,
    pub last_four: Option<String>,
    pub enabled: bool,
}

fn supported(provider: &str) -> Result<(), ApiKeyError> {
    if provider == CLAUDE_CODE_ID {
        Ok(())
    } else {
        Err(ApiKeyError::unsupported())
    }
}

fn last_four(key: &SecretString) -> String {
    let chars: Vec<char> = key.expose().chars().collect();
    chars[chars.len().saturating_sub(LAST_FOUR)..]
        .iter()
        .collect()
}

fn switch_is_on(db: &Database) -> Result<bool, ApiKeyError> {
    let value = db
        .with_connection(|connection| settings::get(connection, ENABLED_SETTING))
        .map_err(ApiKeyError::database)?;
    Ok(value.as_deref() != Some("false"))
}

fn status_of(db: &Database, store: &dyn SecretStore) -> Result<ApiKeyStatus, ApiKeyError> {
    let Some(key) = store.get(ACCOUNT)? else {
        return Ok(ApiKeyStatus {
            saved: false,
            last_four: None,
            enabled: false,
        });
    };
    Ok(ApiKeyStatus {
        saved: true,
        last_four: Some(last_four(&key)),
        enabled: switch_is_on(db)?,
    })
}

pub fn status(
    db: &Database,
    store: &dyn SecretStore,
    provider: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    status_of(db, store)
}

pub fn save(
    db: &Database,
    store: &dyn SecretStore,
    provider: &str,
    key: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    let key = validate::key(key)?;
    db.with_connection(|connection| settings::set(connection, ENABLED_SETTING, "true"))
        .map_err(ApiKeyError::database)?;
    store.set(ACCOUNT, &key)?;
    status_of(db, store)
}

pub fn remove(
    db: &Database,
    store: &dyn SecretStore,
    provider: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    store.delete(ACCOUNT)?;
    db.with_connection(|connection| settings::delete(connection, ENABLED_SETTING))
        .map_err(ApiKeyError::database)?;
    status_of(db, store)
}

pub fn set_enabled(
    db: &Database,
    store: &dyn SecretStore,
    provider: &str,
    enabled: bool,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    if store.get(ACCOUNT)?.is_some() {
        let value = if enabled { "true" } else { "false" };
        db.with_connection(|connection| settings::set(connection, ENABLED_SETTING, value))
            .map_err(ApiKeyError::database)?;
    }
    status_of(db, store)
}

pub fn key_for_turn(
    db: &Database,
    store: &dyn SecretStore,
    provider: &str,
) -> Result<Option<SecretString>, ApiKeyError> {
    supported(provider)?;
    match store.get(ACCOUNT)? {
        Some(key) if switch_is_on(db)? => Ok(Some(key)),
        _ => Ok(None),
    }
}

pub fn api_key_for_turn(
    app: &AppHandle,
    provider: &str,
) -> Result<Option<SecretString>, ApiKeyError> {
    let (db, store) = state(app)?;
    key_for_turn(&db, &**store, provider)
}

pub fn claude_code_status(app: &AppHandle) -> Result<ApiKeyStatus, ApiKeyError> {
    let (db, store) = state(app)?;
    status_of(&db, &**store)
}

fn state(
    app: &AppHandle,
) -> Result<(tauri::State<'_, Database>, tauri::State<'_, SharedStore>), ApiKeyError> {
    let db = app
        .try_state::<Database>()
        .ok_or_else(ApiKeyError::unavailable)?;
    let store = app
        .try_state::<SharedStore>()
        .ok_or_else(ApiKeyError::unavailable)?;
    Ok((db, store))
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&Database, &dyn SecretStore) -> Result<T, ApiKeyError> + Send + 'static,
) -> Result<T, ApiKeyError> {
    tauri::async_runtime::spawn_blocking(move || {
        let (db, store) = state(&app)?;
        work(&db, &**store)
    })
    .await
    .map_err(ApiKeyError::database)?
}

#[tauri::command]
pub async fn api_key_status(app: AppHandle, provider: String) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store| status(db, store, &provider)).await
}

#[tauri::command]
pub async fn save_api_key(
    app: AppHandle,
    provider: String,
    key: String,
) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store| save(db, store, &provider, &key)).await
}

#[tauri::command]
pub async fn remove_api_key(app: AppHandle, provider: String) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store| remove(db, store, &provider)).await
}

#[tauri::command]
pub async fn set_api_key_enabled(
    app: AppHandle,
    provider: String,
    enabled: bool,
) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store| {
        set_enabled(db, store, &provider, enabled)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    const KEY: &str = "sk-ant-test-0000000000000000";
    const PROVIDER: &str = "claude-code";

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Database,
        store: MemoryStore,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        Fixture {
            _dir: dir,
            db,
            store: MemoryStore::default(),
        }
    }

    impl Fixture {
        fn save(&self) -> ApiKeyStatus {
            save(&self.db, &self.store, PROVIDER, KEY).unwrap()
        }

        fn status(&self) -> ApiKeyStatus {
            status(&self.db, &self.store, PROVIDER).unwrap()
        }

        fn setting(&self) -> Option<String> {
            self.db
                .with_connection(|c| settings::get(c, ENABLED_SETTING).unwrap())
        }

        fn text_columns_containing(&self, needle: &str) -> usize {
            self.db.with_connection(|c| {
                let tables: Vec<String> = c
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                let mut hits = 0;
                for table in tables {
                    let columns: Vec<String> = c
                        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
                        .unwrap()
                        .query_map([], |row| row.get(1))
                        .unwrap()
                        .map(Result::unwrap)
                        .collect();
                    for column in columns {
                        let found: i64 = c
                            .query_row(
                                &format!(
                                    "SELECT COUNT(*) FROM \"{table}\" WHERE instr(CAST(\"{column}\" AS TEXT), ?1) > 0"
                                ),
                                [needle],
                                |row| row.get(0),
                            )
                            .unwrap();
                        hits += found as usize;
                    }
                }
                hits
            })
        }
    }

    #[test]
    fn status_without_a_key_is_empty() {
        let fx = fixture();
        assert_eq!(
            fx.status(),
            ApiKeyStatus {
                saved: false,
                last_four: None,
                enabled: false
            }
        );
    }

    #[test]
    fn save_stores_the_trimmed_key_and_reports_the_last_four() {
        let fx = fixture();
        let status = save(&fx.db, &fx.store, PROVIDER, &format!("  {KEY}\n")).unwrap();
        assert_eq!(
            status,
            ApiKeyStatus {
                saved: true,
                last_four: Some("0000".to_string()),
                enabled: true
            }
        );
        assert_eq!(fx.store.get(ACCOUNT).unwrap().unwrap().expose(), KEY);
        assert_eq!(fx.setting().as_deref(), Some("true"));
    }

    #[test]
    fn the_last_four_are_the_real_last_four() {
        let fx = fixture();
        let status = save(&fx.db, &fx.store, PROVIDER, "sk-ant-test-00000000abcd").unwrap();
        assert_eq!(status.last_four.as_deref(), Some("abcd"));
    }

    #[test]
    fn the_database_never_holds_the_key() {
        let fx = fixture();
        fx.save();
        set_enabled(&fx.db, &fx.store, PROVIDER, false).unwrap();
        assert_eq!(fx.text_columns_containing("sk-ant"), 0);
        assert_eq!(fx.text_columns_containing("claude-code.use-api-key"), 1);
    }

    #[test]
    fn no_command_result_contains_the_key() {
        let fx = fixture();
        let results = [
            serde_json::to_string(&fx.save()).unwrap(),
            serde_json::to_string(&fx.status()).unwrap(),
            serde_json::to_string(&set_enabled(&fx.db, &fx.store, PROVIDER, false).unwrap())
                .unwrap(),
            serde_json::to_string(&remove(&fx.db, &fx.store, PROVIDER).unwrap()).unwrap(),
            serde_json::to_string(&save(&fx.db, &fx.store, PROVIDER, "sk-ant bad").unwrap_err())
                .unwrap(),
        ];
        for json in results {
            assert!(!json.contains("sk-ant-test"), "{json}");
        }
    }

    #[test]
    fn a_bad_key_is_refused_and_nothing_is_saved() {
        let fx = fixture();
        let error = save(&fx.db, &fx.store, PROVIDER, "sk-ant bad").unwrap_err();
        assert_eq!(error.code, "key-invalid");
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
        assert_eq!(fx.setting(), None);
    }

    #[test]
    fn toggling_off_and_on_controls_the_key_for_a_turn() {
        let fx = fixture();
        fx.save();
        let off = set_enabled(&fx.db, &fx.store, PROVIDER, false).unwrap();
        assert!(off.saved && !off.enabled);
        assert_eq!(key_for_turn(&fx.db, &fx.store, PROVIDER).unwrap(), None);
        let on = set_enabled(&fx.db, &fx.store, PROVIDER, true).unwrap();
        assert!(on.enabled);
        let key = key_for_turn(&fx.db, &fx.store, PROVIDER).unwrap().unwrap();
        assert_eq!(key.expose(), KEY);
    }

    #[test]
    fn saving_again_turns_the_switch_back_on() {
        let fx = fixture();
        fx.save();
        set_enabled(&fx.db, &fx.store, PROVIDER, false).unwrap();
        assert!(fx.save().enabled);
    }

    #[test]
    fn no_key_means_none_for_a_turn() {
        let fx = fixture();
        assert_eq!(key_for_turn(&fx.db, &fx.store, PROVIDER).unwrap(), None);
    }

    #[test]
    fn toggling_without_a_key_changes_nothing() {
        let fx = fixture();
        let status = set_enabled(&fx.db, &fx.store, PROVIDER, true).unwrap();
        assert!(!status.saved && !status.enabled);
        assert_eq!(fx.setting(), None);
    }

    #[test]
    fn remove_deletes_the_item_and_the_setting_and_is_repeatable() {
        let fx = fixture();
        fx.save();
        let status = remove(&fx.db, &fx.store, PROVIDER).unwrap();
        assert!(!status.saved);
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
        assert_eq!(fx.setting(), None);
        assert!(!remove(&fx.db, &fx.store, PROVIDER).unwrap().saved);
    }

    #[test]
    fn other_providers_are_unsupported_by_every_command() {
        let fx = fixture();
        let codes = [
            status(&fx.db, &fx.store, "codex").unwrap_err().code,
            save(&fx.db, &fx.store, "codex", KEY).unwrap_err().code,
            remove(&fx.db, &fx.store, "codex").unwrap_err().code,
            set_enabled(&fx.db, &fx.store, "codex", true)
                .unwrap_err()
                .code,
            key_for_turn(&fx.db, &fx.store, "codex").unwrap_err().code,
        ];
        assert!(codes.iter().all(|code| *code == "unsupported"));
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
    }

    struct FailingStore;

    impl SecretStore for FailingStore {
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            Err(SecretError::Read)
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            Err(SecretError::Write)
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Delete)
        }
    }

    #[test]
    fn keychain_failures_are_generic() {
        let fx = fixture();
        let error = save(&fx.db, &FailingStore, PROVIDER, KEY).unwrap_err();
        assert_eq!(error.code, "keychain-failed");
        assert_eq!(error.message, "could not save the API key in the Keychain");
        assert_eq!(
            status(&fx.db, &FailingStore, PROVIDER).unwrap_err().code,
            "keychain-failed"
        );
    }
}
