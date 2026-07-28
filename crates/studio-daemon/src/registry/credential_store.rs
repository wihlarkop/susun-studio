use std::fmt;
#[cfg(test)]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "dev.susun.studio.registry";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegistryCredentialId(String);

impl RegistryCredentialId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, CredentialStoreError> {
        let value = value.into();
        Uuid::parse_str(&value).map_err(|_| CredentialStoreError::InvalidId)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([redacted])")
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CredentialStoreError {
    #[error("credential entry is missing")]
    Missing,
    #[error("credential store is unavailable")]
    Unavailable,
    #[error("credential store access was denied")]
    Denied,
    #[error("credential is too large for the platform store")]
    TooLarge,
    #[error("credential identifier is invalid")]
    InvalidId,
    #[error("credential store operation failed")]
    Platform,
}

pub trait RegistryCredentialStore: Send + Sync {
    fn put(
        &self,
        id: &RegistryCredentialId,
        secret: SecretValue,
    ) -> Result<(), CredentialStoreError>;
    fn get(&self, id: &RegistryCredentialId) -> Result<SecretValue, CredentialStoreError>;
    fn delete(&self, id: &RegistryCredentialId) -> Result<(), CredentialStoreError>;
    fn contains(&self, id: &RegistryCredentialId) -> Result<bool, CredentialStoreError>;
}

#[derive(Debug, Default)]
pub struct OsRegistryCredentialStore;

impl RegistryCredentialStore for OsRegistryCredentialStore {
    fn put(
        &self,
        id: &RegistryCredentialId,
        secret: SecretValue,
    ) -> Result<(), CredentialStoreError> {
        entry(id)?
            .set_password(secret.expose())
            .map_err(map_keyring_error)
    }

    fn get(&self, id: &RegistryCredentialId) -> Result<SecretValue, CredentialStoreError> {
        entry(id)?
            .get_password()
            .map(SecretValue::new)
            .map_err(map_keyring_error)
    }

    fn delete(&self, id: &RegistryCredentialId) -> Result<(), CredentialStoreError> {
        entry(id)?.delete_credential().map_err(map_keyring_error)
    }

    fn contains(&self, id: &RegistryCredentialId) -> Result<bool, CredentialStoreError> {
        match self.get(id) {
            Ok(_) => Ok(true),
            Err(CredentialStoreError::Missing) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn entry(id: &RegistryCredentialId) -> Result<keyring::Entry, CredentialStoreError> {
    keyring::Entry::new(KEYRING_SERVICE, id.as_str()).map_err(map_keyring_error)
}

fn map_keyring_error(error: keyring::Error) -> CredentialStoreError {
    match error {
        keyring::Error::NoEntry => CredentialStoreError::Missing,
        keyring::Error::NoDefaultStore | keyring::Error::NotSupportedByStore(_) => {
            CredentialStoreError::Unavailable
        }
        keyring::Error::NoStorageAccess(_) => CredentialStoreError::Denied,
        keyring::Error::TooLong(_, _) => CredentialStoreError::TooLarge,
        _ => CredentialStoreError::Platform,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct MemoryRegistryCredentialStore {
    entries: Arc<Mutex<HashMap<String, String>>>,
}

#[cfg(test)]
impl RegistryCredentialStore for MemoryRegistryCredentialStore {
    fn put(
        &self,
        id: &RegistryCredentialId,
        secret: SecretValue,
    ) -> Result<(), CredentialStoreError> {
        self.entries
            .lock()
            .map_err(|_| CredentialStoreError::Platform)?
            .insert(id.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }

    fn get(&self, id: &RegistryCredentialId) -> Result<SecretValue, CredentialStoreError> {
        self.entries
            .lock()
            .map_err(|_| CredentialStoreError::Platform)?
            .get(id.as_str())
            .cloned()
            .map(SecretValue::new)
            .ok_or(CredentialStoreError::Missing)
    }

    fn delete(&self, id: &RegistryCredentialId) -> Result<(), CredentialStoreError> {
        self.entries
            .lock()
            .map_err(|_| CredentialStoreError::Platform)?
            .remove(id.as_str())
            .map(|_| ())
            .ok_or(CredentialStoreError::Missing)
    }

    fn contains(&self, id: &RegistryCredentialId) -> Result<bool, CredentialStoreError> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| CredentialStoreError::Platform)?
            .contains_key(id.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_satisfies_put_get_overwrite_and_delete_contract() {
        let store = MemoryRegistryCredentialStore::default();
        let id = RegistryCredentialId::new();

        assert!(!store.contains(&id).expect("contains should succeed"));
        store
            .put(&id, SecretValue::new("first-secret"))
            .expect("put should succeed");
        assert!(store.contains(&id).expect("contains should succeed"));
        assert_eq!(
            store.get(&id).expect("get should succeed").expose(),
            "first-secret"
        );

        store
            .put(&id, SecretValue::new("replacement-secret"))
            .expect("overwrite should succeed");
        assert_eq!(
            store.get(&id).expect("get should succeed").expose(),
            "replacement-secret"
        );

        store.delete(&id).expect("delete should succeed");
        assert!(!store.contains(&id).expect("contains should succeed"));
        assert!(matches!(store.get(&id), Err(CredentialStoreError::Missing)));
    }

    #[test]
    fn credential_types_never_debug_secret_material() {
        let id = RegistryCredentialId::new();
        let secret = SecretValue::new("sentinel-registry-secret");

        assert!(!format!("{id:?}").contains("sentinel-registry-secret"));
        let debug = format!("{secret:?}");
        assert!(!debug.contains("sentinel-registry-secret"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn deleting_a_missing_memory_entry_is_typed() {
        let store = MemoryRegistryCredentialStore::default();
        let result = store.delete(&RegistryCredentialId::new());
        assert!(matches!(result, Err(CredentialStoreError::Missing)));
    }
}
