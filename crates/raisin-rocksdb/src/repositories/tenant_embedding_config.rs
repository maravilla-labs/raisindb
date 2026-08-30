//! RocksDB implementation of the TenantEmbeddingConfigStore trait.
//!
//! This module provides persistent storage for tenant embedding configurations
//! using RocksDB's TENANT_EMBEDDING_CONFIG column family.

use raisin_ai::config::{EmbeddingPartition, TenantAIConfig};
use raisin_embeddings::config::{
    EmbeddingDistanceMetric, EmbeddingQuantization, TenantEmbeddingConfig,
};
use raisin_embeddings::storage::{Result, StorageError, TenantEmbeddingConfigStore};
use raisin_hnsw::{IndexSpec, IndexSpecResolver, PartitionId};
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

/// [`IndexSpecResolver`] backed by the tenant embedding config.
///
/// This is the bridge that stops the HNSW engine from having a shape of its own.
/// It reads the SAME `TenantEmbeddingConfig` row that the `EmbeddingGenerate` job
/// handler reads before calling the provider, and that `REBUILD VECTOR INDEX` reads
/// before recreating an index — so the width an index is created at, the width the
/// job produces, and the width a rebuild uses are one value from one place and
/// cannot drift.
///
/// It answers for the METRIC and the QUANTIZATION too, and that is the point of
/// having one resolver rather than three. The metric had already drifted:
/// `distance_metric` was settable, rendered by `SHOW`, parsed by
/// `ALTER EMBEDDING CONFIG` and consumed at QUERY time, while every index was
/// BUILT with `DistanceMetric::default()` — so a tenant could configure a query
/// metric its own graph had never been built under. And `quantization` had been
/// a live control in the admin console with nowhere to land, so every index was
/// F32 by accident.
///
/// `enabled` is deliberately NOT consulted. Disabling embeddings must not change the
/// shape of vectors already indexed; it only stops new ones being generated.
pub struct TenantEmbeddingSpecResolver {
    repo: TenantEmbeddingConfigRepository,
    ai: crate::repositories::TenantAIConfigRepository,
}

impl TenantEmbeddingSpecResolver {
    /// Create a resolver over the shared RocksDB handle.
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            repo: TenantEmbeddingConfigRepository::new(Arc::clone(&db)),
            ai: crate::repositories::TenantAIConfigRepository::new(db),
        }
    }

    /// Read the tenant's embedding config, logging and swallowing a failure.
    ///
    /// A resolver that cannot answer must never make an otherwise healthy index
    /// unloadable, so every failure path here returns `None` and the engine uses
    /// its fallback.
    fn config(&self, tenant_id: &str) -> Option<TenantEmbeddingConfig> {
        match self.repo.get_config(tenant_id) {
            Ok(config) => config,
            Err(e) => {
                error!(
                    "Failed to read the embedding config for tenant {}: {} - falling back",
                    tenant_id, e
                );
                None
            }
        }
    }

    /// Read the tenant AI config, but only when the embedding config points at
    /// one. Synchronous on purpose: this runs on the engine's cache-miss path,
    /// which cannot await. The underlying read is a plain `get_cf`.
    fn ai_config_if_referenced(
        &self,
        tenant_id: &str,
        config: &TenantEmbeddingConfig,
    ) -> Option<TenantAIConfig> {
        if !config.uses_unified_provider() {
            return None;
        }
        match self.ai.get_config_blocking(tenant_id) {
            Ok(c) => Some(c),
            Err(e) => {
                error!(
                    "Failed to read the AI config for tenant {} while resolving its embedding \
                     partition: {} - the index partition cannot be named",
                    tenant_id, e
                );
                None
            }
        }
    }
}

/// Map the config's metric onto the engine's.
///
/// One conversion, here, because the metric an index is BUILT with has to be the
/// metric its queries are answered under.
fn to_hnsw_metric(metric: EmbeddingDistanceMetric) -> raisin_hnsw::DistanceMetric {
    match metric {
        EmbeddingDistanceMetric::Cosine => raisin_hnsw::DistanceMetric::Cosine,
        EmbeddingDistanceMetric::L2 => raisin_hnsw::DistanceMetric::L2,
        EmbeddingDistanceMetric::InnerProduct => raisin_hnsw::DistanceMetric::InnerProduct,
        EmbeddingDistanceMetric::Hamming => raisin_hnsw::DistanceMetric::Hamming,
    }
}

/// Map the config's scalar precision onto the engine's.
fn to_hnsw_quantization(q: EmbeddingQuantization) -> raisin_hnsw::QuantizationType {
    match q {
        EmbeddingQuantization::F32 => raisin_hnsw::QuantizationType::F32,
        EmbeddingQuantization::F16 => raisin_hnsw::QuantizationType::F16,
        EmbeddingQuantization::Int8 => raisin_hnsw::QuantizationType::Int8,
    }
}

impl IndexSpecResolver for TenantEmbeddingSpecResolver {
    fn spec_for(
        &self,
        tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _partition: &PartitionId,
    ) -> Option<IndexSpec> {
        // Config is keyed by tenant alone today; repo/branch/partition are taken
        // so the trait does not have to change when a second partition (image
        // vectors) gets its own model. When it does, THIS is the one place that
        // learns to branch on the partition — not a second resolver.
        let config = self.config(tenant_id)?;
        Some(
            IndexSpec::new(config.dimensions)
                .with_metric(to_hnsw_metric(config.distance_metric))
                .with_quantization(to_hnsw_quantization(config.quantization)),
        )
    }

    fn default_text_partition(
        &self,
        tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
    ) -> Option<PartitionId> {
        let config = self.config(tenant_id)?;
        let ai_config = self.ai_config_if_referenced(tenant_id, &config);

        // Key-less identity resolution: the partition is a public hash of
        // provider/model/dimensions, so it needs no master key — which is what
        // lets this run on a synchronous path. It shares its decision with
        // `resolve_settings` (see `raisin_embeddings::resolve::resolve_shape`),
        // so the read side and the write side cannot land in different
        // partitions.
        let embedder =
            match raisin_embeddings::resolve::resolve_embedder_id(&config, ai_config.as_ref()) {
                Ok(id) => id,
                Err(e) => {
                    error!(
                        "Could not resolve the embedder identity for tenant {}: {} - no vector \
                         index partition can be named, so vector search will find nothing",
                        tenant_id, e
                    );
                    return None;
                }
            };

        Some(PartitionId::new(
            EmbeddingPartition::text(embedder).to_index_token(),
        ))
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
