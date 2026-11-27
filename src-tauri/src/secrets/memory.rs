use std::collections::HashMap;
use std::sync::Mutex;

use super::{SecretError, SecretStore, SecretString};

#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    fn items(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemoryStore {
    fn exists(&self, account: &str) -> Result<bool, SecretError> {
        Ok(self.items().contains_key(account))
    }

    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        Ok(self.items().get(account).map(SecretString::new))
    }

    fn set(&self, account: &str, value: &SecretString) -> Result<(), SecretError> {
        self.items()
            .insert(account.to_string(), value.expose().to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.items().remove(account);
        Ok(())
    }
}
