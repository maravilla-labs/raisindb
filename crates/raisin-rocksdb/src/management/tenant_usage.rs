//! Real storage usage for a tenant (and for the whole database).
//!
//! # How a tenant's bytes are attributed
//!
//! Every column family in [`TENANT_PREFIXED_CFS`] keys its records
//! `{tenant}\0…`, so one tenant's data is the contiguous key range
//! `[{tenant}\0, {tenant}\x01)`. RocksDB's `GetApproximateSizes` answers "how
//! many SST bytes does this range occupy" from the table index blocks alone: no
//! key is read, so the cost is O(files overlapping the range), not O(data).
//! That is the whole reason this is safe to serve per request.
//!
//! Isolation: the range ends at `{tenant}\x01`, so a tenant `acme` never
//! includes `acme2` (`acme2\0…` sorts after `acme\x01`).
//!
//! # Honest limits
//!
//! * The figure is compressed on-disk SST size, approximate by design. Data
//!   still only in the memtable is invisible to the size API; for a column
//!   family whose SST estimate is 0 we therefore read at most
//!   [`MEMTABLE_PROBE_KEYS`] keys so a freshly written tenant is not reported as
//!   empty. A large tenant that has never flushed is under-counted until its
//!   memtable flushes.
//! * Tantivy full-text and HNSW directories live outside RocksDB and are not
//!   included. `EMBEDDINGS` (a column family) is.
//! * `node_count` is not computed (it would need a scan); it is reported as 0.
//!   `index_sizes` always lists every index column family (zero or not) so a
//!   consumer can tell a measured empty value from "not implemented".
//! * A tenant that is not registered anywhere is [`Error::NotFound`], never
//!   zeros.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_error::{Error, Result};
use raisin_storage::Metrics;
use rocksdb::{Range, DB};

use crate::keys::{job_tenant_prefix, job_tenant_upper, KeyBuilder};
use crate::storage::tenant_wipe::TENANT_PREFIXED_CFS;
use crate::storage::tenant_wipe_collision::{collides_with_kind, tenant_repos};
use crate::{cf, cf_handle, RocksDBStorage};

/// Upper bound on keys read per column family when its SST estimate is 0.
const MEMTABLE_PROBE_KEYS: usize = 1000;

/// Column families that are derived indexes (reported under `index_sizes`).
const INDEX_CFS: &[&str] = &[
    cf::PATH_INDEX,
    cf::PROPERTY_INDEX,
    cf::REFERENCE_INDEX,
    cf::RELATION_INDEX,
    cf::ORDER_INDEX,
    cf::ORDERED_CHILDREN,
    cf::NODE_PATH,
    cf::COMPOUND_INDEX,
    cf::UNIQUE_INDEX,
    cf::SPATIAL_INDEX,
    cf::LOCALIZED_NAME_INDEX,
    cf::TRANSLATION_INDEX,
    cf::EMBEDDINGS,
];

/// Whether the tenant is known to this database (registered, has a repository,
/// or has an admin user). Unknown tenants must 404, not read as zero bytes.
fn tenant_exists(db: &DB, tenant: &str) -> Result<bool> {
    let registry = cf_handle(db, cf::REGISTRY)?;
    let map = |e: rocksdb::Error| Error::storage(e.to_string());
    if db
        .get_cf(registry, crate::keys::tenant_key(tenant))
        .map_err(map)?
        .is_some()
    {
        return Ok(true);
    }
    let repos = KeyBuilder::new().push(tenant).push("repos").build_prefix();
    if let Some(item) = crate::prefix_scan(db, registry, &repos).next() {
        let (key, _) = item.map_err(map)?;
        if key.starts_with(&repos) {
            return Ok(true);
        }
    }
    let admins = cf_handle(db, cf::ADMIN_USERS)?;
    let mut prefix = Vec::with_capacity(tenant.len() + 5);
    prefix.extend_from_slice(b"sys\0");
    prefix.extend_from_slice(tenant.as_bytes());
    prefix.push(0);
    if let Some(item) = crate::prefix_scan(db, admins, &prefix).next() {
        let (key, _) = item.map_err(map)?;
        if key.starts_with(&prefix) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Key ranges holding `tenant`'s records in `cf_name`.
///
/// Normally one `[{tenant}\0, {tenant}\x01)`. A tenant NAMED like a kind-first
/// record kind would otherwise swallow other tenants' records in a
/// mixed-layout column family, so there it is measured per repository.
fn ranges_for(
    cf_name: &str,
    tenant: &str,
    collision_repos: &Option<Vec<String>>,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    if let Some(repos) = collision_repos {
        if crate::storage::repo_purge::mixed_layout_cfs().any(|m| m == cf_name) {
            return repos
                .iter()
                .filter_map(|repo| {
                    let lo = KeyBuilder::new().push(tenant).push(repo).build_prefix();
                    crate::prefix_successor(&lo).map(|hi| (lo, hi))
                })
                .collect();
        }
    }
    vec![(job_tenant_prefix(tenant), job_tenant_upper(tenant))]
}

/// Bytes of one column family attributable to `tenant`.
fn cf_bytes(db: &DB, cf_name: &str, ranges: &[(Vec<u8>, Vec<u8>)]) -> Result<u64> {
    let handle = cf_handle(db, cf_name)?;
    let rocks: Vec<Range> = ranges
        .iter()
        .map(|(lo, hi)| Range::new(lo.as_slice(), hi.as_slice()))
        .collect();
    let sst: u64 = db.get_approximate_sizes_cf(handle, &rocks).iter().sum();
    if sst > 0 {
        return Ok(sst);
    }
    // Possibly memtable-only data: a bounded probe, so "0" means empty.
    let mut bytes = 0u64;
    let mut seen = 0usize;
    for (lo, _) in ranges {
        for item in crate::prefix_scan(db, handle, lo) {
            let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
            if !key.starts_with(lo) || seen >= MEMTABLE_PROBE_KEYS {
                break;
            }
            bytes += (key.len() + value.len()) as u64;
            seen += 1;
        }
    }
    Ok(bytes)
}

fn measure_tenant(db: &Arc<DB>, tenant: &str) -> Result<Metrics> {
    if tenant.is_empty() || tenant.contains('\0') || !tenant_exists(db, tenant)? {
        return Err(Error::NotFound(format!("tenant '{tenant}'")));
    }
    let collision_repos = if collides_with_kind(tenant) {
        Some(tenant_repos(db, tenant)?)
    } else {
        None
    };

    let mut total = 0u64;
    let mut index_sizes: HashMap<String, u64> = INDEX_CFS
        .iter()
        .map(|name| ((*name).to_string(), 0))
        .collect();
    for cf_name in TENANT_PREFIXED_CFS {
        let ranges = ranges_for(cf_name, tenant, &collision_repos);
        let bytes = cf_bytes(db, cf_name, &ranges)?;
        total = total.saturating_add(bytes);
        if let Some(slot) = index_sizes.get_mut(*cf_name) {
            *slot = bytes;
        }
    }
    // Admin users: `sys\0{tenant}\0…` (not tenant-first).
    let mut lo = b"sys\0".to_vec();
    lo.extend_from_slice(tenant.as_bytes());
    lo.push(0);
    let mut hi = b"sys\0".to_vec();
    hi.extend_from_slice(tenant.as_bytes());
    hi.push(1);
    total = total.saturating_add(cf_bytes(db, cf::ADMIN_USERS, &[(lo, hi)])?);

    Ok(Metrics {
        tenant: Some(tenant.to_string()),
        operations_per_sec: 0.0,
        error_rate: 0.0,
        disk_usage_bytes: total,
        index_sizes,
        node_count: 0,
        active_connections: 0,
        cache_hit_rate: 0.0,
        last_compaction: None,
    })
}

fn measure_database(db: &Arc<DB>) -> Result<Metrics> {
    let mut total = 0u64;
    let mut index_sizes = HashMap::new();
    for name in crate::all_column_family_names() {
        let handle = cf_handle(db, name)?;
        let bytes = db
            .property_int_value_cf(handle, "rocksdb.total-sst-files-size")
            .ok()
            .flatten()
            .unwrap_or(0);
        total = total.saturating_add(bytes);
        if INDEX_CFS.contains(&name) {
            index_sizes.insert(name.to_string(), bytes);
        }
    }
    Ok(Metrics {
        tenant: None,
        operations_per_sec: 0.0,
        error_rate: 0.0,
        disk_usage_bytes: total,
        index_sizes,
        node_count: 0,
        active_connections: 0,
        cache_hit_rate: 0.0,
        last_compaction: None,
    })
}

/// Usage for `tenant`, or for the whole database when `None`.
pub async fn measure(storage: &RocksDBStorage, tenant: Option<&str>) -> Result<Metrics> {
    let db = storage.db().clone();
    let tenant = tenant.map(str::to_owned);
    tokio::task::spawn_blocking(move || match tenant {
        Some(t) => measure_tenant(&db, &t),
        None => measure_database(&db),
    })
    .await
    .map_err(|e| Error::Internal(format!("metrics task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(db: &DB, cf_name: &str, key: &[u8], len: usize) {
        // Incompressible bytes: SST size must track the logical size.
        let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ key.len() as u64 ^ len as u64;
        let value: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        db.put_cf(cf_handle(db, cf_name).unwrap(), key, value)
            .unwrap();
    }

    fn flush_all(db: &DB) {
        for name in crate::all_column_family_names() {
            db.flush_cf(cf_handle(db, name).unwrap()).unwrap();
        }
    }

    fn storage() -> (tempfile::TempDir, RocksDBStorage) {
        let dir = tempfile::tempdir().unwrap();
        let s = RocksDBStorage::new(dir.path()).unwrap();
        (dir, s)
    }

    #[tokio::test]
    async fn unknown_tenant_is_not_found_not_zero() {
        let (_d, s) = storage();
        let err = measure(&s, Some("ghost")).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn registered_empty_tenant_is_zero_with_index_map() {
        let (_d, s) = storage();
        put(s.db(), cf::REGISTRY, &crate::keys::tenant_key("empty"), 0);
        let m = measure(&s, Some("empty")).await.unwrap();
        assert_eq!(m.tenant.as_deref(), Some("empty"));
        assert!(!m.index_sizes.is_empty());
        // Only the registration marker itself.
        assert!(m.disk_usage_bytes < 64);
    }

    #[tokio::test]
    async fn memtable_only_data_is_not_reported_empty() {
        let (_d, s) = storage();
        put(s.db(), cf::REGISTRY, b"acme\0repos\0site", 1);
        put(s.db(), cf::NODES, b"acme\0site\0main\0ws\0n1", 5000);
        let m = measure(&s, Some("acme")).await.unwrap();
        assert!(m.disk_usage_bytes >= 5000, "got {}", m.disk_usage_bytes);
    }

    #[tokio::test]
    async fn tenants_are_isolated_including_prefix_siblings() {
        let (_d, s) = storage();
        let db = s.db().clone();
        put(&db, cf::REGISTRY, b"acme\0repos\0site", 1);
        put(&db, cf::REGISTRY, b"acme2\0repos\0site", 1);
        put(&db, cf::NODES, b"acme\0site\0main\0ws\0n1", 1000);
        put(&db, cf::NODES, b"acme2\0site\0main\0ws\0n1", 900_000);
        flush_all(&db);
        let a = measure(&s, Some("acme")).await.unwrap();
        let b = measure(&s, Some("acme2")).await.unwrap();
        assert!(a.disk_usage_bytes > 0 && a.disk_usage_bytes < 100_000);
        assert!(b.disk_usage_bytes > 100_000);
    }

    #[tokio::test]
    async fn flushed_sst_size_is_measured() {
        let (_d, s) = storage();
        let db = s.db().clone();
        put(&db, cf::REGISTRY, b"acme\0repos\0site", 1);
        for i in 0..200u32 {
            let key = format!("acme\0site\0main\0ws\0n{i:04}");
            put(&db, cf::NODES, key.as_bytes(), 4096);
        }
        flush_all(&db);
        let m = measure(&s, Some("acme")).await.unwrap();
        assert!(m.disk_usage_bytes > 100_000, "got {}", m.disk_usage_bytes);
    }

    #[tokio::test]
    async fn database_wide_has_no_tenant() {
        let (_d, s) = storage();
        let m = measure(&s, None).await.unwrap();
        assert!(m.tenant.is_none());
    }
}
