//! Background jobs orchestration for RocksDB storage
//!
//! This module provides a comprehensive background job system that runs:
//! - Periodic integrity checks
//! - Automatic index rebuilding when corruption detected
//! - Revision compaction based on retention policies
//! - Scheduled backups
//! - Self-healing operations
//!
//! All operations are scoped to repository level as the primary unit of management.

mod tasks;
#[cfg(test)]
mod tests;

use super::*;
use crate::graph::GraphCacheLayer;
use crate::RocksDBStorage;
use raisin_error::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

// Re-export for use in this module
pub use super::async_indexing;
pub use super::compaction;
pub use super::integrity;

/// Background job scheduler configuration
#[derive(Clone, Debug)]
pub struct BackgroundJobsConfig {
    /// Enable periodic integrity checks
    pub integrity_check_enabled: bool,
    /// How often to run integrity checks (default: 1 hour)
    pub integrity_check_interval: Duration,

    /// Enable automatic compaction
    pub compaction_enabled: bool,
    /// How often to run compaction (default: 6 hours)
    pub compaction_interval: Duration,
    /// Retention policy for compaction
    pub compaction_retention: compaction::RevisionRetentionPolicy,

    /// Enable scheduled backups
    pub backup_enabled: bool,
    /// How often to run backups (default: 24 hours)
    pub backup_interval: Duration,
    /// Backup destination directory
    pub backup_destination: Option<std::path::PathBuf>,

    /// Enable self-healing (auto-fix detected issues)
    pub self_heal_enabled: bool,

    /// Health threshold below which to trigger self-healing (0.0 - 1.0)
    pub self_heal_threshold: f64,

    /// Maximum number of concurrent background jobs
    pub max_concurrent_jobs: usize,

    /// Enable graph algorithm precomputation
    pub graph_compute_enabled: bool,
    /// How often to check for stale graph caches (default: 60 seconds)
    pub graph_compute_interval: Duration,
    /// Maximum graph configs to process per tick
    pub graph_compute_max_configs_per_tick: usize,
}

impl Default for BackgroundJobsConfig {
    fn default() -> Self {
        Self {
            integrity_check_enabled: true,
            integrity_check_interval: Duration::from_secs(3600), // 1 hour

            compaction_enabled: true,
            compaction_interval: Duration::from_secs(21600), // 6 hours
            compaction_retention: compaction::RevisionRetentionPolicy::KeepLatest(10),

            backup_enabled: false, // Disabled by default (requires destination)
            backup_interval: Duration::from_secs(86400), // 24 hours
            backup_destination: None,

            self_heal_enabled: true,
            self_heal_threshold: 0.75, // Trigger healing if health < 75%

            max_concurrent_jobs: 2,

            graph_compute_enabled: true,
            graph_compute_interval: Duration::from_secs(60), // 1 minute
            graph_compute_max_configs_per_tick: 10,
        }
    }
}

/// Statistics for background job execution
#[derive(Clone, Debug, Default)]
pub struct BackgroundJobStats {
    pub integrity_checks_run: u64,
    pub integrity_checks_failed: u64,
    pub compactions_run: u64,
    pub compactions_failed: u64,
    pub backups_run: u64,
    pub backups_failed: u64,
    pub self_heals_triggered: u64,
    pub self_heals_successful: u64,
    pub last_integrity_check: Option<std::time::SystemTime>,
    pub last_compaction: Option<std::time::SystemTime>,
    pub last_backup: Option<std::time::SystemTime>,
    // Graph compute stats
    pub graph_compute_ticks: u64,
    pub graph_compute_configs_processed: u64,
    pub graph_compute_nodes_computed: u64,
    pub graph_compute_errors: u64,
    pub last_graph_compute: Option<std::time::SystemTime>,
}

/// Background jobs orchestrator
pub struct BackgroundJobs {
    pub(super) storage: Arc<RocksDBStorage>,
    pub(super) graph_cache_layer: Arc<GraphCacheLayer>,
    pub(super) config: BackgroundJobsConfig,
    pub(super) running: Arc<AtomicBool>,
    pub(super) stats: Arc<Mutex<BackgroundJobStats>>,
    pub(super) handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl BackgroundJobs {
    /// Create a new background jobs orchestrator
    pub fn new(
        storage: Arc<RocksDBStorage>,
        graph_cache_layer: Arc<GraphCacheLayer>,
        config: BackgroundJobsConfig,
    ) -> Self {
        Self {
            storage,
            graph_cache_layer,
            config,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(BackgroundJobStats::default())),
            handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start all background jobs
    pub async fn start(&self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(raisin_error::Error::storage(
                "Background jobs already running",
            ));
        }

        let mut handles = self.handles.lock().await;
        handles.clear();

        if self.config.integrity_check_enabled {
            let handle = self.spawn_integrity_check_job();
            handles.push(handle);
        }

        if self.config.compaction_enabled {
            let handle = self.spawn_compaction_job();
            handles.push(handle);
        }

        if self.config.backup_enabled && self.config.backup_destination.is_some() {
            let handle = self.spawn_backup_job();
            handles.push(handle);
        }

        if self.config.graph_compute_enabled {
            let handle = self.spawn_graph_compute_job();
            handles.push(handle);
        }

        Ok(())
    }

    /// Stop all background jobs
    pub async fn stop(&self) -> Result<()> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }

        let mut handles = self.handles.lock().await;

        for handle in handles.drain(..) {
            handle.abort();
        }

        Ok(())
    }

    /// Check if background jobs are running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get current statistics
    pub async fn stats(&self) -> BackgroundJobStats {
        self.stats.lock().await.clone()
    }
}

impl Drop for BackgroundJobs {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
