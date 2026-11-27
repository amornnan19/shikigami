use keyring::{Entry, Error};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

use super::{SecretError, SecretStore, SecretString, KEYCHAIN_SERVICE};

pub struct KeychainStore;

const ITEM_NOT_FOUND: i32 = -25300;

fn read_failure(error: &Error) -> SecretError {
    let source = match error {
        Error::PlatformFailure(source) | Error::NoStorageAccess(source) => Some(source),
        _ => None,
    };
    let status = source
        .and_then(|source| source.downcast_ref::<security_framework::base::Error>())
        .map(|error| error.code());
    SecretError::Read(status)
}

fn entry(account: &str) -> keyring::Result<Entry> {
    Entry::new(KEYCHAIN_SERVICE, account)
}

impl SecretStore for KeychainStore {
    fn exists(&self, account: &str) -> Result<bool, SecretError> {
        let found = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(KEYCHAIN_SERVICE)
            .account(account)
            .load_attributes(true)
            .limit(Limit::Max(1))
            .search();
        match found {
            Ok(items) => Ok(!items.is_empty()),
            Err(error) if error.code() == ITEM_NOT_FOUND => Ok(false),
            Err(error) => Err(SecretError::Read(Some(error.code()))),
        }
    }

    fn get(&self, account: &str) -> Result<Option<SecretString>, SecretError> {
        match entry(account).and_then(|entry| entry.get_password()) {
            Ok(value) => Ok(Some(SecretString::new(value))),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(read_failure(&error)),
        }
    }

    fn set(&self, account: &str, value: &SecretString) -> Result<(), SecretError> {
        entry(account)
            .and_then(|entry| entry.set_password(value.expose()))
            .map_err(|_| SecretError::Write)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match entry(account).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => Err(SecretError::Delete),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform_failure(code: i32) -> Error {
        Error::PlatformFailure(Box::new(security_framework::base::Error::from_code(code)))
    }

    #[test]
    fn a_platform_failure_keeps_the_os_status_code() {
        assert_eq!(
            read_failure(&platform_failure(-25293)),
            SecretError::Read(Some(-25293))
        );
        let denied =
            Error::NoStorageAccess(Box::new(security_framework::base::Error::from_code(-128)));
        assert_eq!(read_failure(&denied), SecretError::Read(Some(-128)));
    }

    #[test]
    fn a_failure_without_an_os_status_has_no_code() {
        let other = Error::PlatformFailure(Box::new(std::fmt::Error));
        assert_eq!(read_failure(&other), SecretError::Read(None));
        assert_eq!(
            read_failure(&Error::TooLong("x".into(), 1)),
            SecretError::Read(None)
        );
    }
}
