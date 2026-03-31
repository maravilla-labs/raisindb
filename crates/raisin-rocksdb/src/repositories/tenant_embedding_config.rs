//! RocksDB implementation of the TenantEmbeddingConfigStore trait.
//!
//! This module provides persistent storage for tenant embedding configurations
//! using RocksDB's TENANT_EMBEDDING_CONFIG column family.

use raisin_embeddings::config::TenantEmbeddingConfig;
use raisin_embeddings::storage::{Result, StorageError, TenantEmbeddingConfigStore};
use rocksdb::DB;
use std::sync::Arc;
use tracing::{error, info};

/// RocksDB-backed implementation of tenant embedding configuration storage.
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
/// Values are MessagePack-serialized `TenantEmbeddingConfig` structs.
pub struct TenantEmbeddingConfigRepository {
    db: Arc<DB>,
}

impl TenantEmbeddingConfigRepository {
    /// Create a new repository instance.
    ///
    /// # Arguments
    ///
    /// * `db` - Shared reference to the RocksDB instance
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Get the column family handle for tenant embedding configs.
    fn cf_handle(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db
            .cf_handle(crate::cf::TENANT_EMBEDDING_CONFIG)
            .ok_or_else(|| {
                StorageError::Storage(format!(
                    "Column family '{}' not found",
                    crate::cf::TENANT_EMBEDDING_CONFIG
                ))
            })
    }
}

impl TenantEmbeddingConfigStore for TenantEmbeddingConfigRepository {
    fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>> {
        let cf = self.cf_handle()?;
        let key = tenant_id.as_bytes();

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                info!("Retrieved embedding config for tenant: {}", tenant_id);

                // Deserialize using MessagePack
                let config: TenantEmbeddingConfig = rmp_serde::from_slice(&bytes)
                    .map_err(|e| StorageError::Deserialization(e.to_string()))?;

                Ok(Some(config))
            }
            Ok(None) => {
                info!("No embedding config found for tenant: {}", tenant_id);
                Ok(None)
            }
            Err(e) => {
                error!(
                    "Failed to retrieve embedding config for tenant {}: {}",
                    tenant_id, e
                );
                Err(StorageError::Storage(format!(
                    "Failed to read from storage: {}",
                    e
                )))
            }
        }
    }

    fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<()> {
        let cf = self.cf_handle()?;
        let key = config.tenant_id.as_bytes();

        // Serialize using MessagePack
        let bytes = rmp_serde::to_vec_named(config)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.db.put_cf(cf, key, bytes).map_err(|e| {
            error!(
                "Failed to store embedding config for tenant {}: {}",
                config.tenant_id, e
            );
            StorageError::Storage(format!("Failed to write to storage: {}", e))
        })?;

        info!(
            "Stored embedding config for tenant: {} (enabled: {})",
            config.tenant_id, config.enabled
        );

        Ok(())
    }

    fn delete_config(&self, tenant_id: &str) -> Result<()> {
        let cf = self.cf_handle()?;
        let key = tenant_id.as_bytes();

        self.db.delete_cf(cf, key).map_err(|e| {
            error!(
                "Failed to delete embedding config for tenant {}: {}",
                tenant_id, e
            );
            StorageError::Storage(format!("Failed to delete from storage: {}", e))
        })?;

        info!("Deleted embedding config for tenant: {}", tenant_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{open_db, RocksDBStorage};
    use raisin_embeddings::config::EmbeddingProvider;
    use tempfile::TempDir;

    fn setup_test_db() -> (TempDir, Arc<DB>) {
        let temp_dir = TempDir::new().unwrap();
        let db = open_db(temp_dir.path()).unwrap();
        (temp_dir, Arc::new(db))
    }

    #[test]
    fn test_store_and_retrieve_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());
        config.enabled = true;
        config.model = "text-embedding-3-large".to_string();
        config.dimensions = 3072;

        // Store config
        repo.set_config(&config).unwrap();

        // Retrieve config
        let retrieved = repo.get_config("test-tenant").unwrap();
        assert!(retrieved.is_some());

        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.tenant_id, "test-tenant");
        assert!(retrieved.enabled);
        assert_eq!(retrieved.model, "text-embedding-3-large");
        assert_eq!(retrieved.dimensions, 3072);
    }

    #[test]
    fn test_config_not_found() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        let result = repo.get_config("non-existent-tenant").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());
        config.enabled = false;

        // Store initial config
        repo.set_config(&config).unwrap();

        // Update config
        config.enabled = true;
        config.provider = EmbeddingProvider::Claude;
        repo.set_config(&config).unwrap();

        // Verify update
        let retrieved = repo.get_config("test-tenant").unwrap().unwrap();
        assert!(retrieved.enabled);
        assert_eq!(retrieved.provider, EmbeddingProvider::Claude);
    }

    #[test]
    fn test_delete_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        let config = TenantEmbeddingConfig::new("test-tenant".to_string());

        // Store config
        repo.set_config(&config).unwrap();
        assert!(repo.get_config("test-tenant").unwrap().is_some());

        // Delete config
        repo.delete_config("test-tenant").unwrap();
        assert!(repo.get_config("test-tenant").unwrap().is_none());
    }

    #[test]
    fn test_delete_non_existent_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        // Should not error
        repo.delete_config("non-existent-tenant").unwrap();
    }

    #[test]
    fn test_store_with_encrypted_api_key() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        let mut config = TenantEmbeddingConfig::new("test-tenant".to_string());
        config.api_key_encrypted = Some(vec![1, 2, 3, 4, 5]); // Simulated encrypted key

        repo.set_config(&config).unwrap();

        let retrieved = repo.get_config("test-tenant").unwrap().unwrap();
        assert_eq!(retrieved.api_key_encrypted, Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn test_multiple_tenants() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantEmbeddingConfigRepository::new(db);

        let config1 = TenantEmbeddingConfig::new("tenant-1".to_string());
        let mut config2 = TenantEmbeddingConfig::new("tenant-2".to_string());
        config2.enabled = true;

        repo.set_config(&config1).unwrap();
        repo.set_config(&config2).unwrap();

        let retrieved1 = repo.get_config("tenant-1").unwrap().unwrap();
        let retrieved2 = repo.get_config("tenant-2").unwrap().unwrap();

        assert!(!retrieved1.enabled);
        assert!(retrieved2.enabled);
    }
}
