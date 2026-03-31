//! Job cleanup task for retention policy
//!
//! Runs hourly to delete old completed/failed/cancelled jobs from persistent
//! storage. Prevents unbounded growth of job history.

use chrono::{Duration, Utc};
use raisin_error::Result;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::jobs::JobMetadataStore;

/// Background task that periodically cleans up old jobs
///
/// Runs every hour and deletes jobs older than the configured retention period.
/// Only terminal jobs (Completed, Failed, Cancelled) are cleaned up.
pub struct JobCleanupTask {
    metadata_store: Arc<JobMetadataStore>,
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
            retention_hours,
            shutdown,
        }
    }

    /// Run the cleanup loop (runs every hour)
    ///
    /// Continuously performs cleanup until the shutdown signal is received.
    pub async fn run(self) {
        tracing::info!(
            retention_hours = self.retention_hours,
            "Job cleanup task started"
        );

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    tracing::info!("Job cleanup task received shutdown signal");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {
                    if let Err(e) = self.cleanup_old_jobs().await {
                        tracing::error!(error = %e, "Job cleanup failed");
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

        let deleted_count = self.metadata_store.cleanup_old_jobs(cutoff)?;

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
            tenant: Some("test".to_string()),
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
        let retrieved = store.get(&job_id).unwrap();
        assert!(retrieved.is_none());
    }
}
