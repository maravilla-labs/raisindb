//! Full-text indexing job store implementation using RocksDB
//!
//! This module provides a persistent, crash-safe job queue for full-text indexing operations.
//! Jobs are stored in different states (pending, processing, failed, completed) and can be
//! dequeued in FIFO order for processing.

use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_storage::{FullTextIndexJob, FullTextJobStore};
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// RocksDB-backed full-text indexing job store
///
/// This implementation uses a column family with state-prefixed keys to maintain
/// a persistent job queue. Jobs transition through states:
/// - `pending` → `processing` → deleted (on success)
/// - `pending` → `processing` → `failed` (on failure)
#[derive(Clone)]
pub struct RocksDbJobStore {
    db: Arc<DB>,
}

impl RocksDbJobStore {
    /// Create a new RocksDB job store
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

    /// Extract job_id from a key with format `{state}:{job_id}`
    fn extract_job_id(key: &[u8], state_prefix: &[u8]) -> Option<String> {
        if !key.starts_with(state_prefix) {
            return None;
        }
        let job_id_bytes = &key[state_prefix.len()..];
        String::from_utf8(job_id_bytes.to_vec()).ok()
    }

    /// Serialize a job to JSON
    fn serialize_job(job: &FullTextIndexJob) -> Result<Vec<u8>> {
        rmp_serde::to_vec(job)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to serialize job: {}", e)))
    }

    /// Deserialize a job from JSON
    fn deserialize_job(bytes: &[u8]) -> Result<FullTextIndexJob> {
        rmp_serde::from_slice(bytes)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to deserialize job: {}", e)))
    }
}

impl FullTextJobStore for RocksDbJobStore {
    fn enqueue(&self, job: &FullTextIndexJob) -> Result<()> {
        let cf = cf_handle(&self.db, cf::FULLTEXT_JOBS)?;
        let key = Self::job_key("pending", &job.job_id);
        let value = Self::serialize_job(job)?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to enqueue job: {}", e)))?;

        Ok(())
    }

    fn dequeue(&self, count: usize) -> Result<Vec<FullTextIndexJob>> {
        let cf = cf_handle(&self.db, cf::FULLTEXT_JOBS)?;
        let prefix = Self::state_prefix("pending");
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let mut jobs = Vec::new();
        let mut batch = WriteBatch::default();

        // Collect up to `count` pending jobs
        for item in iter.take(count) {
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
                raisin_error::Error::storage(format!("Failed to dequeue jobs atomically: {}", e))
            })?;
        }

        Ok(jobs)
    }

    fn complete(&self, job_ids: &[String]) -> Result<()> {
        if job_ids.is_empty() {
            return Ok(());
        }

        let cf = cf_handle(&self.db, cf::FULLTEXT_JOBS)?;
        let mut batch = WriteBatch::default();

        // Delete all processing jobs
        for job_id in job_ids {
            let key = Self::job_key("processing", job_id);
            batch.delete_cf(cf, key);
        }

        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to complete jobs: {}", e)))?;

        Ok(())
    }

    fn fail(&self, job_id: &str, error: &str) -> Result<()> {
        let cf = cf_handle(&self.db, cf::FULLTEXT_JOBS)?;
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

        // Augment job with error information (store in a custom field if needed)
        // For now, we just log the error and mark the job as failed
        tracing::warn!(
            job_id = %job_id,
            error = %error,
            "Job failed"
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::workspace::WorkspaceConfig;
    use raisin_storage::JobKind;

    fn create_test_job(job_id: &str) -> FullTextIndexJob {
        FullTextIndexJob {
            job_id: job_id.to_string(),
            kind: JobKind::AddNode,
            tenant_id: "test-tenant".to_string(),
            repo_id: "test-repo".to_string(),
            workspace_id: "test-workspace".to_string(),
            branch: "main".to_string(),
            revision: raisin_hlc::HLC::new(1, 0),
            node_id: Some("node-123".to_string()),
            source_branch: None,
            default_language: "en".to_string(),
            supported_languages: vec!["en".to_string()],
            properties_to_index: Some(vec![]),
        }
    }

    #[test]
    fn test_job_key_format() {
        let key = RocksDbJobStore::job_key("pending", "job-123");
        assert_eq!(key, b"pending:job-123");

        let key = RocksDbJobStore::job_key("processing", "job-456");
        assert_eq!(key, b"processing:job-456");
    }

    #[test]
    fn test_state_prefix() {
        let prefix = RocksDbJobStore::state_prefix("pending");
        assert_eq!(prefix, b"pending:");
    }

    #[test]
    fn test_extract_job_id() {
        let key = b"pending:job-123";
        let prefix = RocksDbJobStore::state_prefix("pending");
        let job_id = RocksDbJobStore::extract_job_id(key, &prefix);
        assert_eq!(job_id, Some("job-123".to_string()));

        // Wrong prefix
        let key = b"processing:job-123";
        let prefix = RocksDbJobStore::state_prefix("pending");
        let job_id = RocksDbJobStore::extract_job_id(key, &prefix);
        assert_eq!(job_id, None);
    }

    #[test]
    fn test_serialize_deserialize() {
        let job = create_test_job("job-123");
        let serialized = RocksDbJobStore::serialize_job(&job).unwrap();
        let deserialized = RocksDbJobStore::deserialize_job(&serialized).unwrap();
        assert_eq!(job.job_id, deserialized.job_id);
        assert_eq!(job.tenant_id, deserialized.tenant_id);
    }
}
