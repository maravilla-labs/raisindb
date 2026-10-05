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

        // Step 1b: Skip-unchanged writes stop BEFORE the copy starts. Local
        // writes keep running while the column families are copied in many
        // independent batches, and must not take a peer version as a proven
        // predecessor (its writer may have left holes this node's rebuild
        // never saw). A failure aborts the ingest before anything is copied.
        crate::management::async_indexing::repair::invalidate_rebuilds_for_ingest(&self.db)
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Failed to invalidate property index rebuilds before checkpoint copy: {}",
                    e
                ))
            })?;
        // The localized name index's state records too (plan Phase 12): the
        // copy brings peer claims this node's records never vouched for.
        crate::localized_name::state::mark_all_not_built(self.db.db()).map_err(|e| {
            raisin_replication::CoordinatorError::Storage(format!(
                "Failed to mark localized name state NotBuilt before checkpoint copy: {}",
                e
            ))
        })?;

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

                // A CF this binary does not know (a peer on a newer release
                // added it) is skipped with a warning, never fatal: the data in
                // it is derived or new, and refusing the whole checkpoint would
                // leave this node unable to bootstrap at all (plan Phase 12.0).
                let Some(target_cf) = target_db.cf_handle(cf_name) else {
                    tracing::warn!(
                        cf = %cf_name,
                        "checkpoint carries a column family this binary does not know; skipping it"
                    );
                    continue;
                };

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

        // FIRST, before anything can read them: the copy brought the PEER's
        // compound state records, and a peer's `Ready` describes the peer's
        // apply history, not this node's — the compound keyspace is local and
        // was never checked against these records. Fail them closed; a local
        // rebuild re-earns `Ready`. (A local build in flight registered a
        // ticket the peer's record does not carry and the mark clears, so it
        // cannot stamp `Ready` over the ingest either — `compound_state::
        // build_cas`.) Propagated, not logged: a failure here would leave the
        // PEER's `Ready` in force on this node.
        let marked = crate::compound_state::CompoundStateStore::new(self.db.db().clone())
            .mark_all_stale()
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Failed to mark compound index state stale after checkpoint copy: {}",
                    e
                ))
            })?;
        tracing::info!(
            marked,
            "checkpoint ingest: compound index state marked NotBuilt pending local rebuild"
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
        //
        // But "no events" has a consequence beyond the init handlers: every
        // in-memory cache derived from stored data keeps itself correct by
        // listening for those same events, so all of them are now silently stale
        // — they describe the database as it was BEFORE this bulk copy. The SQL
        // workspace catalog is the sharp edge: it would answer "unknown table"
        // for every workspace this checkpoint just brought in.
        //
        // This is the blunt, correct response for a bulk path. It does not
        // re-initialize anything, so it does not reintroduce the problems the
        // note above describes.
        // Scoped to THIS database: a cache keyed by database (the compound
        // definitions) drops only its entries here, every other cache drops all.
        raisin_core::invalidate_derived_caches_for_database(&self.db.db().path().to_string_lossy());

        // Again AFTER the copy: INDEX_STATUS came with it, and a peer that
        // once ingested this node's state record may have brought back a
        // `done`. Propagated, not logged: a stale `done` here is a hole.
        crate::management::async_indexing::repair::invalidate_rebuilds_for_ingest(&self.db)
            .map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Failed to invalidate property index rebuilds after checkpoint copy: {}",
                    e
                ))
            })?;

        // The copy may have re-imported corruption a repair already cleaned
        // here (from an unrepaired peer). The repairs are data-detected and
        // idempotent, so re-running them over clean data writes nothing.
        match crate::management::async_indexing::repair::reenqueue_repairs_after_checkpoint(
            &self.db,
        )
        .await
        {
            Ok(repos) => tracing::info!(repos, "checkpoint ingest: index repairs re-enqueued"),
            Err(e) => {
                tracing::warn!(error = %e, "checkpoint ingest: could not re-enqueue index repairs")
            }
        }

        // This node's own compound rebuild (the records were failed closed
        // right after the copy, above): every branch is owed a
        // `compound_builds` link again, one branch at a time (plan Phase 13f;
        // AFTER the mark, so no link can judge the peer's `Ready` records).
        if let Err(e) =
            crate::management::async_indexing::repair::restart_compound_builds_after_ingest(
                &self.db,
            )
            .await
        {
            tracing::warn!(error = %e, "checkpoint ingest: could not queue compound builds");
        }

        // The peer's NODES may carry versions without `created_at` /
        // `updated_at`: the timestamp backfill is owed again (plan Phase 13g).
        if let Err(e) =
            crate::management::async_indexing::repair::restart_timestamp_backfill_after_ingest(
                &self.db,
            )
            .await
        {
            tracing::warn!(error = %e, "checkpoint ingest: could not queue the timestamp backfill");
        }

        // Same for the localized name index (plan Phase 12): the peer's state
        // records and claims arrived; fail the records closed (the lookup
        // falls back) and queue this node's own builds. Propagated, not
        // logged: INDEX_STATUS was just put-merged from the peer, so a failure
        // here would leave the PEER's `Ready` in force on this node.
        let marked =
            crate::localized_name::state::mark_all_not_built(self.db.db()).map_err(|e| {
                raisin_replication::CoordinatorError::Storage(format!(
                    "Failed to mark localized name state NotBuilt after checkpoint copy: {}",
                    e
                ))
            })?;
        tracing::info!(marked, "checkpoint ingest: localized name state NotBuilt");
        if crate::localized_name::enabled() {
            if let Err(e) = crate::management::async_indexing::repair::start_chain(
                &self.db,
                crate::management::async_indexing::repair::RepairKind::LocalizedNames,
            )
            .await
            {
                tracing::warn!(error = %e, "checkpoint ingest: could not queue localized name builds");
            }
        }

        Ok(num_keys)
    }
}
