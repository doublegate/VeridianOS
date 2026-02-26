//! Cryptographic Key Store
//!
//! Secure storage and management of cryptographic keys.

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use spin::RwLock;

use super::{CryptoError, CryptoResult};
use crate::sync::once_lock::OnceLock;

/// Key identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyId(pub u64);

/// Cryptographic key types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    Symmetric,
    SigningKey,
    VerifyingKey,
    EncryptionKey,
}

/// Stored cryptographic key
#[derive(Debug, Clone)]
pub struct Key {
    pub id: KeyId,
    pub key_type: KeyType,
    pub data: Vec<u8>,
    pub metadata: KeyMetadata,
}

/// Key metadata
#[derive(Debug, Clone)]
pub struct KeyMetadata {
    pub name: String,
    pub created: u64,
    pub expires: Option<u64>,
    pub usage_count: u64,
    pub max_usage: Option<u64>,
}

/// Key store for managing cryptographic keys
pub struct KeyStore {
    keys: RwLock<BTreeMap<KeyId, Key>>,
    next_id: RwLock<u64>,
}

impl KeyStore {
    /// Create new key store
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(BTreeMap::new()),
            next_id: RwLock::new(1),
        }
    }

    /// Store a new key
    pub fn store_key(&self, key_type: KeyType, data: Vec<u8>, name: String) -> CryptoResult<KeyId> {
        let mut next_id = self.next_id.write();
        let id = KeyId(*next_id);
        *next_id += 1;

        let key = Key {
            id,
            key_type,
            data,
            metadata: KeyMetadata {
                name,
                created: Self::current_time(),
                expires: None,
                usage_count: 0,
                max_usage: None,
            },
        };

        self.keys.write().insert(id, key);

        Ok(id)
    }

    /// Retrieve a key by ID
    pub fn get_key(&self, id: KeyId) -> CryptoResult<Key> {
        let keys = self.keys.read();

        keys.get(&id).cloned().ok_or(CryptoError::InvalidKey)
    }

    /// Delete a key
    pub fn delete_key(&self, id: KeyId) -> CryptoResult<()> {
        let mut keys = self.keys.write();

        keys.remove(&id).ok_or(CryptoError::InvalidKey).map(|_| ())
    }

    /// List all key IDs
    pub fn list_keys(&self) -> Vec<KeyId> {
        self.keys.read().keys().copied().collect()
    }

    /// Increment key usage count
    pub fn use_key(&self, id: KeyId) -> CryptoResult<()> {
        let mut keys = self.keys.write();

        if let Some(key) = keys.get_mut(&id) {
            key.metadata.usage_count += 1;

            // Check if max usage exceeded
            if let Some(max_usage) = key.metadata.max_usage {
                if key.metadata.usage_count > max_usage {
                    return Err(CryptoError::InvalidKey);
                }
            }

            // Check if key expired
            if let Some(expires) = key.metadata.expires {
                if Self::current_time() > expires {
                    return Err(CryptoError::InvalidKey);
                }
            }

            Ok(())
        } else {
            Err(CryptoError::InvalidKey)
        }
    }

    /// Set key expiration time
    pub fn set_expiration(&self, id: KeyId, expires: u64) -> CryptoResult<()> {
        let mut keys = self.keys.write();

        if let Some(key) = keys.get_mut(&id) {
            key.metadata.expires = Some(expires);
            Ok(())
        } else {
            Err(CryptoError::InvalidKey)
        }
    }

    /// Set maximum usage count for key
    pub fn set_max_usage(&self, id: KeyId, max_usage: u64) -> CryptoResult<()> {
        let mut keys = self.keys.write();

        if let Some(key) = keys.get_mut(&id) {
            key.metadata.max_usage = Some(max_usage);
            Ok(())
        } else {
            Err(CryptoError::InvalidKey)
        }
    }

    fn current_time() -> u64 {
        crate::arch::timer::get_timestamp_secs()
    }
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Global key store
static GLOBAL_KEYSTORE: RwLock<Option<KeyStore>> = RwLock::new(None);

/// Initialize key store
pub fn init() -> CryptoResult<()> {
    let keystore = KeyStore::new();
    *GLOBAL_KEYSTORE.write() = Some(keystore);
    Ok(())
}

/// Global key store
static KEYSTORE_STORAGE: OnceLock<KeyStore> = OnceLock::new();

/// Get global key store
pub fn get_keystore() -> &'static KeyStore {
    KEYSTORE_STORAGE.get_or_init(KeyStore::new)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_keystore_operations() {
        let store = KeyStore::new();

        // Store a key
        let key_data = vec![0x42u8; 32];
        let id = store
            .store_key(
                KeyType::Symmetric,
                key_data.clone(),
                String::from("test_key"),
            )
            .unwrap();

        // Retrieve the key
        let retrieved = store.get_key(id).unwrap();
        assert_eq!(retrieved.data, key_data);

        // Delete the key
        store.delete_key(id).unwrap();

        // Should fail to retrieve deleted key
        assert!(store.get_key(id).is_err());
    }
}
