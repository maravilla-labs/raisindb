//! Storage abstraction for tenant AI configurations.
//!
//! This module defines the storage trait for persisting and retrieving
//! tenant AI configurations. Implementations can use RocksDB, in-memory
//! storage, or other backends.

use crate::config::TenantAIConfig;
use async_trait::async_trait;
use thiserror::Error;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Configuration not found for tenant: {0}")]
    NotFound(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    #[error("Storage backend error: {0}")]
    BackendError(String),

    #[error("Invalid tenant ID: {0}")]
    InvalidTenantId(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Storage trait for tenant AI configurations.
///
/// This trait defines the interface for storing and retrieving tenant AI
/// configurations. Implementations should handle serialization, encryption
/// (if needed at the storage layer), and persistence.
///
/// # Example Implementation
///
/// ```rust,ignore
/// use raisin_ai::storage::{TenantAIConfigStore, Result};
/// use raisin_ai::config::TenantAIConfig;
/// use async_trait::async_trait;
/// use std::collections::HashMap;
/// use tokio::sync::RwLock;
///
/// struct InMemoryStore {
///     configs: RwLock<HashMap<String, TenantAIConfig>>,
/// }
///
/// #[async_trait]
/// impl TenantAIConfigStore for InMemoryStore {
///     async fn get_config(&self, tenant_id: &str) -> Result<TenantAIConfig> {
///         let configs = self.configs.read().await;
///         configs.get(tenant_id)
///             .cloned()
///             .ok_or_else(|| StorageError::NotFound(tenant_id.to_string()))
///     }
///
///     async fn set_config(&self, config: &TenantAIConfig) -> Result<()> {
///         let mut configs = self.configs.write().await;
///         configs.insert(config.tenant_id.clone(), config.clone());
///         Ok(())
///     }
///
///     async fn delete_config(&self, tenant_id: &str) -> Result<()> {
///         let mut configs = self.configs.write().await;
///         configs.remove(tenant_id)
///             .ok_or_else(|| StorageError::NotFound(tenant_id.to_string()))?;
///         Ok(())
///     }
///
///     async fn list_tenant_ids(&self) -> Result<Vec<String>> {
///         let configs = self.configs.read().await;
///         Ok(configs.keys().cloned().collect())
///     }
/// }
/// ```
#[async_trait]
pub trait TenantAIConfigStore: Send + Sync {
    /// Retrieves the AI configuration for a specific tenant.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - The unique identifier for the tenant
    ///
    /// # Returns
    ///
    /// The tenant's AI configuration, or an error if not found or if
    /// deserialization fails.
    ///
    /// # Errors
    ///
    /// - `StorageError::NotFound` if the tenant configuration doesn't exist
    /// - `StorageError::DeserializationError` if the stored data is invalid
    /// - `StorageError::BackendError` if the storage backend fails
    async fn get_config(&self, tenant_id: &str) -> Result<TenantAIConfig>;

    /// Stores or updates the AI configuration for a tenant.
    ///
    /// # Arguments
    ///
    /// * `config` - The tenant AI configuration to store
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error if storage fails.
    ///
    /// # Errors
    ///
    /// - `StorageError::SerializationError` if the config cannot be serialized
    /// - `StorageError::BackendError` if the storage backend fails
    async fn set_config(&self, config: &TenantAIConfig) -> Result<()>;

    /// Deletes the AI configuration for a tenant.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - The unique identifier for the tenant
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or an error if deletion fails.
    ///
    /// # Errors
    ///
    /// - `StorageError::NotFound` if the tenant configuration doesn't exist
    /// - `StorageError::BackendError` if the storage backend fails
    async fn delete_config(&self, tenant_id: &str) -> Result<()>;

    /// Lists all tenant IDs that have AI configurations.
    ///
    /// # Returns
    ///
    /// A vector of tenant IDs, or an error if listing fails.
    ///
    /// # Errors
    ///
    /// - `StorageError::BackendError` if the storage backend fails
    async fn list_tenant_ids(&self) -> Result<Vec<String>>;

    /// Checks if a tenant has an AI configuration.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - The unique identifier for the tenant
    ///
    /// # Returns
    ///
    /// `true` if the tenant has a configuration, `false` otherwise.
    ///
    /// # Note
    ///
    /// Default implementation uses `get_config` and checks for errors.
    /// Implementations may override for better performance.
    async fn exists(&self, tenant_id: &str) -> Result<bool> {
        match self.get_config(tenant_id).await {
            Ok(_) => Ok(true),
            Err(StorageError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AIProvider, AIProviderConfig};
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    // Simple in-memory implementation for testing
    struct TestStore {
        configs: RwLock<HashMap<String, TenantAIConfig>>,
    }

    impl TestStore {
        fn new() -> Self {
            Self {
                configs: RwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl TenantAIConfigStore for TestStore {
        async fn get_config(&self, tenant_id: &str) -> Result<TenantAIConfig> {
            let configs = self.configs.read().await;
            configs
                .get(tenant_id)
                .cloned()
                .ok_or_else(|| StorageError::NotFound(tenant_id.to_string()))
        }

        async fn set_config(&self, config: &TenantAIConfig) -> Result<()> {
            let mut configs = self.configs.write().await;
            configs.insert(config.tenant_id.clone(), config.clone());
            Ok(())
        }

        async fn delete_config(&self, tenant_id: &str) -> Result<()> {
            let mut configs = self.configs.write().await;
            configs
                .remove(tenant_id)
                .ok_or_else(|| StorageError::NotFound(tenant_id.to_string()))?;
            Ok(())
        }

        async fn list_tenant_ids(&self) -> Result<Vec<String>> {
            let configs = self.configs.read().await;
            Ok(configs.keys().cloned().collect())
        }
    }

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let store = TestStore::new();
        let mut config = TenantAIConfig::new("test-tenant".to_string());
        config
            .providers
            .push(AIProviderConfig::new(AIProvider::OpenAI));

        store.set_config(&config).await.unwrap();
        let retrieved = store.get_config("test-tenant").await.unwrap();
        assert_eq!(retrieved.tenant_id, "test-tenant");
        assert_eq!(retrieved.providers.len(), 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let store = TestStore::new();
        let result = store.get_config("nonexistent").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_delete() {
        let store = TestStore::new();
        let config = TenantAIConfig::new("test-tenant".to_string());

        store.set_config(&config).await.unwrap();
        store.delete_config("test-tenant").await.unwrap();

        let result = store.get_config("test-tenant").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_list_tenant_ids() {
        let store = TestStore::new();

        store
            .set_config(&TenantAIConfig::new("tenant1".to_string()))
            .await
            .unwrap();
        store
            .set_config(&TenantAIConfig::new("tenant2".to_string()))
            .await
            .unwrap();

        let mut ids = store.list_tenant_ids().await.unwrap();
        ids.sort();
        assert_eq!(ids, vec!["tenant1", "tenant2"]);
    }

    #[tokio::test]
    async fn test_exists() {
        let store = TestStore::new();
        let config = TenantAIConfig::new("test-tenant".to_string());

        assert!(!store.exists("test-tenant").await.unwrap());
        store.set_config(&config).await.unwrap();
        assert!(store.exists("test-tenant").await.unwrap());
    }
}
