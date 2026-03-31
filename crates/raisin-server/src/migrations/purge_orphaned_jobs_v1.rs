//! Purge Orphaned Jobs Migration V1: Remove undeserializable job entries
//!
//! This migration fixes a stall caused by orphaned job entries in JOB_METADATA
//! that cannot be deserialized (due to changed JobType serialization formats).
//! These orphaned entries:
//! - Cause `list_all()` and `list_by_status()` to log WARNs for each one
//! - Cannot be cleaned up by `cleanup_old_jobs()` or `delete_batch()`
//! - Accumulate over time as dead weight in the column family
//!
//! This migration runs on every startup (idempotent) and simply removes any
//! entries that fail deserialization from both JOB_METADATA and JOB_DATA.

use raisin_error::Result;
use raisin_rocksdb::PersistedJobEntry;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// Statistics from the orphaned jobs purge
#[derive(Debug, Clone, Default)]
pub struct PurgeOrphanedJobsStats {
    pub entries_scanned: usize,
    pub orphans_purged: usize,
    pub errors: usize,
}

/// Run the orphaned jobs purge (idempotent, runs every startup)
pub async fn migrate(db: Arc<DB>) -> Result<PurgeOrphanedJobsStats> {
    tracing::info!("Scanning JOB_METADATA for orphaned (undeserializable) entries...");

    let cf_metadata = db
        .cf_handle("job_metadata")
        .ok_or_else(|| raisin_error::Error::storage("job_metadata column family not found"))?;

    let cf_data = db
        .cf_handle("job_data")
        .ok_or_else(|| raisin_error::Error::storage("job_data column family not found"))?;

    let mut stats = PurgeOrphanedJobsStats::default();
    let mut keys_to_purge: Vec<Vec<u8>> = Vec::new();

    // Scan all entries in JOB_METADATA
    let iter = db.iterator_cf(cf_metadata, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key_bytes, value_bytes) = match item {
            Ok(kv) => kv,
            Err(e) => {
                tracing::warn!(error = %e, "Error reading JOB_METADATA entry during purge scan");
                stats.errors += 1;
                continue;
            }
        };

        stats.entries_scanned += 1;

        // Try to deserialize - if it fails, this is an orphaned entry
        if rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes).is_err() {
            keys_to_purge.push(key_bytes.to_vec());
        }
    }

    // Delete all orphaned entries in a single WriteBatch
    if !keys_to_purge.is_empty() {
        let mut batch = WriteBatch::default();

        for key in &keys_to_purge {
            batch.delete_cf(cf_metadata, key);
            batch.delete_cf(cf_data, key);
        }

        db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to purge orphaned jobs: {}", e))
        })?;

        stats.orphans_purged = keys_to_purge.len();
    }

    Ok(stats)
}
