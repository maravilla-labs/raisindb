//! Cleanup, delete, and purge operations for job metadata

use super::{JobMetadataStore, PersistedJobEntry};
use crate::keys::{is_job_history_key, job_history_key, job_key, job_tenant_prefix};
use crate::{cf, cf_handle};
use chrono::{DateTime, Utc};
use raisin_error::Result;
use raisin_storage::jobs::{JobId, JobStatus};
use rocksdb::WriteBatch;

impl JobMetadataStore {
    /// Delete old terminal jobs across every tenant (retention policy).
    ///
    /// Internal/background-task only — HTTP surface uses
    /// `cleanup_old_jobs_for_tenant`.
    pub fn cleanup_old_jobs(&self, older_than: DateTime<Utc>) -> Result<usize> {
        self.cleanup_old_jobs_inner(None, older_than)
    }

    /// Delete old terminal jobs for one tenant.
    pub fn cleanup_old_jobs_for_tenant(
        &self,
        tenant: &str,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        self.cleanup_old_jobs_inner(Some(tenant), older_than)
    }

    fn cleanup_old_jobs_inner(
        &self,
        tenant_filter: Option<&str>,
        older_than: DateTime<Utc>,
    ) -> Result<usize> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let mut deleted_count = 0;
        let mut keys_to_delete: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();

        let prefix = tenant_filter.map(job_tenant_prefix);
        let iter = if let Some(ref p) = prefix {
            self.db.prefix_iterator_cf(cf_metadata, p.as_slice())
        } else {
            self.db
                .iterator_cf(cf_metadata, rocksdb::IteratorMode::Start)
        };

        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            if is_job_history_key(&key_bytes) {
                continue;
            }

            if let Some(ref p) = prefix {
                if !key_bytes.starts_with(p) {
                    break;
                }
            }

            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(e) => {
                    let key_str = String::from_utf8_lossy(&key_bytes);
                    tracing::info!(
                        key = %key_str,
                        error = %e,
                        "Cleaning up undeserializable job entry (orphaned/corrupted)"
                    );
                    keys_to_delete.push((key_bytes.to_vec(), None));
                    continue;
                }
            };

            if let Some(t) = tenant_filter {
                if entry.tenant != t {
                    continue;
                }
            }

            let is_terminal = matches!(
                entry.status,
                JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled
            );

            if is_terminal {
                if let Some(completed_at) = entry.completed_at {
                    if completed_at < older_than {
                        keys_to_delete.push((
                            key_bytes.to_vec(),
                            Some(job_history_key(
                                &entry.tenant,
                                entry.started_at.timestamp_millis(),
                                &entry.id,
                            )),
                        ));
                    }
                }
            }
        }

        if !keys_to_delete.is_empty() {
            let mut batch = WriteBatch::default();
            for (key, history_key) in &keys_to_delete {
                batch.delete_cf(cf_metadata, key);
                batch.delete_cf(cf_data, key);
                if let Some(history_key) = history_key {
                    batch.delete_cf(cf_metadata, history_key);
                }
                deleted_count += 1;
            }
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete old jobs: {}", e))
            })?;
        }

        Ok(deleted_count)
    }

    /// Delete specific job metadata and context for a tenant.
    pub fn delete(&self, tenant: &str, job_id: &JobId) -> Result<()> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;
        let key = job_key(tenant, job_id.as_str());
        let history_key = self
            .db
            .get_cf(cf_metadata, &key)
            .ok()
            .flatten()
            .and_then(|value| rmp_serde::from_slice::<PersistedJobEntry>(&value).ok())
            .map(|entry| {
                job_history_key(tenant, entry.started_at.timestamp_millis(), job_id.as_str())
            });

        let mut batch = WriteBatch::default();
        batch.delete_cf(cf_metadata, &key);
        batch.delete_cf(cf_data, &key);
        if let Some(history_key) = history_key {
            batch.delete_cf(cf_metadata, history_key);
        }

        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to delete job: {}", e)))?;

        Ok(())
    }

    /// Delete multiple jobs in a single batch operation within one tenant.
    ///
    /// Only deletes jobs with terminal status (Completed, Cancelled, Failed).
    /// Jobs that are Running or Scheduled are skipped.
    pub fn delete_batch(&self, tenant: &str, job_ids: &[JobId]) -> Result<(usize, usize)> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let mut deleted_count = 0;
        let mut skipped_count = 0;
        let mut batch = WriteBatch::default();

        for job_id in job_ids {
            let key = job_key(tenant, job_id.as_str());

            match self.db.get_cf(cf_metadata, &key) {
                Ok(Some(value_bytes)) => {
                    match rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes) {
                        Ok(entry) => {
                            if entry.tenant != tenant {
                                skipped_count += 1;
                                continue;
                            }
                            let can_delete = !matches!(
                                entry.status,
                                JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled
                            );

                            if can_delete {
                                batch.delete_cf(cf_metadata, &key);
                                batch.delete_cf(cf_data, &key);
                                batch.delete_cf(
                                    cf_metadata,
                                    job_history_key(
                                        tenant,
                                        entry.started_at.timestamp_millis(),
                                        job_id.as_str(),
                                    ),
                                );
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
                            batch.delete_cf(cf_metadata, &key);
                            batch.delete_cf(cf_data, &key);
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

    /// Purge ALL job entries from persistent storage (nuclear option).
    ///
    /// Internal/admin only.
    pub fn purge_all(&self) -> Result<usize> {
        self.purge_inner(None, false)
    }

    /// Purge all jobs for a single tenant via prefix scan.
    pub fn purge_all_for_tenant(&self, tenant: &str) -> Result<usize> {
        self.purge_inner(Some(tenant), false)
    }

    /// Purge only orphaned (undeserializable) job entries across all tenants.
    pub fn purge_orphaned(&self) -> Result<usize> {
        self.purge_inner(None, true)
    }

    /// Purge orphaned (undeserializable) entries within one tenant.
    pub fn purge_orphaned_for_tenant(&self, tenant: &str) -> Result<usize> {
        self.purge_inner(Some(tenant), true)
    }

    fn purge_inner(&self, tenant_filter: Option<&str>, only_orphaned: bool) -> Result<usize> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let cf_data = cf_handle(&self.db, cf::JOB_DATA)?;

        let prefix = tenant_filter.map(job_tenant_prefix);
        let iter = if let Some(ref p) = prefix {
            self.db.prefix_iterator_cf(cf_metadata, p.as_slice())
        } else {
            self.db
                .iterator_cf(cf_metadata, rocksdb::IteratorMode::Start)
        };

        let mut keys: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;
            if is_job_history_key(&key_bytes) {
                continue;
            }
            if let Some(ref p) = prefix {
                if !key_bytes.starts_with(p) {
                    break;
                }
            }

            if only_orphaned {
                if rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes).is_err() {
                    keys.push((key_bytes.to_vec(), None));
                }
            } else if let Some(t) = tenant_filter {
                // Belt-and-braces tenant equality.
                match rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes) {
                    Ok(entry) if entry.tenant == t => keys.push((
                        key_bytes.to_vec(),
                        Some(job_history_key(
                            &entry.tenant,
                            entry.started_at.timestamp_millis(),
                            &entry.id,
                        )),
                    )),
                    Ok(_) => {}
                    Err(_) => keys.push((key_bytes.to_vec(), None)),
                }
            } else {
                let history_key = rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes)
                    .ok()
                    .map(|entry| {
                        job_history_key(
                            &entry.tenant,
                            entry.started_at.timestamp_millis(),
                            &entry.id,
                        )
                    });
                keys.push((key_bytes.to_vec(), history_key));
            }
        }

        let count = keys.len();
        if count > 0 {
            let mut batch = WriteBatch::default();
            for (key, history_key) in &keys {
                batch.delete_cf(cf_metadata, key);
                batch.delete_cf(cf_data, key);
                if let Some(history_key) = history_key {
                    batch.delete_cf(cf_metadata, history_key);
                }
            }
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to purge jobs: {}", e))
            })?;
        }

        tracing::info!(
            purged = count,
            tenant = tenant_filter.unwrap_or(""),
            orphaned_only = only_orphaned,
            "Purged jobs from persistent storage"
        );
        Ok(count)
    }
}
