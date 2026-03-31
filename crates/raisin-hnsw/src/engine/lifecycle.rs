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
                // Parse key: {tenant}/{repo}/{branch}/{workspace}
                let path = self.get_index_path(&key);

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
                // Index was evicted, remove from dirty set
                self.dirty_indexes.write().unwrap().remove(&key);
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
