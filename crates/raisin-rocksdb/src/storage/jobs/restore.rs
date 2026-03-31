//! Job restoration after crash/restart
//!
//! Handles scanning persistent storage for pending/running jobs and
//! restoring them to the in-memory JobRegistry on startup.

use crate::storage::{RestoreStats, RocksDBStorage};
use raisin_error::Result;

impl RocksDBStorage {
    /// Restore pending jobs from persistent storage after crash/restart
    ///
    /// Scans JOB_METADATA CF for Scheduled/Running jobs, loads their contexts
    /// from JOB_DATA CF, and restores them to the in-memory JobRegistry.
    /// Running jobs are reset to Scheduled.
    ///
    /// # Returns
    ///
    /// Statistics about the restoration process
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail
    pub async fn restore_pending_jobs(&self) -> Result<RestoreStats> {
        use crate::jobs::PersistedJobEntry;

        tracing::info!("Restoring pending jobs from RocksDB");

        let mut restored = 0;
        let mut failed_to_restore = 0;
        let mut reset_running = 0;

        // List pending jobs from metadata store
        let pending = self.job_metadata_store.list_by_status(&[
            raisin_storage::jobs::JobStatus::Scheduled,
            raisin_storage::jobs::JobStatus::Running,
            raisin_storage::jobs::JobStatus::Executing,
        ])?;

        for (job_id, persisted_entry) in pending {
            // Load JobContext from job_data CF
            match self.job_data_store.get(&job_id)? {
                Some(_context) => {
                    // Convert PersistedJobEntry to JobInfo
                    let mut job_info = raisin_storage::jobs::JobInfo {
                        id: job_id.clone(),
                        job_type: persisted_entry.job_type.clone(),
                        status: persisted_entry.status.clone(),
                        tenant: persisted_entry.tenant.clone(),
                        started_at: persisted_entry.started_at,
                        completed_at: persisted_entry.completed_at,
                        progress: persisted_entry.progress,
                        error: persisted_entry.error.clone(),
                        result: persisted_entry.result.clone(),
                        retry_count: persisted_entry.retry_count,
                        max_retries: persisted_entry.max_retries,
                        last_heartbeat: persisted_entry.last_heartbeat,
                        timeout_seconds: persisted_entry.timeout_seconds,
                        next_retry_at: persisted_entry.next_retry_at,
                    };

                    // Reset Running/Executing → Scheduled (crashed mid-execution)
                    if matches!(job_info.status, raisin_storage::jobs::JobStatus::Running | raisin_storage::jobs::JobStatus::Executing) {
                        job_info.status = raisin_storage::jobs::JobStatus::Scheduled;
                        job_info.last_heartbeat = None;
                        reset_running += 1;

                        // Persist the reset status
                        let updated_entry = PersistedJobEntry {
                            id: persisted_entry.id,
                            job_type: persisted_entry.job_type,
                            status: raisin_storage::jobs::JobStatus::Scheduled,
                            tenant: persisted_entry.tenant,
                            started_at: persisted_entry.started_at,
                            completed_at: persisted_entry.completed_at,
                            error: persisted_entry.error,
                            progress: persisted_entry.progress,
                            result: persisted_entry.result,
                            retry_count: persisted_entry.retry_count,
                            max_retries: persisted_entry.max_retries,
                            last_heartbeat: None,
                            timeout_seconds: persisted_entry.timeout_seconds,
                            next_retry_at: persisted_entry.next_retry_at,
                        };
                        self.job_metadata_store.update(&job_id, &updated_entry)?;
                    }

                    // Restore to in-memory registry
                    self.job_registry.restore_job(job_info).await?;
                    restored += 1;
                }
                None => {
                    // JobContext missing - orphaned job metadata
                    tracing::warn!(job_id = %job_id, "Orphaned job metadata without context");
                    failed_to_restore += 1;

                    // Clean up orphaned metadata
                    self.job_metadata_store.delete(&job_id)?;
                }
            }
        }

        tracing::info!(
            restored = restored,
            reset_running = reset_running,
            failed = failed_to_restore,
            "Job restoration complete"
        );

        Ok(RestoreStats {
            restored,
            reset_running,
            failed_to_restore,
        })
    }
}
