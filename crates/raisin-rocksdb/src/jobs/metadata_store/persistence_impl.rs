//! JobPersistence trait implementation for RocksDB-backed storage

use super::{JobMetadataStore, PersistedJobEntry};
use async_trait::async_trait;
use raisin_error::Result;
use raisin_storage::jobs::{JobId, JobInfo, JobPersistence};

/// Implementation of JobPersistence trait for RocksDB-backed storage
///
/// This enables the JobRegistry to persist job state changes without
/// being directly coupled to RocksDB.
#[async_trait]
impl JobPersistence for JobMetadataStore {
    async fn persist_job(&self, job_id: &JobId, job_info: &JobInfo) -> Result<()> {
        // Convert JobInfo to PersistedJobEntry
        let entry = PersistedJobEntry {
            id: job_id.0.clone(),
            job_type: job_info.job_type.clone(),
            status: job_info.status.clone(),
            tenant: job_info.tenant.clone(),
            started_at: job_info.started_at,
            completed_at: job_info.completed_at,
            error: job_info.error.clone(),
            progress: job_info.progress,
            result: job_info.result.clone(),
            retry_count: job_info.retry_count,
            max_retries: job_info.max_retries,
            last_heartbeat: job_info.last_heartbeat,
            timeout_seconds: job_info.timeout_seconds,
            next_retry_at: job_info.next_retry_at,
        };

        // Persist using synchronous update (JobMetadataStore methods are sync)
        self.update(job_id, &entry)
    }

    async fn delete_job(&self, job_id: &JobId) -> Result<()> {
        self.delete(job_id)
    }
}
