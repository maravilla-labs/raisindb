//! RocksDB implementation of tenant authentication configuration storage.
//!
//! This module provides persistent storage for tenant authentication configurations
//! using RocksDB's TENANT_AUTH_CONFIG column family.
//!
//! # Key Format
//!
//! Keys are simply the tenant ID as UTF-8 bytes.
//!
//! # Value Format
//!
//! Values are MessagePack-serialized `TenantAuthConfig` structs.

use raisin_error::Result;
use raisin_models::auth::TenantAuthConfig;
use rocksdb::DB;
use std::sync::Arc;
use tracing::{debug, error, info};

/// RocksDB-backed implementation of tenant authentication configuration storage.
///
/// This implementation uses MessagePack serialization and stores configurations
/// in a dedicated column family for isolation and performance.
pub struct TenantAuthConfigRepository {
    db: Arc<DB>,
}

impl TenantAuthConfigRepository {
    /// Create a new repository instance.
    ///
    /// # Arguments
    ///
    /// * `db` - Shared reference to the RocksDB instance
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Get the column family handle for tenant auth configs.
    fn cf_handle(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db
            .cf_handle(crate::cf::TENANT_AUTH_CONFIG)
            .ok_or_else(|| {
                raisin_error::Error::storage(format!(
                    "Column family '{}' not found",
                    crate::cf::TENANT_AUTH_CONFIG
                ))
            })
    }

    /// Get the authentication configuration for a tenant.
    ///
    /// Returns `None` if no configuration exists for the tenant.
    pub async fn get_config(&self, tenant_id: &str) -> Result<Option<TenantAuthConfig>> {
        let cf = self.cf_handle()?;
        let key = tenant_id.as_bytes();

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                debug!("Retrieved auth config for tenant: {}", tenant_id);

                let config: TenantAuthConfig = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;

                Ok(Some(config))
            }
            Ok(None) => {
                debug!("No auth config found for tenant: {}", tenant_id);
                Ok(None)
            }
            Err(e) => {
                error!(
                    "Failed to retrieve auth config for tenant {}: {}",
                    tenant_id, e
                );
                Err(raisin_error::Error::storage(format!(
                    "Failed to read from storage: {}",
                    e
                )))
            }
        }
    }

    /// Store the authentication configuration for a tenant.
    pub async fn set_config(&self, config: &TenantAuthConfig) -> Result<()> {
        let cf = self.cf_handle()?;
        let key = config.tenant_id.as_bytes();

        // Serialize using MessagePack with field names (more robust to struct changes)
        let bytes = rmp_serde::to_vec_named(config)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db.put_cf(cf, key, bytes).map_err(|e| {
            error!(
                "Failed to store auth config for tenant {}: {}",
                config.tenant_id, e
            );
            raisin_error::Error::storage(format!("Failed to write to storage: {}", e))
        })?;

        info!(
            "Stored auth config for tenant: {} (providers: {}, anonymous: {})",
            config.tenant_id,
            config.providers.len(),
            config.anonymous_enabled
        );

        Ok(())
    }

    /// Delete the authentication configuration for a tenant.
    pub async fn delete_config(&self, tenant_id: &str) -> Result<()> {
        let cf = self.cf_handle()?;
        let key = tenant_id.as_bytes();

        self.db.delete_cf(cf, key).map_err(|e| {
            error!(
                "Failed to delete auth config for tenant {}: {}",
                tenant_id, e
            );
            raisin_error::Error::storage(format!("Failed to delete from storage: {}", e))
        })?;

        info!("Deleted auth config for tenant: {}", tenant_id);

        Ok(())
    }

    /// Check if anonymous access is enabled for a tenant.
    ///
    /// Returns `false` if no configuration exists (safe default).
    pub async fn is_anonymous_enabled(&self, tenant_id: &str) -> Result<bool> {
        Ok(self
            .get_config(tenant_id)
            .await?
            .map(|c| c.anonymous_enabled)
            .unwrap_or(false))
    }

    /// List all tenant IDs that have authentication configurations.
    pub async fn list_tenant_ids(&self) -> Result<Vec<String>> {
        let cf = self.cf_handle()?;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        let mut tenant_ids = Vec::new();
        for item in iter {
            match item {
                Ok((key, _)) => {
                    let tenant_id = String::from_utf8(key.to_vec()).map_err(|e| {
                        raisin_error::Error::storage(format!("Invalid UTF-8 in key: {}", e))
                    })?;
                    tenant_ids.push(tenant_id);
                }
                Err(e) => {
                    error!("Failed to iterate auth configs: {}", e);
                    return Err(raisin_error::Error::storage(format!(
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
    use crate::open_db;
    use raisin_models::auth::AuthProviderConfig;
    use tempfile::TempDir;

    fn setup_test_db() -> (TempDir, Arc<DB>) {
        let temp_dir = TempDir::new().unwrap();
        let db = open_db(temp_dir.path()).unwrap();
        (temp_dir, Arc::new(db))
    }

    #[tokio::test]
    async fn test_store_and_retrieve_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAuthConfigRepository::new(db);

        let mut config = TenantAuthConfig::new("test-tenant".to_string());
        config.providers.push(AuthProviderConfig::local());
        config.anonymous_enabled = true;

        // Store config
        repo.set_config(&config).await.unwrap();

        // Retrieve config
        let retrieved = repo.get_config("test-tenant").await.unwrap().unwrap();
        assert_eq!(retrieved.tenant_id, "test-tenant");
        assert_eq!(retrieved.providers.len(), 1);
        assert!(retrieved.anonymous_enabled);
    }

    #[tokio::test]
    async fn test_config_not_found() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAuthConfigRepository::new(db);

        let result = repo.get_config("non-existent-tenant").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_is_anonymous_enabled() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAuthConfigRepository::new(db);

        // No config = anonymous disabled
        assert!(!repo.is_anonymous_enabled("test-tenant").await.unwrap());

        // Config with anonymous enabled
        let mut config = TenantAuthConfig::new("test-tenant".to_string());
        config.anonymous_enabled = true;
        repo.set_config(&config).await.unwrap();

        assert!(repo.is_anonymous_enabled("test-tenant").await.unwrap());

        // Config with anonymous disabled
        config.anonymous_enabled = false;
        repo.set_config(&config).await.unwrap();

        assert!(!repo.is_anonymous_enabled("test-tenant").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_config() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAuthConfigRepository::new(db);

        let config = TenantAuthConfig::new("test-tenant".to_string());

        // Store config
        repo.set_config(&config).await.unwrap();
        assert!(repo.get_config("test-tenant").await.unwrap().is_some());

        // Delete config
        repo.delete_config("test-tenant").await.unwrap();

        // Verify deletion
        assert!(repo.get_config("test-tenant").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_list_tenant_ids() {
        let (_temp_dir, db) = setup_test_db();
        let repo = TenantAuthConfigRepository::new(db);

        repo.set_config(&TenantAuthConfig::new("tenant1".to_string()))
            .await
            .unwrap();
        repo.set_config(&TenantAuthConfig::new("tenant2".to_string()))
            .await
            .unwrap();

        let mut ids = repo.list_tenant_ids().await.unwrap();
        ids.sort();
        assert_eq!(ids, vec!["tenant1", "tenant2"]);
    }
}
