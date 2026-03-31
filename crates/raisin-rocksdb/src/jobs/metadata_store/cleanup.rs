//! Cleanup, delete, and purge operations for job metadata

use super::{JobMetadataStore, PersistedJobEntry};
use crate::{cf, cf_handle};
use chrono::{DateTime, Utc};
use raisin_error::Result;
use raisin_storage::jobs::{JobId, JobStatus};
use rocksdb::WriteBatch;

impl JobMetadataStore {
    /// Delete old completed/failed jobs (retention policy)
    ///
    /// Removes jobs completed before the cutoff timestamp to prevent
    /// unbounded growth of job history.
    pub fn cleanup_old_jobs(&self, older_than: DateTime<Utc>) -> Result<usize> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let mut deleted_count = 0;
        let mut keys_to_delete = Vec::new();

        // First, collect keys to delete (can't modify while iterating)
        let iter = self
            .db
            .iterator_cf(cf_metadata, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            // Deserialize the entry - if deserialization fails, the entry is orphaned
            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(e) => {
                    let job_id_str = String::from_utf8_lossy(&key_bytes);
                    tracing::info!(
                        job_id = %job_id_str,
                        error = %e,
                        "Cleaning up undeserializable job entry (orphaned/corrupted)"
                    );
                    keys_to_delete.push(key_bytes.to_vec());
                    continue;
                }
            };

            // Only delete completed/failed/cancelled jobs
            let is_terminal = matches!(
                entry.status,
                JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
            );

            if is_terminal {
                if let Some(completed_at) = entry.completed_at {
                    if completed_at < older_than {
                        keys_to_delete.push(key_bytes.to_vec());
                    }
                }
            }
        }

        // Delete in batch
        if !keys_to_delete.is_empty() {
            let mut batch = WriteBatch::default();

            for key in &keys_to_delete {
                batch.delete_cf(cf_metadata, key);
                batch.delete_cf(cf_data, key);
                deleted_count += 1;
            }

            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete old jobs: {}", e))
            })?;
        }

        Ok(deleted_count)
    }

    /// Delete specific job metadata and context
    pub fn delete(&self, job_id: &JobId) -> Result<()> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_id.as_str().as_bytes();

        let mut batch = WriteBatch::default();
        batch.delete_cf(cf_metadata, key);
        batch.delete_cf(cf_data, key);

        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to delete job: {}", e)))?;

        Ok(())
    }

    /// Delete multiple jobs in a single batch operation
    ///
    /// Only deletes jobs with terminal status (Completed, Cancelled, Failed).
    /// Jobs that are Running or Scheduled are skipped.
    pub fn delete_batch(&self, job_ids: &[JobId]) -> Result<(usize, usize)> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let mut deleted_count = 0;
        let mut skipped_count = 0;
        let mut batch = WriteBatch::default();

        for job_id in job_ids {
            let key = job_id.as_str().as_bytes();

            match self.db.get_cf(cf_metadata, key) {
                Ok(Some(value_bytes)) => {
                    match rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes) {
                        Ok(entry) => {
                            let can_delete =
                                !matches!(entry.status, JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled);

                            if can_delete {
                                batch.delete_cf(cf_metadata, key);
                                batch.delete_cf(cf_data, key);
                                deleted_count += 1;
                            } else {
                                tracing::debug!(
                                    job_id = %job_id,
                                    status = ?entry.status,
                                    "Skipping job - still running/scheduled"
                                );
                                skipped_count += 1;
                            }
                        }
                        Err(e) => {
                            tracing::info!(
                                job_id = %job_id,
                                error = %e,
                                "Deleting undeserializable job entry (orphaned/corrupted)"
                            );
                            batch.delete_cf(cf_metadata, key);
                            batch.delete_cf(cf_data, key);
                            deleted_count += 1;
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!(job_id = %job_id, "Job not found in persistent storage");
                    skipped_count += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %job_id,
                        error = %e,
                        "Failed to read job for deletion"
                    );
                    skipped_count += 1;
                }
            }
        }

        if deleted_count > 0 {
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to batch delete jobs: {}", e))
            })?;
        }

        tracing::info!(
            deleted = deleted_count,
            skipped = skipped_count,
            total = job_ids.len(),
            "Batch deleted jobs from persistent storage"
        );

        Ok((deleted_count, skipped_count))
    }

    /// Purge ALL job entries from persistent storage (nuclear option)
    pub fn purge_all(&self) -> Result<usize> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let mut keys: Vec<Vec<u8>> = Vec::new();

        let iter = self
            .db
            .iterator_cf(cf_metadata, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key_bytes, _) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;
            keys.push(key_bytes.to_vec());
        }

        let count = keys.len();
        if count > 0 {
            let mut batch = WriteBatch::default();
            for key in &keys {
                batch.delete_cf(cf_metadata, key);
                batch.delete_cf(cf_data, key);
            }
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to purge all jobs: {}", e))
            })?;
        }

        tracing::info!(purged = count, "Purged all jobs from persistent storage");
        Ok(count)
    }

    /// Purge only orphaned (undeserializable) job entries
    pub fn purge_orphaned(&self) -> Result<usize> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let mut orphaned_keys: Vec<Vec<u8>> = Vec::new();

        let iter = self
            .db
            .iterator_cf(cf_metadata, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            if rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes).is_err() {
                orphaned_keys.push(key_bytes.to_vec());
            }
        }

        let count = orphaned_keys.len();
        if count > 0 {
            let mut batch = WriteBatch::default();
            for key in &orphaned_keys {
                batch.delete_cf(cf_metadata, key);
                batch.delete_cf(cf_data, key);
            }
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to purge orphaned jobs: {}", e))
            })?;
        }

        tracing::info!(
            purged = count,
            "Purged orphaned jobs from persistent storage"
        );
        Ok(count)
    }
}
