//! Job cleanup task for retention policy
//!
//! Runs hourly to delete old completed/failed/cancelled jobs from persistent
//! storage. Prevents unbounded growth of job history.

use chrono::{Duration, Utc};
use raisin_error::Result;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::jobs::JobMetadataStore;

/// In-memory registry retention. Much shorter than the persisted retention:
/// the registry only needs to serve live status/SSE queries, while history
/// reads go to the metadata store. Without this bound the registry map holds
/// every job since boot (~50k/day on a busy scheduler) and its long-lived
/// entries pin allocator arenas, bloating RSS indefinitely.
const IN_MEMORY_RETENTION_HOURS: i64 = 1;

/// How often the in-memory registry is swept. Must be much shorter than the
/// persisted retention: a runaway trigger/function loop can register
/// thousands of full-fidelity `JobEntry` structs (each holding its complete
/// result JSON) per minute, so an hourly-only sweep lets up to an hour of
/// them pile up before any relief. Sixty seconds bounds that window.
const REGISTRY_SWEEP_INTERVAL_SECS: u64 = 60;

/// Registry sweeps per persisted-store cleanup pass (60 × 60s = hourly).
const SWEEPS_PER_PERSISTED_CLEANUP: u32 = 60;

/// Grace window before a terminal entry's `result` payload is dropped from
/// the in-memory registry. Long enough for every synchronous invoke/export
/// poll loop to read its result from memory; afterwards reads are served
/// from the persisted metadata store (which stored the full result at the
/// terminal transition).
const RESULT_GRACE_MINUTES: i64 = 10;

/// Hard ceiling on in-memory registry entries. Terminal entries beyond this
/// are evicted oldest-first even if younger than the in-memory retention;
/// their state remains readable from the persisted metadata store. Sized so
/// that even worst-case ~10KB entries stay under ~200MB.
const MAX_REGISTRY_ENTRIES: usize = 20_000;

/// Background task that periodically cleans up old jobs
///
/// Runs every hour and deletes jobs older than the configured retention period.
/// Only terminal jobs (Completed, Failed, Cancelled) are cleaned up. Also
/// prunes terminal entries from the in-memory `JobRegistry` (shorter
/// retention), which is never cleaned anywhere else.
pub struct JobCleanupTask {
    metadata_store: Arc<JobMetadataStore>,
    registry: Option<Arc<raisin_storage::jobs::JobRegistry>>,
    retention_hours: i64,
    shutdown: CancellationToken,
}

impl JobCleanupTask {
    /// Create a new cleanup task
    ///
    /// # Arguments
    ///
    /// * `metadata_store` - Job metadata store to clean
    /// * `retention_hours` - Keep jobs for this many hours (default 24)
    /// * `shutdown` - Cancellation token for graceful shutdown
    pub fn new(
        metadata_store: Arc<JobMetadataStore>,
        retention_hours: i64,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            metadata_store,
            registry: None,
            retention_hours,
            shutdown,
        }
    }

    /// Also prune terminal entries from the in-memory job registry each pass.
    pub fn with_registry(mut self, registry: Arc<raisin_storage::jobs::JobRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Run the cleanup loop.
    ///
    /// The in-memory registry is swept every `REGISTRY_SWEEP_INTERVAL_SECS`
    /// (age-based prune + capacity eviction), starting one interval after
    /// boot — not an hour in. The persisted metadata store keeps its hourly
    /// cadence. Continues until the shutdown signal is received.
    pub async fn run(self) {
        tracing::info!(
            retention_hours = self.retention_hours,
            sweep_interval_secs = REGISTRY_SWEEP_INTERVAL_SECS,
            max_registry_entries = MAX_REGISTRY_ENTRIES,
            "Job cleanup task started"
        );

        // Start one tick short so the first sweep (T+60s) also runs the
        // persisted cleanup — after a crash during a runaway, waiting a full
        // hour leaves a large terminal backlog on disk (and in admin-console
        // listings) for no reason.
        let mut sweeps_since_persisted_cleanup: u32 = SWEEPS_PER_PERSISTED_CLEANUP - 1;
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    tracing::info!("Job cleanup task received shutdown signal");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(REGISTRY_SWEEP_INTERVAL_SECS)) => {
                    if let Some(registry) = &self.registry {
                        registry
                            .cleanup_old_jobs(Duration::hours(IN_MEMORY_RETENTION_HOURS))
                            .await;
                        registry
                            .trim_terminal_results(Duration::minutes(RESULT_GRACE_MINUTES))
                            .await;
                        registry.enforce_registry_capacity(MAX_REGISTRY_ENTRIES).await;
                    }

                    sweeps_since_persisted_cleanup += 1;
                    if sweeps_since_persisted_cleanup >= SWEEPS_PER_PERSISTED_CLEANUP {
                        sweeps_since_persisted_cleanup = 0;
                        if let Err(e) = self.cleanup_old_jobs().await {
                            tracing::error!(error = %e, "Job cleanup failed");
                        }
                    }
                }
            }
        }

        tracing::info!("Job cleanup task stopped");
    }

    /// Delete jobs older than retention period
    ///
    /// Calculates cutoff timestamp and delegates to metadata store for deletion.
    async fn cleanup_old_jobs(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::hours(self.retention_hours);

        // RocksDB scan+delete is synchronous; keep it off the async runtime
        // so a large purge can't stall other tasks on this worker thread.
        let store = self.metadata_store.clone();
        let deleted_count = tokio::task::spawn_blocking(move || store.cleanup_old_jobs(cutoff))
            .await
            .map_err(|e| raisin_error::Error::Internal(format!("cleanup task join: {e}")))??;

        if deleted_count > 0 {
            tracing::info!(
                deleted_count = deleted_count,
                retention_hours = self.retention_hours,
                "Cleaned up old jobs"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobMetadataStore, PersistedJobEntry};
    use raisin_storage::jobs::{IndexOperation, JobContext, JobId, JobStatus, JobType};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_cleanup_removes_old_jobs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::open_db(temp_dir.path()).unwrap();
        let store = Arc::new(JobMetadataStore::new(Arc::new(db)));

        // Create an old completed job
        let job_id = JobId::new();
        let entry = PersistedJobEntry {
            id: job_id.0.clone(),
            job_type: JobType::FulltextIndex {
                node_id: "test".to_string(),
                operation: IndexOperation::AddOrUpdate,
            },
            status: JobStatus::Completed,
            tenant: "test".to_string(),
            started_at: Utc::now() - chrono::Duration::hours(48),
            completed_at: Some(Utc::now() - chrono::Duration::hours(48)),
            error: None,
            progress: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        };

        let context = JobContext {
            tenant_id: "test".to_string(),
            repo_id: "test".to_string(),
            branch: "main".to_string(),
            workspace_id: "test".to_string(),
            revision: raisin_hlc::HLC::new(1, 0),
            metadata: HashMap::new(),
        };

        store.put_with_context(&job_id, &entry, &context).unwrap();

        // Create cleanup task with 24 hour retention
        let shutdown = CancellationToken::new();
        let cleanup = JobCleanupTask::new(store.clone(), 24, shutdown);

        // Run cleanup
        cleanup.cleanup_old_jobs().await.unwrap();

        // Job should be deleted
        let retrieved = store.get("test", &job_id).unwrap();
        assert!(retrieved.is_none());
    }
}
