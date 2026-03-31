//! Revision metadata migrations (v2 and v3)
//!
//! - v2: Adds operation field to RevisionMeta
//! - v3: Fixes variable-length MessagePack arrays caused by skip_serializing_if

use raisin_error::{Error, Result};
use rocksdb::{WriteBatch, DB};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use super::old_types::OldRevisionMeta;
use super::BATCH_SIZE;
use crate::cf;

const REVISION_META_MIGRATION_MARKER: &str = ".migrated_revision_meta_v2";
const REVISION_META_MIGRATION_MARKER_V3: &str = ".migrated_revision_meta_v3";

/// Migration: Add operation field to RevisionMeta
///
/// Background:
/// - Old format: RevisionMeta with 8 fields (no operation tracking)
/// - New format: RevisionMeta with 9 fields (includes operation metadata)
pub async fn run_revision_meta_migration(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(REVISION_META_MIGRATION_MARKER);

    if marker_path.exists() {
        info!("Revision metadata migration already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting revision metadata migration: Add operation field");

    migrate_revision_meta_cf(&db)?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Revision metadata migration completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Migration V3: Fix variable-length MessagePack arrays caused by skip_serializing_if
///
/// Background:
/// - Old RevisionMeta had `skip_serializing_if` on changed_nodes and operation
/// - This caused variable-length arrays: 7, 8, or 9 elements
/// - New RevisionMeta always serializes all 9 fields for consistency
pub async fn run_revision_meta_migration_v3(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(REVISION_META_MIGRATION_MARKER_V3);

    if marker_path.exists() {
        info!("Revision metadata migration v3 already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting revision metadata migration v3: Fix variable-length arrays");

    migrate_revision_meta_cf_v3(&db)?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Revision metadata migration v3 completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Filter for RevisionMeta keys: {tenant}\0{repo}\0revisions\0{revision}
fn is_revision_meta_key(key: &[u8]) -> bool {
    let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
    if key_parts.len() < 4 {
        return false;
    }
    String::from_utf8_lossy(key_parts[2]) == "revisions"
}

/// Convert an OldRevisionMeta to the new RevisionMeta format
fn convert_old_to_new(old: &OldRevisionMeta) -> raisin_storage::RevisionMeta {
    raisin_storage::RevisionMeta {
        revision: raisin_hlc::HLC::new(old.revision, 0),
        parent: old.parent.map(|ts| raisin_hlc::HLC::new(ts, 0)),
        merge_parent: None,
        branch: old.branch.clone(),
        timestamp: old.timestamp,
        actor: old.actor.clone(),
        message: old.message.clone(),
        is_system: old.is_system,
        changed_nodes: old.changed_nodes.clone(),
        changed_node_types: old.changed_node_types.clone(),
        changed_archetypes: old.changed_archetypes.clone(),
        changed_element_types: old.changed_element_types.clone(),
        operation: None,
    }
}

/// Migrate REVISIONS column family to add operation field
fn migrate_revision_meta_cf(db: &DB) -> Result<()> {
    info!("Migrating REVISIONS column family");

    let cf = db
        .cf_handle(cf::REVISIONS)
        .ok_or_else(|| Error::storage("REVISIONS column family not found"))?;

    let mut total_keys = 0;
    let mut migrated_keys = 0;
    let mut skipped_keys = 0;
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        total_keys += 1;

        if !is_revision_meta_key(&key) {
            skipped_keys += 1;
            continue;
        }

        let needs_migration = check_needs_v2_migration(&value);

        if !needs_migration {
            skipped_keys += 1;
            continue;
        }

        let old_revision: OldRevisionMeta = match rmp_serde::from_slice(&value) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "Failed to deserialize old revision metadata (skipping): {}",
                    e
                );
                skipped_keys += 1;
                continue;
            }
        };

        if migrated_keys < 5 {
            info!(
                "  Migrating revision {} on branch {}",
                old_revision.revision, old_revision.branch
            );
        }

        let new_revision = convert_old_to_new(&old_revision);

        let new_value = rmp_serde::to_vec(&new_revision)
            .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

        batch.put_cf(cf, &key, &new_value);
        migrated_keys += 1;
        batch_count += 1;

        if batch_count >= BATCH_SIZE {
            db.write(batch)
                .map_err(|e| Error::storage(format!("Batch write error: {}", e)))?;
            info!(
                "  Progress: {} migrated, {} total in REVISIONS",
                migrated_keys, total_keys
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
        "Completed REVISIONS: {} total keys, {} migrated, {} skipped",
        total_keys, migrated_keys, skipped_keys
    );

    Ok(())
}

/// Check if a value needs v2 migration (old 8-element format)
fn check_needs_v2_migration(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }
    match value[0] {
        0x89 => false, // 9 elements - new format
        0x88 => true,  // 8 elements - old format
        0xdc => {
            // array16
            if value.len() >= 3 {
                let count = u16::from_be_bytes([value[1], value[2]]);
                count == 8
            } else {
                false
            }
        }
        0xdd => {
            // array32
            if value.len() >= 5 {
                let count = u32::from_be_bytes([value[1], value[2], value[3], value[4]]);
                count == 8
            } else {
                false
            }
        }
        _ => rmp_serde::from_slice::<raisin_storage::RevisionMeta>(value).is_err(),
    }
}

/// Migrate REVISIONS column family to consistent 9-element format (v3)
fn migrate_revision_meta_cf_v3(db: &DB) -> Result<()> {
    info!("Migrating REVISIONS column family to consistent 9-element format");

    let cf = db
        .cf_handle(cf::REVISIONS)
        .ok_or_else(|| Error::storage("REVISIONS column family not found"))?;

    let mut total_keys = 0;
    let mut migrated_keys = 0;
    let mut skipped_keys = 0;
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        total_keys += 1;

        if !is_revision_meta_key(&key) {
            skipped_keys += 1;
            continue;
        }

        let needs_migration = check_needs_v3_migration(&value);

        if !needs_migration {
            skipped_keys += 1;
            continue;
        }

        // Try old format first, then new format for re-serialization
        let new_value = if let Ok(old_revision) = rmp_serde::from_slice::<OldRevisionMeta>(&value) {
            if migrated_keys < 5 {
                info!(
                    "  Migrating revision {} on branch {}",
                    old_revision.revision, old_revision.branch
                );
            }
            let new_revision = convert_old_to_new(&old_revision);
            rmp_serde::to_vec(&new_revision)
                .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?
        } else if let Ok(new_rev) = rmp_serde::from_slice::<raisin_storage::RevisionMeta>(&value) {
            // Already in new format, but re-serialize to ensure consistency
            rmp_serde::to_vec(&new_rev)
                .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?
        } else {
            warn!("Failed to deserialize revision metadata (skipping)");
            skipped_keys += 1;
            continue;
        };

        batch.put_cf(cf, &key, &new_value);
        migrated_keys += 1;
        batch_count += 1;

        if batch_count >= BATCH_SIZE {
            db.write(batch)
                .map_err(|e| Error::storage(format!("Batch write error: {}", e)))?;
            info!(
                "  Progress: {} migrated, {} total",
                migrated_keys, total_keys
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
        "Completed REVISIONS v3: {} total keys, {} migrated to consistent format, {} skipped",
        total_keys, migrated_keys, skipped_keys
    );

    Ok(())
}

/// Check if a value needs v3 migration (any 7, 8, or 9 element arrays)
fn check_needs_v3_migration(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }
    match value[0] {
        // 7, 8, or 9 elements all need re-serialization for consistency
        0x87..=0x89 => true,
        0xdc => {
            // array16
            if value.len() >= 3 {
                let count = u16::from_be_bytes([value[1], value[2]]);
                (7..=9).contains(&count)
            } else {
                false
            }
        }
        0xdd => {
            // array32
            if value.len() >= 5 {
                let count = u32::from_be_bytes([value[1], value[2], value[3], value[4]]);
                (7..=9).contains(&count)
            } else {
                false
            }
        }
        _ => false,
    }
}
