use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, Manager};

use crate::api_key::{self, ApiKeyError, ApiKeyStatus, StatusCache};
use crate::db::Database;
use crate::providers::{self, ProviderStatus, SignedIn, CLAUDE_CODE_ID};
use crate::secrets::{SecretError, SecretStore, SharedStore};
use crate::settings;
use crate::DataDir;

const AUTH_MODE_SETTING: &str = "claude.auth-mode";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    Subscription,
    ApiKey,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::ApiKey => "api-key",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "subscription" => Some(Self::Subscription),
            "api-key" => Some(Self::ApiKey),
            _ => None,
        }
    }
}

pub fn effective_mode(
    agent_override: Option<AuthMode>,
    app_setting: Option<AuthMode>,
    key_exists: bool,
) -> AuthMode {
    agent_override.or(app_setting).unwrap_or(if key_exists {
        AuthMode::ApiKey
    } else {
        AuthMode::Subscription
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppAuth {
    pub setting: Option<AuthMode>,
    pub key_exists: bool,
}

impl AppAuth {
    pub fn effective(&self, agent_override: Option<AuthMode>) -> AuthMode {
        effective_mode(agent_override, self.setting, self.key_exists)
    }
}

#[derive(Debug, Serialize)]
pub struct AuthError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip)]
    pub status: Option<i32>,
}

impl AuthError {
    fn database(error: impl std::fmt::Display) -> Self {
        Self {
            code: "database-failed",
            message: format!("Could not use the sign-in setting: {error}"),
            status: None,
        }
    }

    pub const KEYCHAIN_FAILED: &'static str = "keychain-failed";

    fn keychain(error: SecretError) -> Self {
        let status = match error {
            SecretError::Read(status) => status,
            SecretError::Write | SecretError::Delete => None,
        };
        Self {
            code: Self::KEYCHAIN_FAILED,
            message: error.to_string(),
            status,
        }
    }

    fn unavailable() -> Self {
        Self {
            code: "unavailable",
            message: "The app database is not available.".to_owned(),
            status: None,
        }
    }
}

impl From<ApiKeyError> for AuthError {
    fn from(error: ApiKeyError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            status: error.status,
        }
    }
}

fn stored_setting(connection: &Connection) -> rusqlite::Result<Option<AuthMode>> {
    Ok(settings::get(connection, AUTH_MODE_SETTING)?
        .as_deref()
        .and_then(AuthMode::parse))
}

pub fn load_app_auth(db: &Database, store: &dyn SecretStore) -> Result<AppAuth, AuthError> {
    let setting = db
        .with_connection(stored_setting)
        .map_err(AuthError::database)?;
    let key_exists = setting.is_none() && api_key::key_exists(store).unwrap_or(false);
    Ok(AppAuth {
        setting,
        key_exists,
    })
}

pub fn mode_for_agent(
    db: &Database,
    store: &dyn SecretStore,
    agent_id: &str,
) -> Result<AuthMode, AuthError> {
    let agent_override = db
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT auth_mode FROM agents WHERE id = ?1",
                    [agent_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
        })
        .map_err(AuthError::database)?
        .flatten()
        .as_deref()
        .and_then(AuthMode::parse);
    match agent_override {
        Some(mode) => Ok(mode),
        None => {
            let setting = db
                .with_connection(stored_setting)
                .map_err(AuthError::database)?;
            let key_exists = match setting {
                Some(_) => false,
                None => api_key::key_exists(store).map_err(AuthError::keychain)?,
            };
            Ok(effective_mode(None, setting, key_exists))
        }
    }
}

pub fn set_app_mode(db: &Database, mode: AuthMode) -> Result<(), AuthError> {
    db.with_connection(|connection| settings::set(connection, AUTH_MODE_SETTING, mode.as_str()))
        .map_err(AuthError::database)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub signed_in: SignedIn,
    pub sign_in_hint: String,
    pub install_hint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAuth {
    pub app_mode: AuthMode,
    pub app_mode_explicit: bool,
    pub subscription: SubscriptionStatus,
    pub api_key: ApiKeyStatus,
}

fn assemble(
    provider: &ProviderStatus,
    setting: Option<AuthMode>,
    api_key: ApiKeyStatus,
    config_dir: &Path,
) -> ClaudeAuth {
    ClaudeAuth {
        app_mode: effective_mode(None, setting, api_key.saved),
        app_mode_explicit: setting.is_some(),
        subscription: SubscriptionStatus {
            installed: provider.installed,
            version: provider.version.clone(),
            signed_in: provider.signed_in,
            sign_in_hint: providers::claude_sign_in_command(config_dir),
            install_hint: provider.install_hint,
        },
        api_key,
    }
}

fn gather(
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
) -> Result<(Option<AuthMode>, ApiKeyStatus), AuthError> {
    let setting = db
        .with_connection(stored_setting)
        .map_err(AuthError::database)?;
    let key = api_key::status(store, cache, CLAUDE_CODE_ID)?;
    Ok((setting, key))
}

fn claude_auth(
    provider: &ProviderStatus,
    db: &Database,
    store: &dyn SecretStore,
    cache: &StatusCache,
    config_dir: &Path,
) -> Result<ClaudeAuth, AuthError> {
    let (setting, key) = gather(db, store, cache)?;
    Ok(assemble(provider, setting, key, config_dir))
}

#[tauri::command]
pub async fn get_claude_auth(app: AppHandle) -> Result<ClaudeAuth, AuthError> {
    let dirs = tauri::async_runtime::spawn_blocking(providers::search_dirs)
        .await
        .map_err(AuthError::database)?;
    let config_dir = app
        .try_state::<DataDir>()
        .map(|data_dir| providers::claude_config_dir(&data_dir.0))
        .ok_or_else(AuthError::unavailable)?;
    let provider = providers::claude_status(&dirs, &config_dir).await;
    tauri::async_runtime::spawn_blocking(move || {
        let db = app
            .try_state::<Database>()
            .ok_or_else(AuthError::unavailable)?;
        let store = app
            .try_state::<SharedStore>()
            .ok_or_else(AuthError::unavailable)?;
        let cache = app
            .try_state::<StatusCache>()
            .ok_or_else(AuthError::unavailable)?;
        claude_auth(&provider, &db, &**store, &cache, &config_dir)
    })
    .await
    .map_err(AuthError::database)?
}

#[tauri::command]
pub async fn set_claude_auth_mode(app: AppHandle, mode: AuthMode) -> Result<(), AuthError> {
    tauri::async_runtime::spawn_blocking(move || {
        let db = app
            .try_state::<Database>()
            .ok_or_else(AuthError::unavailable)?;
        set_app_mode(&db, mode)
    })
    .await
    .map_err(AuthError::database)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, SecretError, SecretString};

    const KEY: &str = "sk-ant-test-0000000000000000";

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

    fn add_agent(db: &Database, id: &str, mode: Option<&str>) {
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at, auth_mode)
                 VALUES (?1, ?1, 'red', 'claude-code', 'm', '/w', 1, 1, ?2)",
                rusqlite::params![id, mode],
            )
            .unwrap()
        });
    }

    fn provider() -> ProviderStatus {
        ProviderStatus {
            id: "claude-code",
            name: "Claude Code",
            installed: true,
            path: Some("/bin/claude".to_owned()),
            version: Some("2.1.0 (Claude Code)".to_owned()),
            signed_in: SignedIn::No,
            install_hint: "https://example.com",
            sign_in_hint: "claude auth login",
        }
    }

    use AuthMode::{ApiKey, Subscription};

    #[test]
    fn effective_mode_table() {
        let modes = [None, Some(Subscription), Some(ApiKey)];
        for over in modes {
            for setting in modes {
                for key_exists in [false, true] {
                    let expected = match (over, setting, key_exists) {
                        (Some(mode), _, _) => mode,
                        (None, Some(mode), _) => mode,
                        (None, None, true) => ApiKey,
                        (None, None, false) => Subscription,
                    };
                    assert_eq!(
                        effective_mode(over, setting, key_exists),
                        expected,
                        "{over:?} {setting:?} {key_exists}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_agent_override_beats_the_app_setting_both_ways() {
        let fx = fixture();
        add_agent(&fx.db, "sub", Some("subscription"));
        add_agent(&fx.db, "key", Some("api-key"));
        add_agent(&fx.db, "default", None);
        for (app, sub_agent, key_agent) in [
            (Subscription, Subscription, ApiKey),
            (ApiKey, Subscription, ApiKey),
        ] {
            set_app_mode(&fx.db, app).unwrap();
            assert_eq!(mode_for_agent(&fx.db, &fx.store, "sub").unwrap(), sub_agent);
            assert_eq!(mode_for_agent(&fx.db, &fx.store, "key").unwrap(), key_agent);
            assert_eq!(mode_for_agent(&fx.db, &fx.store, "default").unwrap(), app);
        }
    }

    #[test]
    fn an_absent_setting_follows_whether_a_key_item_exists() {
        let fx = fixture();
        add_agent(&fx.db, "a", None);
        assert_eq!(
            mode_for_agent(&fx.db, &fx.store, "a").unwrap(),
            Subscription
        );
        fx.store
            .set(api_key::ACCOUNT, &SecretString::new(KEY))
            .unwrap();
        assert_eq!(mode_for_agent(&fx.db, &fx.store, "a").unwrap(), ApiKey);
        set_app_mode(&fx.db, Subscription).unwrap();
        assert_eq!(
            mode_for_agent(&fx.db, &fx.store, "a").unwrap(),
            Subscription
        );
    }

    #[test]
    fn resolving_a_mode_never_decrypts() {
        let fx = fixture();
        add_agent(&fx.db, "a", None);
        let store = NeverDecrypts(MemoryStore::default());
        store
            .set(api_key::ACCOUNT, &SecretString::new(KEY))
            .unwrap();
        assert_eq!(mode_for_agent(&fx.db, &store, "a").unwrap(), ApiKey);
        assert_eq!(load_app_auth(&fx.db, &store).unwrap().setting, None);
    }

    #[test]
    fn an_explicit_choice_or_override_does_not_touch_the_keychain() {
        let fx = fixture();
        add_agent(&fx.db, "over", Some("subscription"));
        add_agent(&fx.db, "plain", None);
        assert_eq!(
            mode_for_agent(&fx.db, &NeverTouched, "over").unwrap(),
            Subscription
        );
        set_app_mode(&fx.db, ApiKey).unwrap();
        assert_eq!(
            mode_for_agent(&fx.db, &NeverTouched, "plain").unwrap(),
            ApiKey
        );
    }

    #[test]
    fn a_failed_existence_check_fails_the_turn_path_but_not_the_status_path() {
        let fx = fixture();
        add_agent(&fx.db, "a", None);
        let error = mode_for_agent(&fx.db, &Broken, "a").unwrap_err();
        assert_eq!(error.code, AuthError::KEYCHAIN_FAILED);
        let listed = load_app_auth(&fx.db, &Broken).unwrap();
        assert_eq!(listed.effective(None), Subscription);
        let shown = claude_auth(
            &provider(),
            &fx.db,
            &Broken,
            &fx.cache,
            Path::new("/data/claude"),
        )
        .unwrap();
        assert!(shown.api_key.read_failed);
        assert_eq!(shown.app_mode, Subscription);
    }

    #[test]
    fn choosing_the_subscription_works_with_a_broken_store() {
        let fx = fixture();
        add_agent(&fx.db, "a", None);
        add_agent(&fx.db, "pinned", Some("api-key"));
        set_app_mode(&fx.db, Subscription).unwrap();
        assert_eq!(mode_for_agent(&fx.db, &Broken, "a").unwrap(), Subscription);
        assert_eq!(mode_for_agent(&fx.db, &Broken, "pinned").unwrap(), ApiKey);
        let shown = claude_auth(
            &provider(),
            &fx.db,
            &Broken,
            &fx.cache,
            Path::new("/data/claude"),
        )
        .unwrap();
        assert!(shown.api_key.read_failed);
        assert!(shown.app_mode_explicit);
    }

    #[test]
    fn claude_auth_reports_the_state_and_never_decrypts() {
        let fx = fixture();
        let store = NeverDecrypts(MemoryStore::default());
        let cold = claude_auth(
            &provider(),
            &fx.db,
            &store,
            &fx.cache,
            Path::new("/data/claude"),
        )
        .unwrap();
        assert_eq!(cold.app_mode, Subscription);
        assert!(!cold.app_mode_explicit);
        assert!(!cold.api_key.saved);
        assert_eq!(cold.subscription.signed_in, SignedIn::No);
        store
            .set(api_key::ACCOUNT, &SecretString::new(KEY))
            .unwrap();
        let saved = claude_auth(
            &provider(),
            &fx.db,
            &store,
            &StatusCache::default(),
            Path::new("/data/claude"),
        )
        .unwrap();
        assert_eq!(saved.app_mode, ApiKey);
        assert!(!saved.app_mode_explicit);
        assert!(saved.api_key.saved);
        set_app_mode(&fx.db, Subscription).unwrap();
        let chosen = claude_auth(
            &provider(),
            &fx.db,
            &store,
            &fx.cache,
            Path::new("/data/claude"),
        )
        .unwrap();
        assert_eq!(chosen.app_mode, Subscription);
        assert!(chosen.app_mode_explicit);
        let json = serde_json::to_value(&chosen).unwrap();
        assert_eq!(json["appMode"], "subscription");
        assert_eq!(json["subscription"]["signedIn"], "no");
        assert_eq!(
            json["subscription"]["signInHint"],
            "CLAUDE_CONFIG_DIR='/data/claude' claude auth login"
        );
        assert!(!json.to_string().contains("sk-ant"));
    }
}
