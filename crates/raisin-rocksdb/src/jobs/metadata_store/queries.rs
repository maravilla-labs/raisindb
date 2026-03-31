//! Query operations for job metadata (list by status, list all)

use super::{JobMetadataStore, PersistedJobEntry};
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_storage::jobs::{JobId, JobStatus};

impl JobMetadataStore {
    /// List all jobs matching specific statuses
    ///
    /// Used for crash recovery to find pending/running jobs that need restoration.
    pub fn list_by_status(
        &self,
        statuses: &[JobStatus],
    ) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let mut results = Vec::new();

        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            // Deserialize the entry - gracefully skip if deserialization fails
            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(e) => {
                    let job_id_str = String::from_utf8_lossy(&key_bytes);
                    tracing::warn!(
                        job_id = %job_id_str,
                        error = %e,
                        "Failed to deserialize job metadata (likely unknown job type), skipping"
                    );
                    continue;
                }
            };

            // Check if status matches filter
            let status_matches = statuses.iter().any(|filter_status| {
                matches!(
                    (&entry.status, filter_status),
                    (JobStatus::Scheduled, JobStatus::Scheduled)
                        | (JobStatus::Running, JobStatus::Running)
                        | (JobStatus::Executing, JobStatus::Executing)
                        | (JobStatus::Completed, JobStatus::Completed)
                        | (JobStatus::Cancelled, JobStatus::Cancelled)
                        | (JobStatus::Failed(_), JobStatus::Failed(_))
                )
            });

            if status_matches {
                let job_id_str = String::from_utf8_lossy(&key_bytes).to_string();
                results.push((JobId::from_string(job_id_str), entry));
            }
        }

        Ok(results)
    }

    /// List all jobs from persistent storage
    ///
    /// Returns all jobs regardless of status, useful for displaying complete job history.
    pub fn list_all(&self) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let mut results = Vec::new();

        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            let job_id_str = String::from_utf8(key_bytes.to_vec())
                .map_err(|e| raisin_error::Error::storage(format!("Invalid job ID key: {}", e)))?;
            let job_id = JobId::from_string(job_id_str.clone());

            // Deserialize the entry - gracefully skip if deserialization fails
            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(
                        job_id = %job_id_str,
                        error = %e,
                        "Failed to deserialize job metadata (likely unknown job type), skipping"
                    );
                    continue;
                }
            };

            results.push((job_id, entry));
        }

        Ok(results)
    }

    /// Count total and orphaned entries in JOB_METADATA
    ///
    /// # Returns
    /// Tuple of (total_entries, orphaned_entries)
    pub fn count_entries(&self) -> Result<(usize, usize)> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;

        let mut total = 0;
        let mut orphaned = 0;

        let iter = self
            .db
            .iterator_cf(cf_metadata, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            total += 1;
            if rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes).is_err() {
                orphaned += 1;
            }
        }

        Ok((total, orphaned))
    }
}
