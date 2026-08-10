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

/// Exclusive upper bound for a prefix scan: the prefix with its last non-`0xFF`
/// byte incremented. `None` when the prefix is all `0xFF` (unbounded above).
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last != 0xFF {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

/// Run RocksDB compaction for a specific repository key range
pub(super) fn run_rocksdb_compaction(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<()> {
    let prefix = keys::repo_prefix(tenant_id, repo_id);
    // The old call passed the same bound as start AND end, which is an empty
    // range — compaction had nothing to do even on the CF it did reach.
    let end = prefix_upper_bound(&prefix);

    compact_all_column_families(storage, Some(&prefix), end.as_deref());

    Ok(())
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
            let iter = storage.db().prefix_iterator_cf(cf, &prefix);

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

/// List all unique node IDs in a repository
pub(super) async fn list_all_node_ids(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<String>> {
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;
    let prefix = keys::repo_prefix(tenant_id, repo_id);

    let mut node_ids = std::collections::HashSet::new();
    let iter = storage.db().prefix_iterator_cf(cf_nodes, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        if !key_str.contains("\0nodes\0") {
            continue;
        }

        // Extract node ID from key: ...nodes\0{node_id}\0{revision}
        let parts: Vec<&str> = key_str.split('\0').collect();
        if let Some(idx) = parts.iter().position(|&p| p == "nodes") {
            if idx + 1 < parts.len() {
                node_ids.insert(parts[idx + 1].to_string());
            }
        }
    }

    Ok(node_ids.into_iter().collect())
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
    let iter = storage.db().prefix_iterator_cf(cf_registry, &prefix);

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
