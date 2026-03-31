//! Background graph algorithm computation
//!
//! This module provides background computation for graph algorithms following
//! the same pattern as integrity checks and compaction in `management/background.rs`.
//!
//! Key design decisions:
//! - NOT using job queue (to avoid queue congestion from many small graph changes)
//! - Runs periodically like compaction/integrity tasks
//! - Natural debouncing: 100 changes -> 1 recomputation when tick runs
//! - Operates at branch level (like vector search)
//! - Checks staleness before recomputing

mod cache_io;
mod projection;
mod tick;

#[cfg(test)]
mod tests;

use super::cache_layer::GraphCacheLayer;
use crate::RocksDBStorage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Background graph computation task configuration
#[derive(Clone, Debug)]
pub struct GraphComputeConfig {
    /// Whether background computation is enabled
    pub enabled: bool,
    /// How often to check for stale caches and recompute
    pub check_interval: Duration,
    /// Maximum configs to process per tick (prevent long-running ticks)
    pub max_configs_per_tick: usize,
    /// Maximum nodes per algorithm execution (prevent memory issues)
    pub max_nodes_per_execution: usize,
}

impl Default for GraphComputeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval: Duration::from_secs(60), // Check every minute
            max_configs_per_tick: 10,
            max_nodes_per_execution: 100_000,
        }
    }
}

/// Statistics for graph computation
#[derive(Clone, Debug, Default)]
pub struct GraphComputeStats {
    pub ticks_run: u64,
    pub configs_processed: u64,
    pub nodes_computed: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub errors: u64,
    pub last_tick: Option<std::time::SystemTime>,
    pub last_computation: Option<std::time::SystemTime>,
}

/// Statistics for a single tick execution (returned from run_tick_static)
#[derive(Clone, Debug, Default)]
pub struct TickStats {
    pub configs_processed: u64,
    pub nodes_computed: u64,
    pub errors: u64,
}

/// Background graph computation task
///
/// Runs periodically to check for stale graph algorithm caches and recompute them.
/// Follows the same pattern as `BackgroundJobs` in `management/background.rs`.
pub struct GraphComputeTask {
    storage: Arc<RocksDBStorage>,
    cache_layer: Arc<GraphCacheLayer>,
    config: GraphComputeConfig,
    running: Arc<AtomicBool>,
    stats: Arc<Mutex<GraphComputeStats>>,
}

impl GraphComputeTask {
    /// Create a new background graph computation task
    pub fn new(
        storage: Arc<RocksDBStorage>,
        cache_layer: Arc<GraphCacheLayer>,
        config: GraphComputeConfig,
    ) -> Self {
        Self {
            storage,
            cache_layer,
            config,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(GraphComputeStats::default())),
        }
    }

    /// Start the background task
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            tracing::warn!("Graph compute task already running");
        }

        let storage = self.storage.clone();
        let cache_layer = self.cache_layer.clone();
        let config = self.config.clone();
        let running = self.running.clone();
        let stats = self.stats.clone();

        tokio::spawn(async move {
            tracing::info!(
                "Starting graph compute background task (interval: {:?})",
                config.check_interval
            );

            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(config.check_interval).await;

                if !running.load(Ordering::SeqCst) {
                    break;
                }

                // Run one tick
                match Self::run_tick(&storage, &cache_layer, &config, &stats).await {
                    Ok(_) => {
                        let mut s = stats.lock().await;
                        s.ticks_run += 1;
                        s.last_tick = Some(std::time::SystemTime::now());
                    }
                    Err(e) => {
                        tracing::error!("Graph compute tick failed: {}", e);
                        let mut s = stats.lock().await;
                        s.errors += 1;
                    }
                }
            }

            tracing::info!("Graph compute background task stopped");
        })
    }

    /// Stop the background task
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get current statistics
    pub async fn stats(&self) -> GraphComputeStats {
        self.stats.lock().await.clone()
    }
}
