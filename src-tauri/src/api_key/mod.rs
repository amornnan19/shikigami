mod validate;

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use zeroize::Zeroize;

use crate::auth::AuthMode;
use crate::providers::CLAUDE_CODE_ID;
use crate::secrets::{SecretError, SecretStore, SecretString, SharedStore};

pub(crate) const ACCOUNT: &str = "anthropic-api-key";
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

    fn task(error: impl std::fmt::Display) -> Self {
        Self::new(
            "task-failed",
            format!("Could not run the API key task: {error}"),
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
    pub read_failed: bool,
}

impl ApiKeyStatus {
    fn absent() -> Self {
        Self {
            saved: false,
            last_four: None,
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

pub fn key_exists(store: &dyn SecretStore) -> Result<bool, SecretError> {
    store.exists(ACCOUNT)
}

fn status_of(store: &dyn SecretStore, cache: &StatusCache) -> Result<ApiKeyStatus, ApiKeyError> {
    let Ok(exists) = store.exists(ACCOUNT) else {
        return Ok(ApiKeyStatus {
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
        read_failed: false,
    })
}

pub fn status(
    store: &dyn SecretStore,
    cache: &StatusCache,
    provider: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    status_of(store, cache)
}

pub fn save(
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
    Ok(ApiKeyStatus {
        saved: true,
        last_four: Some(last_four),
        read_failed: false,
    })
}

pub fn remove(
    store: &dyn SecretStore,
    cache: &StatusCache,
    provider: &str,
) -> Result<ApiKeyStatus, ApiKeyError> {
    supported(provider)?;
    store.delete(ACCOUNT)?;
    *cache.lock() = None;
    Ok(ApiKeyStatus::absent())
}

pub fn key_for_turn(
    store: &dyn SecretStore,
    mode: AuthMode,
    provider: &str,
) -> Result<Option<SecretString>, ApiKeyError> {
    if provider != CLAUDE_CODE_ID || mode == AuthMode::Subscription || !store.exists(ACCOUNT)? {
        return Ok(None);
    }
    store
        .get(ACCOUNT)?
        .map(Some)
        .ok_or_else(|| SecretError::Read(None).into())
}

type State<'a> = (tauri::State<'a, SharedStore>, tauri::State<'a, StatusCache>);

fn state(app: &AppHandle) -> Result<State<'_>, ApiKeyError> {
    Ok((
        app.try_state::<SharedStore>()
            .ok_or_else(ApiKeyError::unavailable)?,
        app.try_state::<StatusCache>()
            .ok_or_else(ApiKeyError::unavailable)?,
    ))
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&dyn SecretStore, &StatusCache) -> Result<T, ApiKeyError> + Send + 'static,
) -> Result<T, ApiKeyError> {
    tauri::async_runtime::spawn_blocking(move || {
        let (store, cache) = state(&app)?;
        work(&**store, &cache)
    })
    .await
    .map_err(ApiKeyError::task)?
}

#[tauri::command]
pub async fn api_key_status(app: AppHandle, provider: String) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |store, cache| status(store, cache, &provider)).await
}

#[tauri::command]
pub async fn save_api_key(
    app: AppHandle,
    provider: String,
    mut key: String,
) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |store, cache| {
        let result = save(store, cache, &provider, &key);
        key.zeroize();
        result
    })
    .await
}

#[tauri::command]
pub async fn remove_api_key(app: AppHandle, provider: String) -> Result<ApiKeyStatus, ApiKeyError> {
    blocking(app, move |store, cache| remove(store, cache, &provider)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    const KEY: &str = "sk-ant-test-0000000000000000";
    const PROVIDER: &str = "claude-code";

    struct Fixture {
        store: MemoryStore,
        cache: StatusCache,
    }

    fn fixture() -> Fixture {
        Fixture {
            store: MemoryStore::default(),
            cache: StatusCache::default(),
        }
    }

    impl Fixture {
        fn save(&self) -> ApiKeyStatus {
            save(&self.store, &self.cache, PROVIDER, KEY).unwrap()
        }

        fn status(&self) -> ApiKeyStatus {
            status(&self.store, &self.cache, PROVIDER).unwrap()
        }

        fn remove(&self) -> ApiKeyStatus {
            remove(&self.store, &self.cache, PROVIDER).unwrap()
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
    fn status_without_a_key_is_empty() {
        let fx = fixture();
        assert_eq!(fx.status(), ApiKeyStatus::absent());
    }

    #[test]
    fn save_stores_the_trimmed_key_and_reports_the_last_four() {
        let fx = fixture();
        let status = save(&fx.store, &fx.cache, PROVIDER, &format!("  {KEY}\n")).unwrap();
        assert_eq!(
            status,
            ApiKeyStatus {
                saved: true,
                last_four: Some("0000".to_string()),
                read_failed: false
            }
        );
        assert_eq!(fx.store.get(ACCOUNT).unwrap().unwrap().expose(), KEY);
    }

    #[test]
    fn the_last_four_are_the_real_last_four() {
        let fx = fixture();
        let status = save(&fx.store, &fx.cache, PROVIDER, "sk-ant-test-00000000abcd").unwrap();
        assert_eq!(status.last_four.as_deref(), Some("abcd"));
        assert_eq!(fx.status().last_four.as_deref(), Some("abcd"));
        assert_eq!(last_four(&SecretString::new("ab")), "ab");
        assert_eq!(last_four(&SecretString::new("abcde")), "bcde");
    }

    #[test]
    fn no_command_result_contains_the_key() {
        let fx = fixture();
        let results = [
            serde_json::to_string(&fx.save()).unwrap(),
            serde_json::to_string(&fx.status()).unwrap(),
            serde_json::to_string(&fx.remove()).unwrap(),
            serde_json::to_string(&save(&fx.store, &fx.cache, PROVIDER, "sk-ant bad").unwrap_err())
                .unwrap(),
        ];
        for json in results {
            assert!(!json.contains("sk-ant-test"), "{json}");
        }
    }

    #[test]
    fn a_bad_key_is_refused_and_nothing_is_saved() {
        let fx = fixture();
        let error = save(&fx.store, &fx.cache, PROVIDER, "sk-ant bad").unwrap_err();
        assert_eq!(error.code, "key-invalid");
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
    }

    #[test]
    fn an_api_key_turn_gets_the_saved_key() {
        let fx = fixture();
        fx.save();
        let key = key_for_turn(&fx.store, AuthMode::ApiKey, PROVIDER)
            .unwrap()
            .unwrap();
        assert_eq!(key.expose(), KEY);
    }

    #[test]
    fn an_api_key_turn_without_an_item_gets_none_and_does_not_decrypt() {
        let store = NeverDecrypts(MemoryStore::default());
        assert_eq!(
            key_for_turn(&store, AuthMode::ApiKey, PROVIDER).unwrap(),
            None
        );
    }

    #[test]
    fn a_subscription_turn_does_not_touch_the_keychain() {
        assert_eq!(
            key_for_turn(&NeverTouched, AuthMode::Subscription, PROVIDER).unwrap(),
            None
        );
    }

    #[test]
    fn another_provider_gets_no_key_and_no_error() {
        assert_eq!(
            key_for_turn(&NeverTouched, AuthMode::ApiKey, "other").unwrap(),
            None
        );
    }

    #[test]
    fn a_turn_reports_a_failed_keychain_read() {
        let error = key_for_turn(&Broken, AuthMode::ApiKey, PROVIDER).unwrap_err();
        assert_eq!(error.code, "keychain-failed");
    }

    #[test]
    fn a_saved_item_that_cannot_be_read_back_is_an_error_for_a_turn() {
        let error = key_for_turn(&Vanished, AuthMode::ApiKey, PROVIDER).unwrap_err();
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
    fn status_never_decrypts() {
        let store = NeverDecrypts(MemoryStore::default());
        store.set(ACCOUNT, &SecretString::new(KEY)).unwrap();
        let cold = StatusCache::default();
        let first = status(&store, &cold, PROVIDER).unwrap();
        assert_eq!(
            first,
            ApiKeyStatus {
                saved: true,
                last_four: None,
                read_failed: false
            }
        );
        assert!(status(&store, &cold, PROVIDER).unwrap().saved);
    }

    #[test]
    fn the_cache_keeps_the_last_four_after_a_save_and_clears_on_remove() {
        let cache = StatusCache::default();
        let store = NeverDecrypts(MemoryStore::default());
        save(&store, &cache, PROVIDER, KEY).unwrap();
        let again = status(&store, &cache, PROVIDER).unwrap();
        assert_eq!(again.last_four.as_deref(), Some("0000"));
        remove(&store, &cache, PROVIDER).unwrap();
        assert_eq!(
            status(&store, &cache, PROVIDER).unwrap(),
            ApiKeyStatus::absent()
        );
        store.0.set(ACCOUNT, &SecretString::new(KEY)).unwrap();
        let after = status(&store, &cache, PROVIDER).unwrap();
        assert!(after.saved);
        assert_eq!(after.last_four, None);
    }

    #[test]
    fn a_failed_keychain_read_is_shown_and_does_not_trap_the_user() {
        let shown = status(&Broken, &StatusCache::default(), PROVIDER).unwrap();
        assert!(shown.read_failed && !shown.saved);
    }

    #[test]
    fn remove_needs_no_decrypt_and_is_repeatable() {
        let cache = StatusCache::default();
        let store = NeverDecrypts(MemoryStore::default());
        save(&store, &cache, PROVIDER, KEY).unwrap();
        assert_eq!(
            remove(&store, &cache, PROVIDER).unwrap(),
            ApiKeyStatus::absent()
        );
        assert!(!store.0.exists(ACCOUNT).unwrap());
        assert!(!remove(&store, &cache, PROVIDER).unwrap().saved);
    }

    #[test]
    fn other_providers_are_unsupported_by_every_command() {
        let fx = fixture();
        let codes = [
            status(&fx.store, &fx.cache, "other").unwrap_err().code,
            save(&fx.store, &fx.cache, "other", KEY).unwrap_err().code,
            remove(&fx.store, &fx.cache, "other").unwrap_err().code,
        ];
        assert!(codes.iter().all(|code| *code == "unsupported"));
        assert_eq!(fx.store.get(ACCOUNT).unwrap(), None);
    }

    #[test]
    fn keychain_failures_are_generic() {
        let fx = fixture();
        let error = save(&Broken, &fx.cache, PROVIDER, KEY).unwrap_err();
        assert_eq!(error.message, "could not save the API key in the Keychain");
    }
}
