//! Compaction helper functions.
//!
//! Contains utility functions for size estimation, repository enumeration,
//! and RocksDB-level compaction operations.

use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;

/// Compact a key range across EVERY column family.
///
/// `DB::compact_range` operates on the **default** column family only, and this
/// database keeps no data there — so calling it directly compacts nothing at
/// all. Every compaction entry point must fan out over
/// [`crate::all_column_families`] with `compact_range_cf`, which is what this
/// helper exists to guarantee.
///
/// `None`/`None` compacts the full key space of each CF.
pub(super) fn compact_all_column_families(
    storage: &RocksDBStorage,
    start: Option<&[u8]>,
    end: Option<&[u8]>,
) {
    for cf_name in crate::all_column_families() {
        match cf_handle(storage.db(), cf_name) {
            Ok(cf) => storage.db().compact_range_cf(cf, start, end),
            Err(e) => {
                tracing::warn!(cf = cf_name, error = %e, "Skipping compaction for column family")
            }
        }
    }
}

/// Get approximate size of a repository
pub fn get_repository_size(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<u64> {
    // RocksDB doesn't provide easy per-key-range size queries
    // We approximate by counting keys and average value sizes
    let mut total_size = 0u64;

    let prefix = keys::repo_prefix(tenant_id, repo_id);

    for cf_name in crate::all_column_families() {
        if let Ok(cf) = cf_handle(storage.db(), cf_name) {
            let iter = crate::prefix_scan(storage.db(), cf, &prefix);

            for (key, value) in iter.flatten() {
                total_size += key.len() as u64 + value.len() as u64;
            }
        }
    }

    Ok(total_size)
}

/// Get total database size across all column families
pub fn get_total_db_size(storage: &RocksDBStorage) -> Result<u64> {
    let mut total_size = 0u64;

    for cf_name in crate::all_column_families() {
        if let Ok(cf) = cf_handle(storage.db(), cf_name) {
            let iter = storage.db().iterator_cf(cf, rocksdb::IteratorMode::Start);

            for (key, value) in iter.flatten() {
                total_size += key.len() as u64 + value.len() as u64;
            }
        }
    }

    Ok(total_size)
}

/// List all repositories for a tenant
pub(super) async fn list_repositories(
    storage: &RocksDBStorage,
    tenant_id: &str,
) -> Result<Vec<String>> {
    let cf_registry = cf_handle(storage.db(), cf::REGISTRY)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push("repos")
        .build_prefix();

    let mut repos = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_registry, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // `prefix_iterator_cf` is bounded by the CF's prefix extractor, not the
        // seek key, so it walks past `{tenant}\0repos\0` — and the shape test
        // below then reports the over-read rows as phantom repositories. See
        // `crate::management::helpers::list_repositories_for_tenant`.
        if !key.starts_with(&prefix) {
            break;
        }

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        // Exactly `{tenant}\0repos\0{repo}` (`keys::repository_key`).
        if parts.len() == 3 {
            repos.push(parts[2].to_string());
        }
    }

    Ok(repos)
}
