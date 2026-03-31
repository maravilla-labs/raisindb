//! Tenant embedding config migration
//!
//! Removes the deprecated node_type_settings field from TenantEmbeddingConfig.
//! Indexing is now controlled by NodeType schema instead.

use raisin_error::{Error, Result};
use rocksdb::DB;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use super::old_types::OldTenantEmbeddingConfig;
use super::BATCH_SIZE;

const TENANT_EMBEDDING_CONFIG_MIGRATION_MARKER: &str = ".migrated_tenant_embedding_config_v2";

/// Migration: Remove node_type_settings from TenantEmbeddingConfig
///
/// Background:
/// - Old format: TenantEmbeddingConfig with node_type_settings HashMap
/// - New format: TenantEmbeddingConfig without node_type_settings (now in NodeType schema)
///
/// This migration reads existing tenant embedding configs, removes the node_type_settings field,
/// and re-serializes in the new format.
pub async fn run_tenant_embedding_config_migration(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(TENANT_EMBEDDING_CONFIG_MIGRATION_MARKER);

    if marker_path.exists() {
        info!("Tenant embedding config migration already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting tenant embedding config migration: Remove node_type_settings field");

    migrate_tenant_embedding_config_cf(&db)?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Tenant embedding config migration completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Migrate TENANT_EMBEDDING_CONFIG column family to remove node_type_settings
fn migrate_tenant_embedding_config_cf(db: &Arc<DB>) -> Result<()> {
    use raisin_embeddings::config::TenantEmbeddingConfig;

    let cf_handle = db
        .cf_handle(crate::cf::TENANT_EMBEDDING_CONFIG)
        .ok_or_else(|| {
            Error::storage(format!(
                "Column family {} not found",
                crate::cf::TENANT_EMBEDDING_CONFIG
            ))
        })?;

    info!("  Migrating TENANT_EMBEDDING_CONFIG column family...");

    let mut batch_count = 0;
    let mut total_migrated = 0;
    let mut write_batch = rocksdb::WriteBatch::default();

    let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value_bytes) =
            item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

        let old_config: OldTenantEmbeddingConfig = match rmp_serde::from_slice(&value_bytes) {
            Ok(v) => v,
            Err(e) => {
                let tenant_id = String::from_utf8_lossy(&key);
                warn!(
                    "Failed to deserialize tenant embedding config for '{}': {} - DELETING corrupted config (you'll need to reconfigure)",
                    tenant_id, e
                );
                write_batch.delete_cf(cf_handle, &key);
                batch_count += 1;
                continue;
            }
        };

        let new_config = TenantEmbeddingConfig {
            tenant_id: old_config.tenant_id.clone(),
            enabled: old_config.enabled,
            ai_provider_ref: None,
            ai_model_ref: None,
            provider: old_config.provider,
            model: old_config.model,
            dimensions: old_config.dimensions,
            api_key_encrypted: old_config.api_key_encrypted,
            include_name: old_config.include_name,
            include_path: old_config.include_path,
            max_embeddings_per_repo: old_config.max_embeddings_per_repo,
            chunking: None,
            distance_metric: Default::default(),
        };

        let new_value = rmp_serde::to_vec(&new_config).map_err(|e| {
            Error::storage(format!(
                "Failed to serialize new tenant embedding config: {}",
                e
            ))
        })?;

        write_batch.put_cf(cf_handle, &key, &new_value);
        batch_count += 1;
        total_migrated += 1;

        if total_migrated <= 5 {
            info!(
                "  Migrated tenant embedding config: {} (had {} node_type_settings)",
                old_config.tenant_id,
                old_config.node_type_settings.len()
            );
        }

        if batch_count >= BATCH_SIZE {
            db.write(write_batch)
                .map_err(|e| Error::storage(format!("Failed to write batch: {}", e)))?;
            info!("    Migrated {} tenant configs...", total_migrated);
            write_batch = rocksdb::WriteBatch::default();
            batch_count = 0;
        }
    }

    if batch_count > 0 {
        db.write(write_batch)
            .map_err(|e| Error::storage(format!("Failed to write final batch: {}", e)))?;
    }

    info!(
        "  Migrated {} total tenant embedding configs",
        total_migrated
    );

    Ok(())
}
