//! Key encoding for the JOB_METADATA and JOB_DATA column families.
//!
//! Job keys follow the `{tenant}\0{job_id}` layout so that tenant-scoped
//! prefix scans and `delete_range_cf` operations are structurally isolated.

/// Encode a job key as `{tenant_id}\0{job_id}`.
pub fn job_key(tenant_id: &str, job_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(tenant_id.len() + 1 + job_id.len());
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(0);
    k.extend_from_slice(job_id.as_bytes());
    k
}

/// Inclusive lower bound for prefix scans of a tenant's jobs.
pub fn job_tenant_prefix(tenant_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(tenant_id.len() + 1);
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(0);
    k
}

/// Exclusive upper bound for prefix scans of a tenant's jobs.
///
/// Used with `delete_range_cf(prefix, upper)` and similar APIs.
pub fn job_tenant_upper(tenant_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(tenant_id.len() + 1);
    k.extend_from_slice(tenant_id.as_bytes());
    k.push(1);
    k
}

/// Prefix for the write-maintained, newest-first job history index. A leading
/// NUL keeps index records outside every valid tenant job prefix.
const HISTORY_PREFIX: &[u8] = b"\0job-history\0";

pub fn is_job_history_key(key: &[u8]) -> bool {
    key.starts_with(HISTORY_PREFIX)
}

/// Newest-first index key for a persisted job.
///
/// Timestamp is inverted and encoded big-endian so RocksDB's ordinary forward
/// iterator returns the most recent records first. The job ID makes ties
/// deterministic without depending on wall-clock precision.
pub fn job_history_key(tenant_id: &str, started_at_ms: i64, job_id: &str) -> Vec<u8> {
    let mut key = job_history_prefix(tenant_id);
    key.extend_from_slice(&(u64::MAX - started_at_ms.max(0) as u64).to_be_bytes());
    key.push(0);
    key.extend_from_slice(job_id.as_bytes());
    key
}

/// Tenant-scoped prefix for cursor pagination of the history index.
pub fn job_history_prefix(tenant_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(HISTORY_PREFIX.len() + tenant_id.len() + 1);
    key.extend_from_slice(HISTORY_PREFIX);
    key.extend_from_slice(tenant_id.as_bytes());
    key.push(0);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_key_layout() {
        let k = job_key("tenant1", "job-123");
        assert_eq!(k, b"tenant1\0job-123".to_vec());
    }

    #[test]
    fn prefix_and_upper_bound_tenant() {
        let p = job_tenant_prefix("tenant1");
        let u = job_tenant_upper("tenant1");
        assert_eq!(p, b"tenant1\0".to_vec());
        assert_eq!(u, b"tenant1\x01".to_vec());
        assert!(p < u);
    }

    #[test]
    fn prefix_isolates_between_tenants() {
        let a = job_key("a", "j1");
        let b = job_key("b", "j1");
        let pa = job_tenant_prefix("a");
        let ua = job_tenant_upper("a");
        assert!(pa.as_slice() <= a.as_slice());
        assert!(a.as_slice() < ua.as_slice());
        assert!(b.as_slice() >= ua.as_slice());
    }

    #[test]
    fn history_keys_sort_newest_first_and_are_tenant_scoped() {
        let newer = job_history_key("a", 200, "newer");
        let older = job_history_key("a", 100, "older");
        assert!(newer < older);
        assert!(newer.starts_with(&job_history_prefix("a")));
        assert!(!newer.starts_with(&job_history_prefix("b")));
    }
}
