//! Storage trait for tenant embedding configurations.
//!
//! This module defines the trait that storage backends must implement to
//! persist and retrieve tenant embedding configurations.

use crate::config::TenantEmbeddingConfig;
use thiserror::Error;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Trait for storing and retrieving tenant embedding configurations.
///
/// This trait abstracts the storage layer, allowing different implementations
/// (e.g., RocksDB, PostgreSQL, in-memory) to be used.
///
/// # Thread Safety
///
/// Implementations must be thread-safe (Send + Sync) to support concurrent access.
///
/// # Example Implementation
///
/// ```rust,ignore
/// use raisin_embeddings::storage::{TenantEmbeddingConfigStore, Result};
/// use raisin_embeddings::config::TenantEmbeddingConfig;
/// use std::collections::HashMap;
/// use std::sync::RwLock;
///
/// struct InMemoryStore {
///     configs: RwLock<HashMap<String, TenantEmbeddingConfig>>,
/// }
///
/// impl TenantEmbeddingConfigStore for InMemoryStore {
///     fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>> {
///         let configs = self.configs.read().unwrap();
///         Ok(configs.get(tenant_id).cloned())
///     }
///
///     fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<()> {
///         let mut configs = self.configs.write().unwrap();
///         configs.insert(config.tenant_id.clone(), config.clone());
///         Ok(())
///     }
///
///     fn delete_config(&self, tenant_id: &str) -> Result<()> {
///         let mut configs = self.configs.write().unwrap();
///         configs.remove(tenant_id);
///         Ok(())
///     }
/// }
/// ```
pub trait TenantEmbeddingConfigStore: Send + Sync {
    /// Retrieve the embedding configuration for a tenant.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - The tenant identifier
    ///
    /// # Returns
    ///
    /// * `Ok(Some(config))` - Configuration found
    /// * `Ok(None)` - No configuration exists for this tenant
    /// * `Err(_)` - Storage error occurred
    fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>>;

    /// Store or update the embedding configuration for a tenant.
    ///
    /// If a configuration already exists for the tenant, it will be replaced.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration to store
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Configuration stored successfully
    /// * `Err(_)` - Storage error occurred
    fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<()>;

    /// Delete the embedding configuration for a tenant.
    ///
    /// This operation is idempotent - deleting a non-existent configuration
    /// should succeed without error.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - The tenant identifier
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Configuration deleted successfully (or didn't exist)
    /// * `Err(_)` - Storage error occurred
    fn delete_config(&self, tenant_id: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TenantEmbeddingConfig;
    use std::collections::HashMap;
    use std::sync::RwLock;

    // Simple in-memory implementation for testing
    struct InMemoryStore {
        configs: RwLock<HashMap<String, TenantEmbeddingConfig>>,
    }

    impl InMemoryStore {
        fn new() -> Self {
            Self {
                configs: RwLock::new(HashMap::new()),
            }
        }
    }

    impl TenantEmbeddingConfigStore for InMemoryStore {
        fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>> {
            let configs = self.configs.read().unwrap();
            Ok(configs.get(tenant_id).cloned())
        }

        fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<()> {
            let mut configs = self.configs.write().unwrap();
            configs.insert(config.tenant_id.clone(), config.clone());
            Ok(())
        }

        fn delete_config(&self, tenant_id: &str) -> Result<()> {
            let mut configs = self.configs.write().unwrap();
            configs.remove(tenant_id);
            Ok(())
        }
    }

    #[test]
    fn test_store_and_retrieve() {
        let store = InMemoryStore::new();
        let config = TenantEmbeddingConfig::new("test-tenant".to_string());

        // Initially no config
        assert!(store.get_config("test-tenant").unwrap().is_none());

        // Store config
        store.set_config(&config).unwrap();

        // Retrieve config
        let retrieved = store.get_config("test-tenant").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().tenant_id, "test-tenant");
    }

    #[test]
    fn test_update_config() {
        let store = InMemoryStore::new();
        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());

        // Store initial config
        store.set_config(&config).unwrap();

        // Update config
        config.enabled = true;
        store.set_config(&config).unwrap();

        // Verify update
        let retrieved = store.get_config("test-tenant").unwrap().unwrap();
        assert!(retrieved.enabled);
    }

    #[test]
    fn test_delete_config() {
        let store = InMemoryStore::new();
        let config = TenantEmbeddingConfig::new("test-tenant".to_string());

        // Store config
        store.set_config(&config).unwrap();
        assert!(store.get_config("test-tenant").unwrap().is_some());

        // Delete config
        store.delete_config("test-tenant").unwrap();
        assert!(store.get_config("test-tenant").unwrap().is_none());

        // Deleting again should not error
        store.delete_config("test-tenant").unwrap();
    }

    #[test]
    fn test_multiple_tenants() {
        let store = InMemoryStore::new();
        let config1 = TenantEmbeddingConfig::new("tenant-1".to_string());
        let config2 = TenantEmbeddingConfig::new("tenant-2".to_string());

        store.set_config(&config1).unwrap();
        store.set_config(&config2).unwrap();

        assert!(store.get_config("tenant-1").unwrap().is_some());
        assert!(store.get_config("tenant-2").unwrap().is_some());
        assert!(store.get_config("tenant-3").unwrap().is_none());
    }
}
