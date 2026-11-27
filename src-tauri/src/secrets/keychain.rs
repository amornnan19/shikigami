use keyring::{Entry, Error};

use super::{SecretError, SecretStore, SecretString, KEYCHAIN_SERVICE};

pub struct KeychainStore;

fn entry(account: &str, failure: SecretError) -> Result<Entry, SecretError> {
    Entry::new(KEYCHAIN_SERVICE, account).map_err(|_| failure)
}

impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        match entry(account, SecretError::Read)?.get_password() {
            Ok(value) => Ok(Some(SecretString::new(value))),
            Err(Error::NoEntry) => Ok(None),
            Err(_) => Err(SecretError::Read),
        }
    }

    fn set(&self, account: &str, value: &SecretString) -> Result<(), SecretError> {
        entry(account, SecretError::Write)?
            .set_password(value.expose())
            .map_err(|_| SecretError::Write)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match entry(account, SecretError::Delete)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => Err(SecretError::Delete),
        }
    }
}
