mod validate;

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use zeroize::Zeroize;

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
    #[serde(skip)]
    pub status: Option<i32>,
}

impl ApiKeyError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            status: None,
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
        let status = match error {
            SecretError::Read(status) => status,
            SecretError::Write | SecretError::Delete => None,
        };
        Self {
            status,
            ..Self::new("keychain-failed", error.to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyStatus {
    pub saved: bool,
    pub last_four: Option<String>,
    pub enabled: bool,
    pub read_failed: bool,
}

impl ApiKeyStatus {
    fn absent() -> Self {
        Self {
            saved: false,
            last_four: None,
            enabled: false,
            read_failed: false,
        }
    }
}

struct CachedStatus {
    saved: bool,
    last_four: Option<String>,
}

#[derive(Default)]
pub struct StatusCache(Mutex<Option<CachedStatus>>);

impl StatusCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<CachedStatus>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn supported(provider: &str) -> Result<(), ApiKeyError> {
    if provider == CLAUDE_CODE_ID {
        Ok(())
    } else {
        Err(ApiKeyError::unsupported())
    }
}

fn last_four(key: &SecretString) -> String {
    let mut tail: Vec<char> = key.expose().chars().rev().take(LAST_FOUR).collect();
    tail.reverse();
    tail.into_iter().collect()
}

fn switch_is_on(db: &Database) -> Result<bool, ApiKeyError> {
    let value = db
        .with_connection(|connection| settings::get(connection, ENABLED_SETTING))
        .map_err(ApiKeyError::database)?;
    Ok(value.as_deref() != Some("false"))
}

fn write_switch(db: &Database, enabled: bool) -> Result<(), ApiKeyError> {
    let value = if enabled { "true" } else { "false" };
    db.with_connection(|connection| settings::set(connection, ENABLED_SETTING, value))
        .map_err(ApiKeyError::database)
}

fn status_of(
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
) -> Result<ApiKeyStatus, ApiKeyError> {
    let enabled = switch_is_on(db)?;
    let Ok(exists) = store.exists(ACCOUNT) else {
        return Ok(ApiKeyStatus {
            enabled,
            read_failed: true,
            ..ApiKeyStatus::absent()
        });
    };
    let mut cached = cache.lock();
    let last_four = cached
        .as_ref()
        .filter(|entry| entry.saved && exists)
        .and_then(|entry| entry.last_four.clone());
    *cached = Some(CachedStatus {
        saved: exists,
        last_four: last_four.clone(),
    });
    if !exists {
        return Ok(ApiKeyStatus::absent());
    }
    Ok(ApiKeyStatus {
        saved: true,
        last_four,
        enabled,
        read_failed: false,
    })
}

pub fn status(
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
    provider: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    status_of(db, store, cache)
}

pub fn save(
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
    provider: &str,
    key: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    let key = validate::key(key)?;
    store.set(ACCOUNT, &key)?;
    let last_four = last_four(&key);
    *cache.lock() = Some(CachedStatus {
        saved: true,
        last_four: Some(last_four.clone()),
    });
    write_switch(db, true)?;
    Ok(ApiKeyStatus {
        saved: true,
        last_four: Some(last_four),
        enabled: true,
        read_failed: false,
    })
}

pub fn remove(
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
    provider: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    store.delete(ACCOUNT)?;
    *cache.lock() = None;
    db.with_connection(|connection| settings::delete(connection, ENABLED_SETTING))
        .map_err(ApiKeyError::database)?;
    Ok(ApiKeyStatus::absent())
}

pub fn set_enabled(
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
    provider: &str,
    enabled: bool,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    write_switch(db, enabled)?;
    status_of(db, store, cache)
}

pub fn key_for_turn(
    db: &Database,
    store: &dyn SecretStore,
    provider: &str,
) -> Result<Option<SecretString>, ApiKeyError> {
    if provider != CLAUDE_CODE_ID || !switch_is_on(db)? || !store.exists(ACCOUNT)? {
        return Ok(None);
    }
    store
        .get(ACCOUNT)?
        .map(Some)
        .ok_or_else(|| SecretError::Read(None).into())
}

pub fn api_key_for_turn(
    app: &AppHandle,
    provider: &str,
) -> Result<Option<SecretString>, ApiKeyError> {
    let (db, store, _) = state(app)?;
    key_for_turn(&db, &**store, provider)
}

pub fn claude_code_status(app: &AppHandle) -> Result<ApiKeyStatus, ApiKeyError> {
    let (db, store, cache) = state(app)?;
    status_of(&db, &**store, &cache)
}

type State<'a> = (
    tauri::State<'a, Database>,
    tauri::State<'a, SharedStore>,
    tauri::State<'a, StatusCache>,
);

fn state(app: &AppHandle) -> Result<State<'_>, ApiKeyError> {
    Ok((
        app.try_state::<Database>()
            .ok_or_else(ApiKeyError::unavailable)?,
        app.try_state::<SharedStore>()
            .ok_or_else(ApiKeyError::unavailable)?,
        app.try_state::<StatusCache>()
            .ok_or_else(ApiKeyError::unavailable)?,
    ))
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&Database, &dyn SecretStore, &StatusCache) -> Result<T, ApiKeyError>
        + Send
        + 'static,
) -> Result<T, ApiKeyError> {
    tauri::async_runtime::spawn_blocking(move || {
        let (db, store, cache) = state(&app)?;
        work(&db, &**store, &cache)
    })
    .await
    .map_err(ApiKeyError::database)?
}

#[tauri::command]
pub async fn api_key_status(app: AppHandle, provider: String) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store, cache| {
        status(db, store, cache, &provider)
    })
    .await
}

#[tauri::command]
pub async fn save_api_key(
    app: AppHandle,
    provider: String,
    mut key: String,
) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store, cache| {
        let result = save(db, store, cache, &provider, &key);
        key.zeroize();
        result
    })
    .await
}

#[tauri::command]
pub async fn remove_api_key(app: AppHandle, provider: String) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store, cache| {
        remove(db, store, cache, &provider)
    })
    .await
}

#[tauri::command]
pub async fn set_api_key_enabled(
    app: AppHandle,
    provider: String,
    enabled: bool,
) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |db, store, cache| {
        set_enabled(db, store, cache, &provider, enabled)
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
        cache: StatusCache,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path()).unwrap();
        Fixture {
            _dir: dir,
            db,
            store: MemoryStore::default(),
            cache: StatusCache::default(),
        }
    }

    impl Fixture {
        fn save(&self) -> ApiKeyStatus {
            save(&self.db, &self.store, &self.cache, PROVIDER, KEY).unwrap()
        }

        fn status(&self) -> ApiKeyStatus {
            status(&self.db, &self.store, &self.cache, PROVIDER).unwrap()
        }

        fn set_enabled(&self, enabled: bool) -> ApiKeyStatus {
            set_enabled(&self.db, &self.store, &self.cache, PROVIDER, enabled).unwrap()
        }

        fn remove(&self) -> ApiKeyStatus {
            remove(&self.db, &self.store, &self.cache, PROVIDER).unwrap()
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

    struct NeverDecrypts(MemoryStore);

    impl SecretStore for NeverDecrypts {
        fn exists(&self, account: &str) -> Result<bool, SecretError> {
            self.0.exists(account)
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            panic!("the key was decrypted");
        }
        fn set(&self, account: &str, value: &SecretString) -> Result<(), SecretError> {
            self.0.set(account, value)
        }
        fn delete(&self, account: &str) -> Result<(), SecretError> {
            self.0.delete(account)
        }
    }

    struct NeverTouched;

    impl SecretStore for NeverTouched {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            panic!("the Keychain was touched");
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            panic!("the Keychain was touched");
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            panic!("the Keychain was touched");
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            panic!("the Keychain was touched");
        }
    }

    struct Broken;

    impl SecretStore for Broken {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            Err(SecretError::Read(None))
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            Err(SecretError::Read(None))
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            Err(SecretError::Write)
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Delete)
        }
    }

    #[test]
    fn status_without_a_key_is_empty() {
        let fx = fixture();
        assert_eq!(fx.status(), ApiKeyStatus::absent());
    }

    #[test]
    fn save_stores_the_trimmed_key_and_reports_the_last_four() {
        let fx = fixture();
        let status = save(
            &fx.db,
            &fx.store,
            &fx.cache,
            PROVIDER,
            &format!("  {KEY}\n"),
        )
        .unwrap();
        assert_eq!(
            status,
            ApiKeyStatus {
                saved: true,
                last_four: Some("0000".to_string()),
                enabled: true,
                read_failed: false
            }
        );
        assert_eq!(fx.store.get(ACCOUNT).unwrap().unwrap().expose(), KEY);
        assert_eq!(fx.setting().as_deref(), Some("true"));
    }

    #[test]
    fn the_last_four_are_the_real_last_four() {
        let fx = fixture();
        let status = save(
            &fx.db,
            &fx.store,
            &fx.cache,
            PROVIDER,
            "sk-ant-test-00000000abcd",
        )
        .unwrap();
        assert_eq!(status.last_four.as_deref(), Some("abcd"));
        assert_eq!(fx.status().last_four.as_deref(), Some("abcd"));
        assert_eq!(last_four(&SecretString::new("ab")), "ab");
        assert_eq!(last_four(&SecretString::new("abcde")), "bcde");
    }

    #[test]
    fn the_database_never_holds_the_key() {
        let fx = fixture();
        fx.save();
        fx.set_enabled(false);
        assert_eq!(fx.text_columns_containing("sk-ant"), 0);
        assert_eq!(fx.text_columns_containing("claude-code.use-api-key"), 1);
    }

    #[test]
    fn no_command_result_contains_the_key() {
        let fx = fixture();
        let results = [
            serde_json::to_string(&fx.save()).unwrap(),
            serde_json::to_string(&fx.status()).unwrap(),
            serde_json::to_string(&fx.set_enabled(false)).unwrap(),
            serde_json::to_string(&fx.remove()).unwrap(),
            serde_json::to_string(
                &save(&fx.db, &fx.store, &fx.cache, PROVIDER, "sk-ant bad").unwrap_err(),
            )
            .unwrap(),
        ];
        for json in results {
            assert!(!json.contains("sk-ant-test"), "{json}");
        }
    }

    #[test]
    fn a_bad_key_is_refused_and_nothing_is_saved() {
        let fx = fixture();
        let error = save(&fx.db, &fx.store, &fx.cache, PROVIDER, "sk-ant bad").unwrap_err();
        assert_eq!(error.code, "key-invalid");
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
        assert_eq!(fx.setting(), None);
    }

    #[test]
    fn toggling_off_and_on_controls_the_key_for_a_turn() {
        let fx = fixture();
        fx.save();
        let off = fx.set_enabled(false);
        assert!(off.saved && !off.enabled);
        assert_eq!(key_for_turn(&fx.db, &fx.store, PROVIDER).unwrap(), None);
        assert!(fx.set_enabled(true).enabled);
        let key = key_for_turn(&fx.db, &fx.store, PROVIDER).unwrap().unwrap();
        assert_eq!(key.expose(), KEY);
    }

    #[test]
    fn saving_again_turns_the_switch_back_on() {
        let fx = fixture();
        fx.save();
        fx.set_enabled(false);
        assert!(fx.save().enabled);
    }

    #[test]
    fn no_key_means_none_for_a_turn() {
        let fx = fixture();
        assert_eq!(key_for_turn(&fx.db, &fx.store, PROVIDER).unwrap(), None);
    }

    #[test]
    fn a_turn_with_the_switch_off_does_not_touch_the_keychain() {
        let fx = fixture();
        fx.save();
        fx.set_enabled(false);
        assert_eq!(key_for_turn(&fx.db, &NeverTouched, PROVIDER).unwrap(), None);
    }

    #[test]
    fn a_turn_without_an_item_does_not_decrypt() {
        let fx = fixture();
        let store = NeverDecrypts(MemoryStore::default());
        assert_eq!(key_for_turn(&fx.db, &store, PROVIDER).unwrap(), None);
    }

    #[test]
    fn another_provider_gets_no_key_and_no_error() {
        let fx = fixture();
        fx.save();
        assert_eq!(key_for_turn(&fx.db, &NeverTouched, "other").unwrap(), None);
    }

    #[test]
    fn a_turn_reports_a_failed_keychain_read() {
        let fx = fixture();
        let error = key_for_turn(&fx.db, &Broken, PROVIDER).unwrap_err();
        assert_eq!(error.code, "keychain-failed");
    }

    struct Vanished;

    impl SecretStore for Vanished {
        fn exists(&self, _: &str) -> Result<bool, SecretError> {
            Ok(true)
        }
        fn get(&self, _: &str) -> Result<Option<SecretString>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &SecretString) -> Result<(), SecretError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[test]
    fn a_saved_item_that_cannot_be_read_back_is_an_error_for_a_turn() {
        let fx = fixture();
        let error = key_for_turn(&fx.db, &Vanished, PROVIDER).unwrap_err();
        assert_eq!(error.code, "keychain-failed");
        assert_eq!(error.status, None);
    }

    #[test]
    fn the_keychain_status_code_is_kept_for_diagnostics_and_never_serialised() {
        let error = ApiKeyError::from(SecretError::Read(Some(-25293)));
        assert_eq!(error.status, Some(-25293));
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({"code": "keychain-failed", "message": "could not read the saved API key from the Keychain"})
        );
    }

    #[test]
    fn status_and_toggle_never_decrypt() {
        let fx = fixture();
        fx.save();
        let store = NeverDecrypts(MemoryStore::default());
        store.set(ACCOUNT, &SecretString::new(KEY)).unwrap();
        let cold = StatusCache::default();
        let first = status(&fx.db, &store, &cold, PROVIDER).unwrap();
        assert_eq!(
            first,
            ApiKeyStatus {
                saved: true,
                last_four: None,
                enabled: true,
                read_failed: false
            }
        );
        let off = set_enabled(&fx.db, &store, &cold, PROVIDER, false).unwrap();
        assert!(off.saved && !off.enabled);
        assert!(status(&fx.db, &store, &cold, PROVIDER).unwrap().saved);
    }

    #[test]
    fn the_cache_keeps_the_last_four_after_a_save_and_clears_on_remove() {
        let fx = fixture();
        let store = NeverDecrypts(MemoryStore::default());
        save(&fx.db, &store, &fx.cache, PROVIDER, KEY).unwrap();
        let again = status(&fx.db, &store, &fx.cache, PROVIDER).unwrap();
        assert_eq!(again.last_four.as_deref(), Some("0000"));
        remove(&fx.db, &store, &fx.cache, PROVIDER).unwrap();
        assert_eq!(
            status(&fx.db, &store, &fx.cache, PROVIDER).unwrap(),
            ApiKeyStatus::absent()
        );
        store.0.set(ACCOUNT, &SecretString::new(KEY)).unwrap();
        let after = status(&fx.db, &store, &fx.cache, PROVIDER).unwrap();
        assert!(after.saved);
        assert_eq!(after.last_four, None);
    }

    #[test]
    fn a_failed_keychain_read_does_not_trap_the_user() {
        let fx = fixture();
        fx.db
            .with_connection(|c| settings::set(c, ENABLED_SETTING, "true"))
            .unwrap();
        let shown = status(&fx.db, &Broken, &fx.cache, PROVIDER).unwrap();
        assert!(shown.read_failed && shown.enabled);
        let off = set_enabled(&fx.db, &Broken, &fx.cache, PROVIDER, false).unwrap();
        assert!(off.read_failed && !off.enabled);
        assert_eq!(fx.setting().as_deref(), Some("false"));
    }

    #[test]
    fn remove_needs_no_decrypt_and_is_repeatable() {
        let fx = fixture();
        let store = NeverDecrypts(MemoryStore::default());
        save(&fx.db, &store, &fx.cache, PROVIDER, KEY).unwrap();
        assert_eq!(
            remove(&fx.db, &store, &fx.cache, PROVIDER).unwrap(),
            ApiKeyStatus::absent()
        );
        assert!(!store.0.exists(ACCOUNT).unwrap());
        assert_eq!(fx.setting(), None);
        assert!(!remove(&fx.db, &store, &fx.cache, PROVIDER).unwrap().saved);
    }

    #[test]
    fn save_writes_the_keychain_before_the_setting() {
        let fx = fixture();
        let error = save(&fx.db, &Broken, &fx.cache, PROVIDER, KEY).unwrap_err();
        assert_eq!(error.code, "keychain-failed");
        assert_eq!(fx.setting(), None);
    }

    #[test]
    fn other_providers_are_unsupported_by_every_command() {
        let fx = fixture();
        let codes = [
            status(&fx.db, &fx.store, &fx.cache, "other")
                .unwrap_err()
                .code,
            save(&fx.db, &fx.store, &fx.cache, "other", KEY)
                .unwrap_err()
                .code,
            remove(&fx.db, &fx.store, &fx.cache, "other")
                .unwrap_err()
                .code,
            set_enabled(&fx.db, &fx.store, &fx.cache, "other", true)
                .unwrap_err()
                .code,
        ];
        assert!(codes.iter().all(|code| *code == "unsupported"));
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
    }

    #[test]
    fn keychain_failures_are_generic() {
        let fx = fixture();
        let error = save(&fx.db, &Broken, &fx.cache, PROVIDER, KEY).unwrap_err();
        assert_eq!(error.message, "could not save the API key in the Keychain");
    }
}
