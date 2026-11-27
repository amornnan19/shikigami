mod keychain;
#[cfg(any(test, feature = "e2e"))]
mod memory;

use std::fmt;

pub use keychain::KeychainStore;
#[cfg(any(test, feature = "e2e"))]
pub use memory::MemoryStore;

pub const KEYCHAIN_SERVICE: &str = "com.amornnan.shikigami";

#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("could not read the saved API key from the Keychain")]
    Read,
    #[error("could not save the API key in the Keychain")]
    Write,
    #[error("could not remove the API key from the Keychain")]
    Delete,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError>;
    fn set(&self, account: &str, value: &SecretString) -> Result<(), SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

pub type SharedStore = Box<dyn SecretStore>;

pub fn select_store() -> SharedStore {
    #[cfg(feature = "e2e")]
    if std::env::var("SHIKIGAMI_SECRET_STORE").as_deref() == Ok("memory") {
        return Box::new(MemoryStore::default());
    }
    Box::new(KeychainStore)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_of_a_secret_is_redacted() {
        let secret = SecretString::new("sk-ant-test-0000000000000000");
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{:?}", Some(secret)), "Some([redacted])");
    }

    #[test]
    fn memory_store_round_trips_and_deletes_missing_items() {
        let store = MemoryStore::default();
        assert_eq!(store.get("a").unwrap(), None);
        store.delete("a").unwrap();
        store.set("a", &SecretString::new("one")).unwrap();
        store.set("a", &SecretString::new("two")).unwrap();
        assert_eq!(store.get("a").unwrap().unwrap().expose(), "two");
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
    }
}
