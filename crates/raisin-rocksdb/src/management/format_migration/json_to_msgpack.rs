//! JSON to MessagePack serialization migration
//!
//! Migrates all serialized data in column families from JSON to MessagePack format.

use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use rocksdb::{WriteBatch, DB};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use super::BATCH_SIZE;
use crate::cf;

const MIGRATION_MARKER_FILE: &str = ".migrated_to_msgpack";

/// Run one-time format migration from JSON to MessagePack
///
/// This function:
/// 1. Checks if migration has already been completed (.migrated_to_msgpack marker)
/// 2. If not, migrates all serialized data in column families from JSON to MessagePack
/// 3. Creates marker file on success
///
/// # Arguments
/// * `db` - RocksDB database instance
/// * `data_dir` - Data directory path (for marker file)
pub async fn run_migration(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(MIGRATION_MARKER_FILE);

    if marker_path.exists() {
        info!("Format migration already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting one-time format migration: JSON → MessagePack");

    migrate_cf_generic::<Node>(&db, cf::NODES, "Node")?;
    migrate_node_types(&db)?;
    migrate_cf_generic::<Workspace>(&db, cf::WORKSPACES, "Workspace")?;

    // Special migration for RELATION_INDEX to handle corrupted data from old bug
    super::relation_schema::migrate_relation_index(&db)?;

    // Migrate branches, revisions, trees, etc.
    migrate_cf_value(&db, cf::BRANCHES, "Branch")?;
    migrate_cf_value(&db, cf::REVISIONS, "Revision")?;
    migrate_cf_value(&db, cf::TREES, "Tree")?;
    migrate_cf_value(&db, cf::REGISTRY, "Registry")?;
    migrate_cf_value(&db, cf::WORKSPACE_DELTAS, "WorkspaceDelta")?;
    migrate_cf_value(&db, cf::FULLTEXT_JOBS, "FulltextJob")?;
    migrate_cf_value(&db, cf::REFERENCE_INDEX, "ReferenceIndex")?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Format migration completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Migrate a column family with a specific type
fn migrate_cf_generic<T>(db: &DB, cf_name: &str, type_name: &str) -> Result<()>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    info!("Migrating column family: {} ({})", cf_name, type_name);

    let cf = db
        .cf_handle(cf_name)
        .ok_or_else(|| Error::storage(format!("Column family not found: {}", cf_name)))?;

    let mut total_keys = 0;
    let mut migrated_keys = 0;
    let mut skipped_keys = 0;
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        total_keys += 1;

        if value.as_ref() == b"T" {
            skipped_keys += 1;
            continue;
        }

        let deserialized: T = match serde_json::from_slice(&value) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "Failed to deserialize {} at key (skipping): {}",
                    type_name, e
                );
                skipped_keys += 1;
                continue;
            }
        };

        let new_value = rmp_serde::to_vec(&deserialized)
            .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

        batch.put_cf(cf, &key, &new_value);
        migrated_keys += 1;
        batch_count += 1;

        if batch_count >= BATCH_SIZE {
            db.write(batch)
                .map_err(|e| Error::storage(format!("Batch write error: {}", e)))?;
            info!(
                "  Progress: {}/{} keys migrated in {}",
                migrated_keys, total_keys, cf_name
            );
            batch = WriteBatch::default();
            batch_count = 0;
        }
    }

    if batch_count > 0 {
        db.write(batch)
            .map_err(|e| Error::storage(format!("Final batch write error: {}", e)))?;
    }

    info!(
        "Completed {}: {} total keys, {} migrated, {} skipped",
        cf_name, total_keys, migrated_keys, skipped_keys
    );

    Ok(())
}

/// Migrate a column family using serde_json::Value (for flexible types)
fn migrate_cf_value(db: &DB, cf_name: &str, type_name: &str) -> Result<()> {
    info!("Migrating column family: {} ({})", cf_name, type_name);

    let cf = db
        .cf_handle(cf_name)
        .ok_or_else(|| Error::storage(format!("Column family not found: {}", cf_name)))?;

    let mut total_keys = 0;
    let mut migrated_keys = 0;
    let mut skipped_keys = 0;
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        total_keys += 1;

        if value.as_ref() == b"T" {
            skipped_keys += 1;
            continue;
        }

        // Skip if value looks like a plain string/bytes (not serialized JSON object)
        if !value.is_empty() && value[0] != b'{' && value[0] != b'[' {
            skipped_keys += 1;
            continue;
        }

        let deserialized: serde_json::Value = match serde_json::from_slice(&value) {
            Ok(v) => v,
            Err(_) => {
                skipped_keys += 1;
                continue;
            }
        };

        let new_value = rmp_serde::to_vec(&deserialized)
            .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

        batch.put_cf(cf, &key, &new_value);
        migrated_keys += 1;
        batch_count += 1;

        if batch_count >= BATCH_SIZE {
            db.write(batch)
                .map_err(|e| Error::storage(format!("Batch write error: {}", e)))?;
            info!(
                "  Progress: {}/{} keys migrated in {}",
                migrated_keys, total_keys, cf_name
            );
            batch = WriteBatch::default();
            batch_count = 0;
        }
    }

    if batch_count > 0 {
        db.write(batch)
            .map_err(|e| Error::storage(format!("Final batch write error: {}", e)))?;
    }

    info!(
        "Completed {}: {} total keys, {} migrated, {} skipped",
        cf_name, total_keys, migrated_keys, skipped_keys
    );

    Ok(())
}

/// Migrate NODE_TYPES column family using clean slate approach
///
/// NodeTypes are system data defined in YAML files and recreated by NodeTypeInitHandler.
/// Rather than trying to migrate complex schema changes, we simply clear all entries
/// and let them be recreated fresh from YAML files on server startup.
fn migrate_node_types(db: &DB) -> Result<()> {
    info!("Migrating column family: node_types (clean slate approach)");
    info!("  NodeTypes will be cleared and recreated from YAML files by NodeTypeInitHandler");

    let cf = db
        .cf_handle(cf::NODE_TYPES)
        .ok_or_else(|| Error::storage("Column family not found: node_types"))?;

    let mut batch = WriteBatch::default();
    let mut deleted_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, _) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        batch.delete_cf(cf, &key);
        deleted_count += 1;
    }

    if deleted_count > 0 {
        db.write(batch)
            .map_err(|e| Error::storage(format!("Batch delete error: {}", e)))?;
        info!("Cleared {} NodeType entries from database", deleted_count);
        info!("  NodeTypes will be recreated from YAML files (raisin:Folder, raisin:Page, raisin:Asset)");
    } else {
        info!("NODE_TYPES CF was already empty");
    }

    Ok(())
}
