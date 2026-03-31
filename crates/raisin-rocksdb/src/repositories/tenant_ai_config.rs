//! RocksDB implementation of the TenantAIConfigStore trait.
//!
//! This module provides persistent storage for tenant AI/LLM configurations
//! using RocksDB's TENANT_AI_CONFIG column family.

use async_trait::async_trait;
use raisin_ai::config::TenantAIConfig;
use raisin_ai::storage::{Result, StorageError, TenantAIConfigStore};
use rocksdb::DB;
use std::sync::Arc;
use tracing::{error, info};

/// RocksDB-backed implementation of tenant AI configuration storage.
///
/// This implementation uses MessagePack serialization and stores configurations
/// in a dedicated column family for isolation and performance.
///
/// # Key Format
///
/// Keys are simply the tenant ID as UTF-8 bytes.
///
/// # Value Format
///
/// Values are MessagePack-serialized `TenantAIConfig` structs.
pub struct TenantAIConfigRepository {
    db: Arc<DB>,
}

impl TenantAIConfigRepository {
    /// Create a new repository instance.
    ///
    /// # Arguments
    ///
    /// * `db` - Shared reference to the RocksDB instance
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Get the column family handle for tenant AI configs.
    fn cf_handle(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db
            .cf_handle(crate::cf::TENANT_AI_CONFIG)
            .ok_or_else(|| {
                StorageError::BackendError(format!(
                    "Column family '{}' not found",
                    crate::cf::TENANT_AI_CONFIG
                ))
            })
    }
}

#[async_trait]
impl TenantAIConfigStore for TenantAIConfigRepository {
    async fn get_config(&self, tenant_id: &str) -> Result<TenantAIConfig> {
        let cf = self.cf_handle()?;
        let key = tenant_id.as_bytes();

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                info!("Retrieved AI config for tenant: {}", tenant_id);

                // Deserialize using MessagePack
                let config: TenantAIConfig = rmp_serde::from_slice(&bytes)
                    .map_err(|e| StorageError::DeserializationError(e.to_string()))?;

                Ok(config)
            }
            Ok(None) => {
                info!("No AI config found for tenant: {}", tenant_id);
                Err(StorageError::NotFound(tenant_id.to_string()))
            }
            Err(e) => {
                error!(
                    "Failed to retrieve AI config for tenant {}: {}",
                    tenant_id, e
                );
                Err(StorageError::BackendError(format!(
                    "Failed to read from storage: {}",
                    e
                )))
            }
        }
    }

    async fn set_config(&self, config: &TenantAIConfig) -> Result<()> {
        let cf = self.cf_handle()?;
        let key = config.tenant_id.as_bytes();

        // Serialize using MessagePack with field names (more robust to struct changes)
        let bytes = rmp_serde::to_vec_named(config)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        self.db.put_cf(cf, key, bytes).map_err(|e| {
            error!(
                "Failed to store AI config for tenant {}: {}",
                config.tenant_id, e
            );
            StorageError::BackendError(format!("Failed to write to storage: {}", e))
        })?;

        info!(
            "Stored AI config for tenant: {} (providers: {})",
            config.tenant_id,
            config.providers.len()
        );

        Ok(())
    }

    async fn delete_config(&self, tenant_id: &str) -> Result<()> {
        let cf = self.cf_handle()?;
        let key = tenant_id.as_bytes();

        self.db.delete_cf(cf, key).map_err(|e| {
            error!("Failed to delete AI config for tenant {}: {}", tenant_id, e);
            StorageError::BackendError(format!("Failed to delete from storage: {}", e))
        })?;

        info!("Deleted AI config for tenant: {}", tenant_id);

        Ok(())
    }

    async fn list_tenant_ids(&self) -> Result<Vec<String>> {
        let cf = self.cf_handle()?;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        let mut tenant_ids = Vec::new();
        for item in iter {
            match item {
                Ok((key, _)) => {
                    let tenant_id = String::from_utf8(key.to_vec()).map_err(|e| {
                        StorageError::DeserializationError(format!("Invalid UTF-8 in key: {}", e))
                    })?;
                    tenant_ids.push(tenant_id);
                }
                Err(e) => {
                    error!("Failed to iterate AI configs: {}", e);
                    return Err(StorageError::BackendError(format!(
                        "Failed to iterate: {}",
                        e
                    )));
                }
            }
        }

        Ok(tenant_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{open_db, RocksDBStorage};
    use raisin_ai::config::{AIProvider, AIProviderConfig};
    use tempfile::TempDir;

    fn setup_test_db() -> (TempDir, Arc<DB>) {
        let temp_dir = TempDir::new().unwrap();
        let db = open_db(temp_dir.path()).unwrap();
        (temp_dir, Arc::new(db))
    }

    #[tokio::test]
    async fn test_store_and_retrieve_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        let mut config = TenantAIConfig::new("test-tenant".to_string());
        config
            .providers
            .push(AIProviderConfig::new(AIProvider::OpenAI));

        // Store config
        repo.set_config(&config).await.unwrap();

        // Retrieve config
        let retrieved = repo.get_config("test-tenant").await.unwrap();
        assert_eq!(retrieved.tenant_id, "test-tenant");
        assert_eq!(retrieved.providers.len(), 1);
        assert_eq!(retrieved.providers[0].provider, AIProvider::OpenAI);
    }

    #[tokio::test]
    async fn test_config_not_found() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        let result = repo.get_config("non-existent-tenant").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_update_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        let mut config = TenantAIConfig::new("test-tenant".to_string());
        config
            .providers
            .push(AIProviderConfig::new(AIProvider::OpenAI));

        // Store initial config
        repo.set_config(&config).await.unwrap();

        // Update config
        config
            .providers
            .push(AIProviderConfig::new(AIProvider::Anthropic));
        repo.set_config(&config).await.unwrap();

        // Verify update
        let retrieved = repo.get_config("test-tenant").await.unwrap();
        assert_eq!(retrieved.providers.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        let config = TenantAIConfig::new("test-tenant".to_string());

        // Store config
        repo.set_config(&config).await.unwrap();
        assert!(repo.get_config("test-tenant").await.is_ok());

        // Delete config
        repo.delete_config("test-tenant").await.unwrap();

        // Verify deletion
        let result = repo.get_config("test-tenant").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_delete_non_existent_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        // Should not error
        repo.delete_config("non-existent-tenant").await.unwrap();
    }

    #[tokio::test]
    async fn test_store_with_encrypted_api_key() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        let mut config = TenantAIConfig::new("test-tenant".to_string());
        let mut provider = AIProviderConfig::new(AIProvider::OpenAI);
        provider.api_key_encrypted = Some(vec![1, 2, 3, 4, 5]); // Simulated encrypted key
        config.providers.push(provider);

        repo.set_config(&config).await.unwrap();

        let retrieved = repo.get_config("test-tenant").await.unwrap();
        assert_eq!(
            retrieved.providers[0].api_key_encrypted,
            Some(vec![1, 2, 3, 4, 5])
        );
    }

    #[tokio::test]
    async fn test_list_tenant_ids() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        repo.set_config(&TenantAIConfig::new("tenant1".to_string()))
            .await
            .unwrap();
        repo.set_config(&TenantAIConfig::new("tenant2".to_string()))
            .await
            .unwrap();

        let mut ids = repo.list_tenant_ids().await.unwrap();
        ids.sort();
        assert_eq!(ids, vec!["tenant1", "tenant2"]);
    }

    #[tokio::test]
    async fn test_multiple_tenants() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAIConfigRepository::new(db);

        let mut config1 = TenantAIConfig::new("tenant-1".to_string());
        config1
            .providers
            .push(AIProviderConfig::new(AIProvider::OpenAI));

        let mut config2 = TenantAIConfig::new("tenant-2".to_string());
        config2
            .providers
            .push(AIProviderConfig::new(AIProvider::Anthropic));

        repo.set_config(&config1).await.unwrap();
        repo.set_config(&config2).await.unwrap();

        let retrieved1 = repo.get_config("tenant-1").await.unwrap();
        let retrieved2 = repo.get_config("tenant-2").await.unwrap();

        assert_eq!(retrieved1.providers[0].provider, AIProvider::OpenAI);
        assert_eq!(retrieved2.providers[0].provider, AIProvider::Anthropic);
    }
}
