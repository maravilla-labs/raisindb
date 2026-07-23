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

    /// List the `limit` most recent jobs (by `started_at`) for a tenant,
    /// streaming the prefix scan so the full backlog is never materialized.
    ///
    /// Result payloads larger than `max_inline_result_bytes` are replaced by
    /// a `{"$truncated": true, "size": n}` marker — the full result stays in
    /// storage and is served by single-job lookups. Serving surfaces MUST use
    /// this instead of `list_for_tenant`: after a runaway burst the persisted
    /// backlog can hold hundreds of thousands of entries with multi-KB result
    /// payloads, and one unbounded listing request (admin console jobs page,
    /// jobs SSE initial dump) then allocates gigabytes — the observed
    /// "RSS explodes the moment the jobs page is opened" failure.
    pub fn list_recent_for_tenant(
        &self,
        tenant: &str,
        limit: usize,
        max_inline_result_bytes: usize,
    ) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let prefix = job_tenant_prefix(tenant);

        // Min-heap of the newest `limit` entries, keyed by started_at.
        let mut newest: BinaryHeap<Reverse<(chrono::DateTime<chrono::Utc>, Vec<u8>)>> =
            BinaryHeap::new();

        let iter = self.db.prefix_iterator_cf(cf, &prefix);
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;
            if !key_bytes.starts_with(&prefix) {
                break;
            }
            // Cheap pre-parse: only started_at + tenant are needed to rank;
            // decode fully only for entries that make the cut, keyed by the
            // raw key so we can re-read the value lazily. To keep this in one
            // pass we decode here but drop losers immediately.
            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if entry.tenant != tenant {
                continue;
            }
            newest.push(Reverse((entry.started_at, key_bytes.to_vec())));
            if newest.len() > limit {
                newest.pop();
            }
        }

        let mut results = Vec::with_capacity(newest.len());
        for Reverse((_, key_bytes)) in newest.into_sorted_vec() {
            let Some(value_bytes) = self.db.get_cf(cf, &key_bytes).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to read job metadata: {}", e))
            })?
            else {
                continue;
            };
            let mut entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if let Some(result) = &entry.result {
                let size = serde_json::to_vec(result).map(|v| v.len()).unwrap_or(0);
                if size > max_inline_result_bytes {
                    entry.result = Some(serde_json::json!({
                        "$truncated": true,
                        "size": size,
                    }));
                }
            }
            let job_id = match split_key(&key_bytes) {
                Some((_, jid)) => jid,
                None => JobId::from_string(entry.id.clone()),
            };
            results.push((job_id, entry));
        }
        Ok(results)
    }

    /// Cross-tenant variant of [`list_recent_for_tenant`]: the `limit` most
    /// recent jobs across ALL tenants, streaming, with oversized results
    /// truncated. For operator/superadmin listing surfaces — the unbounded
    /// `list_all_unscoped` must never back an HTTP endpoint (the deploy
    /// health check hitting `/management/admin/jobs` against a runaway
    /// backlog was enough to OOM the server).
    pub fn list_recent_unscoped(
        &self,
        limit: usize,
        max_inline_result_bytes: usize,
    ) -> Result<Vec<(JobId, PersistedJobEntry)>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let mut newest: BinaryHeap<Reverse<(chrono::DateTime<chrono::Utc>, Vec<u8>)>> =
            BinaryHeap::new();

        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key_bytes, value_bytes) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate job metadata: {}", e))
            })?;
            let entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            newest.push(Reverse((entry.started_at, key_bytes.to_vec())));
            if newest.len() > limit {
                newest.pop();
            }
        }

        let mut results = Vec::with_capacity(newest.len());
        for Reverse((_, key_bytes)) in newest.into_sorted_vec() {
            let Some(value_bytes) = self.db.get_cf(cf, &key_bytes).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to read job metadata: {}", e))
            })?
            else {
                continue;
            };
            let mut entry: PersistedJobEntry = match rmp_serde::from_slice(&value_bytes) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if let Some(result) = &entry.result {
                let size = serde_json::to_vec(result).map(|v| v.len()).unwrap_or(0);
                if size > max_inline_result_bytes {
                    entry.result = Some(serde_json::json!({
                        "$truncated": true,
                        "size": size,
                    }));
                }
            }
            let job_id = match split_key(&key_bytes) {
                Some((_, jid)) => jid,
                None => JobId::from_string(entry.id.clone()),
            };
            results.push((job_id, entry));
        }
        Ok(results)
    }

    /// Stream every job matching `statuses` to `visit`, one entry at a time,
    /// without materializing the full result set.
    ///
    /// Crash-recovery scans use this instead of `list_by_status`: after a
    /// runaway registration burst the persisted backlog can hold hundreds of
    /// thousands of pending entries (each carrying result payloads), and
    /// collecting them into a `Vec` at startup multiplies that into gigabytes
    /// of RSS before the first job even runs.
    pub fn for_each_by_status<F>(&self, statuses: &[JobStatus], mut visit: F) -> Result<()>
    where
        F: FnMut(JobId, PersistedJobEntry) -> Result<()>,
    {
        let cf = cf_handle(&self.db, cf::JOB_METADATA)?;
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
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
            if !status_in_filter(&entry.status, statuses) {
                continue;
            }
            let job_id = match split_key(&key_bytes) {
                Some((_, jid)) => jid,
                None => JobId::from_string(entry.id.clone()),
            };
            visit(job_id, entry)?;
        }
        Ok(())
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
