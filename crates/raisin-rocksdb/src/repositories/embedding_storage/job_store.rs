//! RocksDB-backed embedding job store implementation.
//!
//! Manages embedding job lifecycle with state-prefixed keys in the embedding_jobs CF.

use crate::{cf, cf_handle};
use raisin_embeddings::{EmbeddingJob, EmbeddingJobStore};
use raisin_error::Result;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// RocksDB-backed embedding job store
///
/// Uses state-prefixed keys similar to fulltext job store:
/// - `pending:{job_id}` - Jobs waiting to be processed
/// - `processing:{job_id}` - Jobs currently being processed
/// - `failed:{job_id}` - Jobs that failed with errors
#[derive(Clone)]
pub struct RocksDBEmbeddingJobStore {
    db: Arc<DB>,
}

impl RocksDBEmbeddingJobStore {
    /// Create a new RocksDB embedding job store
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Build a job key with state prefix: `{state}:{job_id}`
    fn job_key(state: &str, job_id: &str) -> Vec<u8> {
        format!("{}:{}", state, job_id).into_bytes()
    }

    /// Build a state prefix for scanning: `{state}:`
    fn state_prefix(state: &str) -> Vec<u8> {
        format!("{}:", state).into_bytes()
    }

    /// Serialize a job
    fn serialize_job(job: &EmbeddingJob) -> Result<Vec<u8>> {
        rmp_serde::to_vec(job)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to serialize job: {}", e)))
    }

    /// Deserialize a job
    fn deserialize_job(bytes: &[u8]) -> Result<EmbeddingJob> {
        rmp_serde::from_slice(bytes)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to deserialize job: {}", e)))
    }
}

impl EmbeddingJobStore for RocksDBEmbeddingJobStore {
    fn enqueue(&self, job: &EmbeddingJob) -> Result<()> {
        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;
        let key = Self::job_key("pending", &job.job_id);
        let value = Self::serialize_job(job)?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to enqueue job: {}", e)))?;

        tracing::debug!(
            job_id = %job.job_id,
            kind = ?job.kind,
            tenant_id = %job.tenant_id,
            "Enqueued embedding job"
        );

        Ok(())
    }

    fn dequeue(&self, limit: usize) -> Result<Vec<EmbeddingJob>> {
        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;
        let prefix = Self::state_prefix("pending");
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let mut jobs = Vec::new();
        let mut batch = WriteBatch::default();

        // Collect up to `limit` pending jobs
        for item in iter.take(limit) {
            let (key, value) = item
                .map_err(|e| raisin_error::Error::storage(format!("Failed to read job: {}", e)))?;

            // Verify key has correct prefix
            if !key.starts_with(&prefix) {
                break;
            }

            // Deserialize job
            let job = Self::deserialize_job(&value)?;

            // Build new key for processing state
            let processing_key = Self::job_key("processing", &job.job_id);

            // Add to batch: delete from pending, add to processing
            batch.delete_cf(cf, &key);
            batch.put_cf(cf, processing_key, value);

            jobs.push(job);
        }

        // Atomically move all jobs from pending to processing
        if !jobs.is_empty() {
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to dequeue jobs: {}", e))
            })?;

            tracing::debug!(count = jobs.len(), "Dequeued embedding jobs");
        }

        Ok(jobs)
    }

    fn complete(&self, job_ids: &[String]) -> Result<()> {
        if job_ids.is_empty() {
            return Ok(());
        }

        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;
        let mut batch = WriteBatch::default();

        // Delete all processing jobs
        for job_id in job_ids {
            let key = Self::job_key("processing", job_id);
            batch.delete_cf(cf, key);
        }

        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to complete jobs: {}", e)))?;

        tracing::debug!(count = job_ids.len(), "Completed embedding jobs");

        Ok(())
    }

    fn fail(&self, job_id: &str, error: &str) -> Result<()> {
        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;
        let processing_key = Self::job_key("processing", job_id);

        // Read the job from processing state
        let job_bytes = self
            .db
            .get_cf(cf, &processing_key)
            .map_err(|e| {
                raisin_error::Error::storage(format!("Failed to read processing job: {}", e))
            })?
            .ok_or_else(|| {
                raisin_error::Error::storage(format!(
                    "Job {} not found in processing state",
                    job_id
                ))
            })?;

        let job = Self::deserialize_job(&job_bytes)?;

        // Log the error
        tracing::warn!(
            job_id = %job_id,
            error = %error,
            "Embedding job failed"
        );

        // Build failed state key and value
        let failed_key = Self::job_key("failed", job_id);
        let failed_value = Self::serialize_job(&job)?;

        // Atomically move from processing to failed
        let mut batch = WriteBatch::default();
        batch.delete_cf(cf, processing_key);
        batch.put_cf(cf, failed_key, failed_value);

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to mark job as failed: {}", e))
        })?;

        Ok(())
    }

    fn get(&self, job_id: &str) -> Result<Option<EmbeddingJob>> {
        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;

        // Check all states
        for state in &["pending", "processing", "failed"] {
            let key = Self::job_key(state, job_id);
            if let Some(bytes) = self
                .db
                .get_cf(cf, key)
                .map_err(|e| raisin_error::Error::storage(format!("Failed to get job: {}", e)))?
            {
                return Ok(Some(Self::deserialize_job(&bytes)?));
            }
        }

        Ok(None)
    }

    fn list_pending(&self) -> Result<Vec<EmbeddingJob>> {
        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;
        let prefix = Self::state_prefix("pending");
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let mut jobs = Vec::new();

        for result in iter {
            let (key, value) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate jobs: {}", e))
            })?;

            if !key.starts_with(&prefix) {
                break;
            }

            jobs.push(Self::deserialize_job(&value)?);
        }

        Ok(jobs)
    }

    fn count_pending(&self) -> Result<usize> {
        let cf = cf_handle(&self.db, cf::EMBEDDING_JOBS)?;
        let prefix = Self::state_prefix("pending");
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let count = iter
            .take_while(|result| {
                result
                    .as_ref()
                    .map(|(key, _)| key.starts_with(&prefix))
                    .unwrap_or(false)
            })
            .count();

        Ok(count)
    }
}
