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
            executing_since: job_info.executing_since,
        };

        // Persist on Tokio's blocking pool. `update` does a synchronous
        // `rocksdb::put_cf` that can park on RocksDB write-stall backpressure;
        // this runs for every job status change and every 10s heartbeat across all
        // in-flight jobs, so keeping it off the async job-pool worker threads stops
        // a write-heavy burst from parking the threads that drive the queue. Callers
        // already await this method, so ordering is unchanged.
        let store = self.clone();
        let job_id = job_id.clone();
        tokio::task::spawn_blocking(move || store.update(&job_id, &entry))
            .await
            .map_err(|e| {
                raisin_error::Error::storage(format!("Job persist task failed to join: {}", e))
            })?
    }

    async fn delete_job(&self, tenant: &str, job_id: &JobId) -> Result<()> {
        self.delete(tenant, job_id)
    }
}
