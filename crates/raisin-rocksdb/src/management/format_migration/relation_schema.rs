//! Relation schema migration
//!
//! Updates RelationRef and FullRelation structures to include explicit node types
//! and semantic relationship types.
//!
//! # Changes
//! - RelationRef: 3 fields -> 5 fields (added target_node_type, relation_type)
//! - FullRelation: 5 fields -> 8 fields (added source_node_type, target_node_type, relation_type)

use raisin_error::{Error, Result};
use raisin_models::nodes::RelationRef;
use rocksdb::{WriteBatch, DB};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use super::old_types::{OldFullRelation, OldRelationRef};
use super::BATCH_SIZE;
use crate::cf;

const RELATION_SCHEMA_MIGRATION_MARKER: &str = ".migrated_relation_schema_v2";

/// Run relation schema migration: Old 3/5-field format -> New 5/8-field format
///
/// This migration updates RelationRef and FullRelation structures to include
/// explicit node types and semantic relationship types.
///
/// # Arguments
/// * `db` - RocksDB database instance
/// * `data_dir` - Data directory path (for marker file)
pub async fn run_relation_schema_migration(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(RELATION_SCHEMA_MIGRATION_MARKER);

    if marker_path.exists() {
        info!("Relation schema migration already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting relation schema migration: Old format -> New format with node types");

    migrate_relation_index(&db)?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Relation schema migration completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Migrate RELATION_INDEX column family with special handling for corrupted data
///
/// This function handles three data formats:
/// 1. Plain workspace strings (from old bug in nodes/crud.rs)
/// 2. JSON-formatted RelationRefs (from old JSON era)
/// 3. Already-migrated MessagePack RelationRefs (skip)
pub(super) fn migrate_relation_index(db: &DB) -> Result<()> {
    info!("Migrating column family: relation_index (RelationRef with bug fix)");

    let cf = db
        .cf_handle(cf::RELATION_INDEX)
        .ok_or_else(|| Error::storage("Column family not found: relation_index"))?;

    let mut total_keys = 0;
    let mut migrated_keys = 0;
    let mut skipped_keys = 0;
    let mut fixed_keys = 0;
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        total_keys += 1;

        if value.as_ref() == b"T" || value.is_empty() {
            skipped_keys += 1;
            continue;
        }

        // Try to deserialize as new MessagePack format (already migrated with 5 fields)
        if rmp_serde::from_slice::<RelationRef>(&value).is_ok() {
            skipped_keys += 1;
            continue;
        }

        let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        let is_global_index =
            key_parts.len() >= 5 && String::from_utf8_lossy(key_parts[3]) == "rel_global";

        if is_global_index {
            if let Some(result) =
                try_migrate_global_index(&key_parts, &value, cf, &mut batch, &mut batch_count, db)?
            {
                migrated_keys += result;
                if batch_count >= BATCH_SIZE {
                    flush_batch(db, &mut batch, &mut batch_count, migrated_keys)?;
                }
                continue;
            }
        } else if let Some(result) = try_migrate_forward_reverse_index(
            &key_parts,
            &value,
            cf,
            &mut batch,
            &mut batch_count,
            db,
        )? {
            migrated_keys += result;
            if batch_count >= BATCH_SIZE {
                flush_batch(db, &mut batch, &mut batch_count, migrated_keys)?;
            }
            continue;
        }

        // Try to deserialize from JSON
        if let Ok(relation) = serde_json::from_slice::<RelationRef>(&value) {
            let new_value = rmp_serde::to_vec(&relation)
                .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

            batch.put_cf(cf, &key, &new_value);
            migrated_keys += 1;
            batch_count += 1;
        } else {
            // Not JSON or MessagePack - likely plain workspace string from bug
            let fixed = try_fix_corrupted_relation(
                &key,
                &value,
                cf,
                &mut batch,
                &mut batch_count,
                &mut skipped_keys,
            )?;
            fixed_keys += fixed;
        }

        if batch_count >= BATCH_SIZE {
            flush_batch(db, &mut batch, &mut batch_count, migrated_keys)?;
        }
    }

    if batch_count > 0 {
        db.write(batch)
            .map_err(|e| Error::storage(format!("Final batch write error: {}", e)))?;
    }

    info!(
        "Completed relation_index: {} total keys, {} migrated from JSON, {} fixed from bug, {} skipped",
        total_keys, migrated_keys, fixed_keys, skipped_keys
    );

    Ok(())
}

/// Attempt to migrate a global index entry from old MessagePack format
fn try_migrate_global_index(
    key_parts: &[&[u8]],
    value: &[u8],
    cf: &rocksdb::ColumnFamily,
    batch: &mut WriteBatch,
    batch_count: &mut usize,
    _db: &DB,
) -> Result<Option<usize>> {
    if let Ok(old_full) = rmp_serde::from_slice::<OldFullRelation>(value) {
        if key_parts.len() >= 10 {
            let old_relation_type = String::from_utf8_lossy(key_parts[4]).to_string();

            let full_relation = raisin_models::nodes::FullRelation::new(
                old_full.source_id,
                old_full.source_workspace,
                old_relation_type.clone(),
                old_full.target_id,
                old_full.target_workspace,
                old_relation_type,
                "references".to_string(),
                old_full.weight,
            );

            let new_value = rmp_serde::to_vec(&full_relation)
                .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

            // Reconstruct key from parts for put_cf
            let key = reconstruct_key(key_parts);
            batch.put_cf(cf, &key, &new_value);
            *batch_count += 1;
            return Ok(Some(1));
        }
    }
    Ok(None)
}

/// Attempt to migrate a forward/reverse index entry from old MessagePack format
fn try_migrate_forward_reverse_index(
    key_parts: &[&[u8]],
    value: &[u8],
    cf: &rocksdb::ColumnFamily,
    batch: &mut WriteBatch,
    batch_count: &mut usize,
    _db: &DB,
) -> Result<Option<usize>> {
    if let Ok(old_relation) = rmp_serde::from_slice::<OldRelationRef>(value) {
        if key_parts.len() >= 9 {
            let prefix = String::from_utf8_lossy(key_parts[4]).to_string();
            let old_relation_type = String::from_utf8_lossy(key_parts[6]).to_string();

            let relation = if prefix == "rel" {
                RelationRef::new(
                    old_relation.target,
                    old_relation.workspace,
                    old_relation_type,
                    "references".to_string(),
                    old_relation.weight,
                )
            } else if prefix == "rel_rev" {
                let source_node_id = String::from_utf8_lossy(key_parts[8]).to_string();
                let source_workspace = String::from_utf8_lossy(key_parts[3]).to_string();

                RelationRef::new(
                    source_node_id,
                    source_workspace,
                    old_relation_type,
                    "references".to_string(),
                    old_relation.weight,
                )
            } else {
                warn!("Unknown relation key prefix: {}, skipping", prefix);
                return Ok(None);
            };

            let new_value = rmp_serde::to_vec(&relation)
                .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

            let key = reconstruct_key(key_parts);
            batch.put_cf(cf, &key, &new_value);
            *batch_count += 1;
            return Ok(Some(1));
        }
    }
    Ok(None)
}

/// Try to fix a corrupted relation entry (plain workspace string from old bug)
fn try_fix_corrupted_relation(
    key: &[u8],
    value: &[u8],
    cf: &rocksdb::ColumnFamily,
    batch: &mut WriteBatch,
    batch_count: &mut usize,
    skipped_keys: &mut usize,
) -> Result<usize> {
    let target_workspace = match std::str::from_utf8(value) {
        Ok(s) => s,
        Err(_) => {
            warn!("Failed to parse value as UTF-8 string, skipping");
            *skipped_keys += 1;
            return Ok(0);
        }
    };

    let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();

    if key_parts.len() < 9 {
        warn!(
            "Invalid key structure (expected 9 parts, got {}), skipping",
            key_parts.len()
        );
        *skipped_keys += 1;
        return Ok(0);
    }

    let prefix = String::from_utf8_lossy(key_parts[4]).to_string();

    let (relation_type_idx, target_id_idx) = if prefix == "rel" {
        (6, 8)
    } else if prefix == "rel_rev" {
        (6, 5)
    } else {
        warn!("Unknown relation key prefix: {}, skipping", prefix);
        *skipped_keys += 1;
        return Ok(0);
    };

    let relation_type = String::from_utf8_lossy(key_parts[relation_type_idx]).to_string();
    let target_id = String::from_utf8_lossy(key_parts[target_id_idx]).to_string();

    let relation = RelationRef::new(
        target_id,
        target_workspace.to_string(),
        relation_type,
        "references".to_string(),
        None,
    );

    let new_value = rmp_serde::to_vec(&relation)
        .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

    batch.put_cf(cf, key, &new_value);
    *batch_count += 1;

    Ok(1)
}

/// Reconstruct a key from its null-separated parts
fn reconstruct_key(parts: &[&[u8]]) -> Vec<u8> {
    let mut key = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            key.push(0);
        }
        key.extend_from_slice(part);
    }
    key
}

/// Flush the current write batch and reset counters
fn flush_batch(
    db: &DB,
    batch: &mut WriteBatch,
    batch_count: &mut usize,
    migrated_keys: usize,
) -> Result<()> {
    let old_batch = std::mem::take(batch);
    db.write(old_batch)
        .map_err(|e| Error::storage(format!("Batch write error: {}", e)))?;
    info!("  Progress: {} migrated", migrated_keys);
    *batch_count = 0;
    Ok(())
}
