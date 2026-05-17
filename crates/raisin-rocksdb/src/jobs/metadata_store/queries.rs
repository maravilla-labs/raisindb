//! Query operations for job metadata (list by status, list all)

use super::{JobMetadataStore, PersistedJobEntry};
use crate::keys::job_tenant_prefix;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_storage::jobs::{JobId, JobStatus};

/// Split a `{tenant}\0{job_id}` key into (tenant, job_id). Returns None for
/// malformed keys (no separator).
fn split_key(key: &[u8]) -> Option<(String, JobId)> {
    let sep = key.iter().position(|&b| b == 0)?;
    let tenant = String::from_utf8(key[..sep].to_vec()).ok()?;
    let job_id_str = String::from_utf8(key[sep + 1..].to_vec()).ok()?;
    Some((tenant, JobId::from_string(job_id_str)))
}

impl JobMetadataStore {
    /// List all jobs matching specific statuses across every tenant.
    ///
    /// Used for crash recovery to find pending/running jobs that need restoration.
    /// **Internal use only** — HTTP handlers MUST go through `list_by_status_for_tenant`.
    pub fn list_by_status(
        &self,
        statuses: &[JobStatus],
    ) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        self.collect_matching(rocksdb::IteratorMode::Start, |entry| {
            status_in_filter(&entry.status, statuses)
        })
    }

    /// List jobs matching specific statuses for a single tenant.
    pub fn list_by_status_for_tenant(
        &self,
        tenant: &str,
        statuses: &[JobStatus],
    ) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        let all = self.list_for_tenant(tenant)?;
        Ok(all
            .into_iter()
            .filter(|(_, e)| status_in_filter(&e.status, statuses))
            .collect())
    }

    /// List every job in persistent storage (across all tenants).
    ///
    /// **Internal use only** — HTTP surface uses `list_for_tenant`.
    pub fn list_all_unscoped(&self) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        self.collect_matching(rocksdb::IteratorMode::Start, |_| true)
    }

    /// Backwards-compatible alias for `list_all_unscoped`.
    ///
    /// Used by a handful of internal call sites still in flight; prefer
    /// `list_for_tenant` or `list_all_unscoped` directly.
    pub fn list_all(&self) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        self.list_all_unscoped()
    }

    /// List jobs for a single tenant by scanning the `{tenant}\0` prefix.
    pub fn list_for_tenant(&self, tenant: &str) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let prefix = job_tenant_prefix(tenant);
        let mut results = Vec::new();

        let iter = self.db.prefix_iterator_cf(cf, &prefix);
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            // prefix_iterator_cf may emit rows past the prefix (depending on bloom
            // filter behavior); guard explicitly.
            if !key_bytes.starts_with(&prefix) {
                break;
            }

            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(e) => {
                    let key_str = String::from_utf8_lossy(&key_bytes);
                    tracing::warn!(
                        key = %key_str,
                        error = %e,
                        "Failed to deserialize job metadata (likely unknown job type), skipping"
                    );
                    continue;
                }
            };
            if entry.tenant != tenant {
                continue;
            }
            let job_id = match split_key(&key_bytes) {
                Some((_, jid)) => jid,
                None => JobId::from_string(entry.id.clone()),
            };
            results.push((job_id, entry));
        }

        Ok(results)
    }

    /// Count total and orphaned entries in JOB_METADATA across all tenants.
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

    /// Count total and orphaned entries in JOB_METADATA for one tenant.
    pub fn count_entries_for_tenant(&self, tenant: &str) -> Result<(usize, usize)> {
        let cf_metadata = cf_handle(&self.db, cf::JOB_METADATA)?;
        let prefix = job_tenant_prefix(tenant);

        let mut total = 0;
        let mut orphaned = 0;

        let iter = self.db.prefix_iterator_cf(cf_metadata, &prefix);
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;
            if !key_bytes.starts_with(&prefix) {
                break;
            }
            total += 1;
            match rmp_serde::from_slice::<PersistedJobEntry>(&value_bytes) {
                Ok(entry) if entry.tenant != tenant => {
                    // Foreign tenant slipping through — treat as orphan for this tenant.
                    orphaned += 1;
                }
                Err(_) => {
                    orphaned += 1;
                }
                Ok(_) => {}
            }
        }

        Ok((total, orphaned))
    }

    /// Shared iteration helper.
    fn collect_matching<F>(
        &self,
        mode: rocksdb::IteratorMode,
        mut keep: F,
    ) -> Result<Vec<(JobId, PersistedJobEntry)>>
    where
        F: FnMut(&PersistedJobEntry) -> bool,
    {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let mut results = Vec::new();

        let iter = self.db.iterator_cf(cf, mode);
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;

            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(e) => {
                    let key_str = String::from_utf8_lossy(&key_bytes);
                    tracing::warn!(
                        key = %key_str,
                        error = %e,
                        "Failed to deserialize job metadata (likely unknown job type), skipping"
                    );
                    continue;
                }
            };

            if !keep(&entry) {
                continue;
            }

            let job_id = match split_key(&key_bytes) {
                Some((_, jid)) => jid,
                None => JobId::from_string(entry.id.clone()),
            };
            results.push((job_id, entry));
        }

        Ok(results)
    }
}

fn status_in_filter(actual: &JobStatus, statuses: &[JobStatus]) -> bool {
    statuses.iter().any(|filter_status| {
        matches!(
            (actual, filter_status),
            (JobStatus::Scheduled, JobStatus::Scheduled)
                | (JobStatus::Running, JobStatus::Running)
                | (JobStatus::Executing, JobStatus::Executing)
                | (JobStatus::Completed, JobStatus::Completed)
                | (JobStatus::Cancelled, JobStatus::Cancelled)
                | (JobStatus::Failed(_), JobStatus::Failed(_))
        )
    })
}
