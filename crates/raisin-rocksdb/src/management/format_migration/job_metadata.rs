//! Job metadata and type serialization migrations
//!
//! - `run_job_metadata_migration`: Adds next_retry_at field to PersistedJobEntry
//! - `run_job_type_serialization_migration`: Converts JobType from JSON objects to strings

use raisin_error::{Error, Result};
use rocksdb::DB;
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

use super::old_types::{convert_old_job_type, OldPersistedJobEntry, OldPersistedJobEntryV3};
use super::BATCH_SIZE;
use crate::cf;

const JOB_METADATA_MIGRATION_MARKER: &str = ".migrated_job_metadata_v2";
const JOB_TYPE_SERIALIZATION_MARKER: &str = ".migrated_job_type_v3";

/// Run job metadata migration: Old 13-field format -> New 14-field format (added next_retry_at)
///
/// This migration updates PersistedJobEntry to include the next_retry_at field
/// for exponential backoff retry delays.
///
/// # Arguments
/// * `db` - RocksDB database instance
/// * `data_dir` - Data directory path (for marker file)
pub async fn run_job_metadata_migration(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(JOB_METADATA_MIGRATION_MARKER);

    if marker_path.exists() {
        info!("Job metadata migration already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting job metadata migration: Adding next_retry_at field");

    migrate_job_metadata_cf(&db)?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Job metadata migration completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Migration: Convert JobType from old JSON object format to new string format
///
/// Background:
/// - Old format: Serde default enum serialization as JSON objects
///   Example: `{"EmbeddingGenerate": {"node_id": "xyz"}}`
/// - New format: Custom string serialization via Display
///   Example: `"EmbeddingGenerate(xyz)"`
pub async fn run_job_type_serialization_migration(db: Arc<DB>, data_dir: &Path) -> Result<()> {
    let marker_path = data_dir.join(JOB_TYPE_SERIALIZATION_MARKER);

    if marker_path.exists() {
        info!("Job type serialization migration already completed (marker file exists), skipping");
        return Ok(());
    }

    info!("Starting job type serialization migration: JSON objects -> strings");

    migrate_job_type_cf(&db)?;

    std::fs::write(&marker_path, "migrated")
        .map_err(|e| Error::storage(format!("Failed to create migration marker: {}", e)))?;

    info!("Job type serialization migration completed successfully!");
    info!("  Marker file created at: {}", marker_path.display());

    Ok(())
}

/// Migrate JOB_METADATA column family to add next_retry_at field
fn migrate_job_metadata_cf(db: &DB) -> Result<()> {
    use crate::jobs::PersistedJobEntry;

    info!("Migrating JOB_METADATA column family");

    let cf = db
        .cf_handle(cf::JOB_METADATA)
        .ok_or_else(|| Error::storage("JOB_METADATA column family not found"))?;

    let mut total_keys = 0;
    let mut migrated_keys = 0;
    let mut skipped_keys = 0;
    let mut batch = rocksdb::WriteBatch::default();
    let mut batch_count = 0;

    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;
        total_keys += 1;

        let old_entry: OldPersistedJobEntry = match rmp_serde::from_slice(&value) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "Failed to deserialize old job metadata at key (skipping): {}",
                    e
                );
                skipped_keys += 1;
                continue;
            }
        };

        let new_entry = PersistedJobEntry {
            id: old_entry.id,
            job_type: old_entry.job_type,
            status: old_entry.status,
            tenant: old_entry.tenant,
            started_at: old_entry.started_at,
            completed_at: old_entry.completed_at,
            error: old_entry.error,
            progress: old_entry.progress,
            result: old_entry.result,
            retry_count: old_entry.retry_count,
            max_retries: old_entry.max_retries,
            last_heartbeat: old_entry.last_heartbeat,
            timeout_seconds: old_entry.timeout_seconds,
            next_retry_at: None,
        };

        let new_value = rmp_serde::to_vec(&new_entry)
            .map_err(|e| Error::storage(format!("MessagePack serialization error: {}", e)))?;

        batch.put_cf(cf, &key, &new_value);
        migrated_keys += 1;
        batch_count += 1;

        if batch_count >= BATCH_SIZE {
            db.write(batch)
                .map_err(|e| Error::storage(format!("Batch write error: {}", e)))?;
            info!(
                "  Progress: {} migrated, {} total in JOB_METADATA",
                migrated_keys, total_keys
            );
            batch = rocksdb::WriteBatch::default();
            batch_count = 0;
        }
    }

    if batch_count > 0 {
        db.write(batch)
            .map_err(|e| Error::storage(format!("Final batch write error: {}", e)))?;
    }

    info!(
        "Completed JOB_METADATA: {} total keys, {} migrated, {} skipped",
        total_keys, migrated_keys, skipped_keys
    );

    Ok(())
}

/// Migrate JOB_METADATA column family to new JobType string serialization format
fn migrate_job_type_cf(db: &Arc<DB>) -> Result<()> {
    use crate::jobs::metadata_store::PersistedJobEntry;

    let cf_handle = db
        .cf_handle(cf::JOB_METADATA)
        .ok_or_else(|| Error::storage(format!("Column family {} not found", cf::JOB_METADATA)))?;

    info!("  Migrating JOB_METADATA column family...");

    let mut batch_count = 0;
    let mut total_migrated = 0;
    let mut write_batch = rocksdb::WriteBatch::default();

    let iter = db.iterator_cf(cf_handle, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value_bytes) =
            item.map_err(|e| Error::storage(format!("Iterator error: {}", e)))?;

        let old_entry: OldPersistedJobEntryV3 =
            rmp_serde::from_slice(&value_bytes).map_err(|e| {
                Error::storage(format!("Failed to deserialize old job metadata: {}", e))
            })?;

        let new_entry = PersistedJobEntry {
            id: old_entry.id,
            job_type: convert_old_job_type(old_entry.job_type),
            status: old_entry.status,
            tenant: old_entry.tenant,
            started_at: old_entry.started_at,
            completed_at: old_entry.completed_at,
            error: old_entry.error,
            progress: old_entry.progress,
            result: old_entry.result,
            retry_count: old_entry.retry_count,
            max_retries: old_entry.max_retries,
            last_heartbeat: old_entry.last_heartbeat,
            timeout_seconds: old_entry.timeout_seconds,
            next_retry_at: old_entry.next_retry_at,
        };

        let new_value = rmp_serde::to_vec(&new_entry)
            .map_err(|e| Error::storage(format!("Failed to serialize new job metadata: {}", e)))?;

        write_batch.put_cf(cf_handle, &key, &new_value);
        batch_count += 1;
        total_migrated += 1;

        if batch_count >= BATCH_SIZE {
            db.write(write_batch)
                .map_err(|e| Error::storage(format!("Failed to write batch: {}", e)))?;
            info!("    Migrated {} jobs...", total_migrated);
            write_batch = rocksdb::WriteBatch::default();
            batch_count = 0;
        }
    }

    if batch_count > 0 {
        db.write(write_batch)
            .map_err(|e| Error::storage(format!("Failed to write final batch: {}", e)))?;
    }

    info!("  Migrated {} total jobs", total_migrated);

    Ok(())
}
