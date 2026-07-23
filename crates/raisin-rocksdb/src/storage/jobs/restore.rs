//! Job restoration after crash/restart
//!
//! Handles scanning persistent storage for pending/running jobs and
//! restoring them to the in-memory JobRegistry on startup.

use crate::storage::{RestoreStats, RocksDBStorage};
use raisin_error::Result;

/// Non-terminal jobs older than this are abandoned (marked Failed) at
/// restart instead of restored, unless they are explicitly scheduled for a
/// future time (`next_retry_at` in the future). Without this, a runaway
/// registration burst leaves an ever-growing backlog of stale Scheduled
/// entries that every restart re-loads into memory and re-dispatches at
/// once — the observed "RSS explodes within seconds of startup" failure.
const MAX_RESTORE_AGE_HOURS: i64 = 24;

/// Hard per-tenant ceiling on jobs restored at startup when no
/// `max_active_jobs_per_tenant` is configured. Jobs beyond the cap
/// (oldest-last scan order) are abandoned rather than restored.
const DEFAULT_RESTORE_CAP_PER_TENANT: usize = 5_000;

impl RocksDBStorage {
    /// Restore pending jobs from persistent storage after crash/restart
    ///
    /// Streams JOB_METADATA for Scheduled/Running/Executing jobs, loads
    /// their contexts from JOB_DATA, and restores them to the in-memory
    /// JobRegistry. Running jobs are reset to Scheduled.
    ///
    /// Safety valves (all abandoned jobs are marked Failed in persistent
    /// storage, never silently dropped):
    /// - the scan is streaming — the backlog is never materialized in one Vec
    /// - non-terminal jobs older than [`MAX_RESTORE_AGE_HOURS`] with no
    ///   future schedule are abandoned instead of re-dispatched
    /// - per-tenant restore cap (the configured tenant job cap, else
    ///   [`DEFAULT_RESTORE_CAP_PER_TENANT`])
    /// - stale `result` payloads from prior attempts are not carried into
    ///   the registry (the persisted copy is untouched)
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
        use raisin_storage::jobs::JobStatus;
        use std::collections::HashMap;

        tracing::info!("Restoring pending jobs from RocksDB");

        let restore_cap = self
            .config()
            .max_active_jobs_per_tenant
            .unwrap_or(DEFAULT_RESTORE_CAP_PER_TENANT);
        let stale_cutoff = chrono::Utc::now() - chrono::Duration::hours(MAX_RESTORE_AGE_HOURS);

        let mut restored = 0usize;
        let mut failed_to_restore = 0usize;
        let mut reset_running = 0usize;
        let mut abandoned_stale = 0usize;
        let mut abandoned_over_cap = 0usize;
        let mut per_tenant: HashMap<String, usize> = HashMap::new();

        // Collect only (JobId, PersistedJobEntry) pairs that pass the valves,
        // one at a time; heavy processing happens per entry inside the visit.
        let mut pending: Vec<(raisin_storage::jobs::JobId, PersistedJobEntry)> = Vec::new();
        self.job_metadata_store.for_each_by_status(
            &[
                JobStatus::Scheduled,
                JobStatus::Running,
                JobStatus::Executing,
            ],
            |job_id, mut entry| {
                let future_scheduled = entry
                    .next_retry_at
                    .map(|t| t > chrono::Utc::now())
                    .unwrap_or(false);

                if entry.started_at < stale_cutoff && !future_scheduled {
                    let msg = format!(
                        "Abandoned at restart: pending for more than {} hours",
                        MAX_RESTORE_AGE_HOURS
                    );
                    entry.status = JobStatus::Failed(msg.clone());
                    entry.error = Some(msg);
                    entry.completed_at = Some(chrono::Utc::now());
                    self.job_metadata_store.update(&job_id, &entry)?;
                    abandoned_stale += 1;
                    return Ok(());
                }

                let count = per_tenant.entry(entry.tenant.clone()).or_insert(0);
                if *count >= restore_cap {
                    let msg = format!(
                        "Abandoned at restart: tenant exceeded the restore cap of {} pending jobs",
                        restore_cap
                    );
                    entry.status = JobStatus::Failed(msg.clone());
                    entry.error = Some(msg);
                    entry.completed_at = Some(chrono::Utc::now());
                    self.job_metadata_store.update(&job_id, &entry)?;
                    abandoned_over_cap += 1;
                    return Ok(());
                }
                *count += 1;

                // A pending job's `result` is detritus from a prior attempt;
                // don't pin it in the registry (persisted copy stays as-is).
                entry.result = None;
                pending.push((job_id, entry));
                Ok(())
            },
        )?;

        for (job_id, persisted_entry) in pending {
            // Load JobContext from job_data CF
            match self.job_data_store.get(&persisted_entry.tenant, &job_id)? {
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
                        result: None,
                        retry_count: persisted_entry.retry_count,
                        max_retries: persisted_entry.max_retries,
                        last_heartbeat: persisted_entry.last_heartbeat,
                        timeout_seconds: persisted_entry.timeout_seconds,
                        next_retry_at: persisted_entry.next_retry_at,
                        executing_since: persisted_entry.executing_since,
                    };

                    // Reset Running/Executing → Scheduled (crashed mid-execution)
                    if matches!(
                        job_info.status,
                        raisin_storage::jobs::JobStatus::Running
                            | raisin_storage::jobs::JobStatus::Executing
                    ) {
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
                            executing_since: persisted_entry.executing_since,
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
                    self.job_metadata_store
                        .delete(&persisted_entry.tenant, &job_id)?;
                }
            }
        }

        if abandoned_stale > 0 || abandoned_over_cap > 0 {
            tracing::warn!(
                abandoned_stale,
                abandoned_over_cap,
                restore_cap,
                max_restore_age_hours = MAX_RESTORE_AGE_HOURS,
                "Abandoned pending jobs at restart instead of restoring them"
            );
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
