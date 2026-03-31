//! RocksDB checkpoint ingestion for replication catch-up.
//!
//! Implements CheckpointIngestor from raisin-replication, copying all data
//! from a checkpoint database into the running RocksDB instance.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_storage::Storage;

use crate::RocksDBStorage;

/// Implements CheckpointIngestor trait for RocksDB
pub struct RocksDbCheckpointIngestor {
    db: Arc<RocksDBStorage>,
}

impl RocksDbCheckpointIngestor {
    /// Create a new RocksDB checkpoint ingestor
    pub fn new(db: Arc<RocksDBStorage>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl raisin_replication::CheckpointIngestor for RocksDbCheckpointIngestor {
    async fn ingest_checkpoint(
        &self,
        checkpoint_dir: &std::path::Path,
        snapshot_id: &str,
    ) -> Result<usize, raisin_replication::CoordinatorError> {
        tracing::info!(
            snapshot_id = %snapshot_id,
            checkpoint_dir = %checkpoint_dir.display(),
            "Ingesting checkpoint into RocksDB via copy-based approach"
        );

        // Step 1: Open the checkpoint as a temporary read-only database with ALL column families
        // CRITICAL: Must open with column families, otherwise only default CF is accessible!
        let checkpoint_db = tokio::task::spawn_blocking({
            let checkpoint_path = checkpoint_dir.to_path_buf();
            move || {
                use crate::all_column_families;

                // List existing column families in checkpoint
                let cf_names = match rocksdb::DB::list_cf(&rocksdb::Options::default(), &checkpoint_path) {
                    Ok(cfs) => cfs,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to list column families in checkpoint (may be old format): {}. Using default set.",
                            e
                        );
                        // Use our standard set if listing fails
                        all_column_families().iter().map(|s| s.to_string()).collect()
                    }
                };

                tracing::info!(
                    cf_count = cf_names.len(),
                    "Opening checkpoint with {} column families",
                    cf_names.len()
                );

                // Open checkpoint database with all column families
                let opts = rocksdb::Options::default();
                rocksdb::DB::open_cf_for_read_only(&opts, checkpoint_path, cf_names, false)
                    .map_err(|e| raisin_replication::CoordinatorError::Storage(
                        format!("Failed to open checkpoint database with column families: {}", e)
                    ))
            }
        })
        .await
        .map_err(|e| raisin_replication::CoordinatorError::Storage(
            format!("Failed to spawn checkpoint open task: {}", e)
        ))??;

        // Step 2: Copy all data from ALL column families in checkpoint to target database
        // CRITICAL: Must iterate through each column family separately!
        let target_db = self.db.db().clone();
        let cf_names_for_iteration = tokio::task::spawn_blocking({
            let checkpoint_path = checkpoint_dir.to_path_buf();
            move || {
                // List column families in checkpoint
                rocksdb::DB::list_cf(&rocksdb::Options::default(), &checkpoint_path).unwrap_or_else(
                    |_| {
                        // If listing fails, use our standard set
                        crate::all_column_families()
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    },
                )
            }
        })
        .await
        .map_err(|e| {
            raisin_replication::CoordinatorError::Storage(format!(
                "Failed to list checkpoint CFs: {}",
                e
            ))
        })?;

        let num_keys = tokio::task::spawn_blocking(move || {
            use rocksdb::{IteratorMode, WriteBatch};

            let mut total_count = 0usize;
            const BATCH_SIZE: usize = 1000;

            tracing::info!(
                cf_count = cf_names_for_iteration.len(),
                "Copying data from {} column families",
                cf_names_for_iteration.len()
            );

            // Iterate through each column family
            for cf_name in &cf_names_for_iteration {
                let cf_handle = checkpoint_db.cf_handle(cf_name).ok_or_else(|| {
                    raisin_replication::CoordinatorError::Storage(format!(
                        "Column family '{}' not found in checkpoint",
                        cf_name
                    ))
                })?;

                let target_cf = target_db.cf_handle(cf_name).ok_or_else(|| {
                    raisin_replication::CoordinatorError::Storage(format!(
                        "Column family '{}' not found in target database",
                        cf_name
                    ))
                })?;

                let mut batch = WriteBatch::default();
                let mut cf_count = 0usize;

                // Iterate through all keys in this column family
                let iter = checkpoint_db.iterator_cf(&cf_handle, IteratorMode::Start);
                for item in iter {
                    let (key, value) = item.map_err(|e| {
                        raisin_replication::CoordinatorError::Storage(format!(
                            "Failed to read from checkpoint CF '{}': {}",
                            cf_name, e
                        ))
                    })?;

                    batch.put_cf(&target_cf, &key, &value);
                    cf_count += 1;
                    total_count += 1;

                    // Write in batches for efficiency
                    if cf_count % BATCH_SIZE == 0 {
                        target_db.write(batch).map_err(|e| {
                            raisin_replication::CoordinatorError::Storage(format!(
                                "Failed to write batch for CF '{}' at key {}: {}",
                                cf_name, cf_count, e
                            ))
                        })?;
                        batch = WriteBatch::default();
                    }

                    if total_count % 10000 == 0 {
                        tracing::debug!("Copied {} keys so far...", total_count);
                    }
                }

                // Write remaining keys for this CF
                if !batch.is_empty() {
                    target_db.write(batch).map_err(|e| {
                        raisin_replication::CoordinatorError::Storage(format!(
                            "Failed to write final batch for CF '{}': {}",
                            cf_name, e
                        ))
                    })?;
                }

                tracing::info!(
                    cf_name = cf_name,
                    cf_keys = cf_count,
                    "Copied {} keys from CF '{}'",
                    cf_count,
                    cf_name
                );
            }

            Ok::<usize, raisin_replication::CoordinatorError>(total_count)
        })
        .await
        .map_err(|e| {
            raisin_replication::CoordinatorError::Storage(format!(
                "Checkpoint copy task failed: {}",
                e
            ))
        })??;

        tracing::info!(
            snapshot_id = %snapshot_id,
            num_keys = num_keys,
            "Checkpoint ingestion complete via copy-based approach"
        );

        // NOTE: We do NOT emit RepositoryCreated events after checkpoint restoration.
        // The checkpoint contains a complete copy of all data including:
        // - NodeTypes (in NODE_TYPES CF)
        // - Workspace structures (nodes in NODES CF)
        // - All metadata and indexes
        //
        // Emitting events would trigger handlers that try to re-initialize this data,
        // which can cause:
        // 1. Deserialization errors if data formats don't match YAML definitions
        // 2. Duplicate workspace structure creation
        // 3. Unnecessary overhead
        //
        // Checkpoint restoration is a pure data copy operation - no initialization needed.

        Ok(num_keys)
    }
}
