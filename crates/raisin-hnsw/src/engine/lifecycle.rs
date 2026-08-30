// SPDX-License-Identifier: BSL-1.1

//! Lifecycle management for the HNSW indexing engine.
//!
//! Handles periodic snapshot tasks, dirty index persistence, and graceful shutdown.

use raisin_error::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use super::HnswIndexingEngine;

impl HnswIndexingEngine {
    /// Start periodic snapshot task.
    ///
    /// Returns a JoinHandle that can be used to abort the task.
    ///
    /// The task runs every 60 seconds and saves all dirty indexes.
    pub fn start_snapshot_task(self: &Arc<Self>) -> JoinHandle<()> {
        let engine = Arc::clone(self);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                if let Err(e) = engine.snapshot_dirty_indexes() {
                    tracing::error!("Failed to snapshot HNSW indexes: {}", e);
                }
            }
        })
    }

    /// Save all dirty indexes to disk.
    ///
    /// This is called periodically by the snapshot task and during shutdown.
    pub fn snapshot_dirty_indexes(&self) -> Result<()> {
        let dirty = self.dirty_indexes.read().unwrap().clone();

        if dirty.is_empty() {
            return Ok(());
        }

        tracing::debug!("Snapshotting {} dirty HNSW indexes...", dirty.len());

        let mut saved_count = 0;
        let mut error_count = 0;

        for key in dirty {
            if let Some(index_arc) = self.index_cache.get(&key) {
                let path = key.index_path(self.base_path());

                // Save to disk
                let index_guard = index_arc.read().unwrap();
                match index_guard.save_to_file(&path) {
                    Ok(()) => {
                        saved_count += 1;
                        // Mark as clean
                        self.dirty_indexes.write().unwrap().remove(&key);
                        tracing::debug!("Saved HNSW index: {}", key);
                    }
                    Err(e) => {
                        error_count += 1;
                        tracing::error!("Failed to save HNSW index {}: {}", key, e);
                    }
                }
            } else {
                // The index is no longer resident. It used to be dropped from
                // the dirty set right here, WITHOUT being saved — silent loss of
                // every vector added since the last snapshot, and more likely
                // the more cache entries there are (which partitioning
                // increases). The eviction listener now saves a dirty index as
                // it leaves the cache, so reaching this branch means it was
                // already written out; drop the bookkeeping only.
                self.dirty_indexes.write().unwrap().remove(&key);
                self.mutations_since_weigh.write().unwrap().remove(&key);
            }
        }

        if saved_count > 0 {
            tracing::info!(
                "Snapshotted {} HNSW indexes ({} errors)",
                saved_count,
                error_count
            );
        }

        Ok(())
    }

    /// Graceful shutdown: save all dirty indexes.
    ///
    /// Should be called before process termination.
    pub async fn shutdown(&self) -> Result<()> {
        tracing::info!("Saving all dirty HNSW indexes before shutdown...");
        self.snapshot_dirty_indexes()?;
        tracing::info!("HNSW indexes saved successfully");
        Ok(())
    }
}
