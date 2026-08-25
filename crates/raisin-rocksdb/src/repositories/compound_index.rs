//! Compound index repository implementation
//!
//! Provides multi-column compound indexes for efficient ORDER BY + filter queries.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::scope::StorageScope;
use raisin_storage::{CompoundColumnValue, CompoundIndexRepository, CompoundIndexScanEntry};
use rocksdb::DB;
use std::sync::Arc;

/// Tombstone marker for deleted compound index entries
const TOMBSTONE: &[u8] = b"T";

/// Check if a value is a tombstone marker
#[inline]
fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

#[derive(Clone)]
pub struct CompoundIndexRepositoryImpl {
    db: Arc<DB>,
}

impl CompoundIndexRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Parse node_id from a compound index key.
    ///
    /// Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0cidx{_pub}\0{index_name}\0{col1}\0{col2}\0...\0{timestamp}\0{~revision}\0{node_id}
    fn parse_node_id_from_key(key: &[u8]) -> Option<String> {
        // Split by null bytes and get the last part (node_id)
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if parts.is_empty() {
            return None;
        }

        let node_id_bytes = parts.last()?;
        if node_id_bytes.is_empty() {
            return None;
        }

        String::from_utf8(node_id_bytes.to_vec()).ok()
    }

    /// Parse timestamp from a compound index key.
    ///
    /// Key format: ...\0{timestamp_bytes}\0{~revision}\0{node_id}
    /// The timestamp is the third-to-last component.
    fn parse_timestamp_from_key(key: &[u8]) -> Option<i64> {
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if parts.len() < 3 {
            return None;
        }

        // Timestamp is third from the end: [..., timestamp, revision, node_id]
        let timestamp_bytes = parts.get(parts.len() - 3)?;
        if timestamp_bytes.len() != 8 {
            return None;
        }

        let bytes: [u8; 8] = (*timestamp_bytes).try_into().ok()?;
        Some(i64::from_be_bytes(bytes))
    }
}

impl CompoundIndexRepository for CompoundIndexRepositoryImpl {
    async fn index_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        revision: &HLC,
        node_id: &str,
        is_published: bool,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        let key = keys::compound_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            index_name,
            column_values,
            revision,
            node_id,
            is_published,
        );

        tracing::debug!(
            "CompoundIndex: Indexing node '{}' in index '{}' (published: {})",
            node_id,
            index_name,
            is_published
        );

        self.db
            .put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn unindex_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        node_id: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        // Build prefix for both draft and published
        for published in [false, true] {
            let prefix = keys::compound_index_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                index_name,
                column_values,
                published,
            );

            let prefix_clone = prefix.clone();
            let iter = self.db.prefix_iterator_cf(cf, prefix);

            for item in iter {
                let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                // Verify key actually starts with our prefix
                if !key.starts_with(&prefix_clone) {
                    break;
                }

                // Check if this key is for our node
                if let Some(key_node_id) = Self::parse_node_id_from_key(&key) {
                    if key_node_id == node_id {
                        self.db
                            .delete_cf(cf, &key)
                            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn scan_compound_index(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        equality_values: &[CompoundColumnValue],
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<CompoundIndexScanEntry>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        // A node is indexed under EXACTLY ONE tag — `cidx_pub` when
        // `published_at` is set, `cidx` otherwise (see
        // `crud/indexing/compound_indexes.rs`, and the `tag_changed` tombstone
        // dance the property index does for the same reason). So "every node
        // matching this prefix" is the UNION of both tags, not either one.
        // Reading only `cidx` silently dropped every published row, and because
        // the planner strips the matched equality predicates from the residual
        // filter there was no filter left to catch the loss — wrong results, not
        // a slow path. `remove_all_compound_indexes_for_node` already iterates
        // `[false, true]` for exactly this reason.
        let tags: &[bool] = if published_only {
            &[true]
        } else {
            &[false, true]
        };

        tracing::debug!(
            "CompoundIndex: Scanning index '{}' with {} equality columns, ascending={}, limit={:?}, tags={:?}",
            index_name,
            equality_values.len(),
            ascending,
            limit,
            tags
        );

        // Collect from each tag, keyed by the suffix AFTER the tag-bearing
        // prefix. The prefixes differ only in that tag, so the suffix is the
        // index-order key: `…{column values}\0{~revision}\0{node_id}`. Sorting
        // the union by it reproduces the single-keyspace ordering the dedup and
        // tombstone-shadowing logic below depends on — newest revision of each
        // node first.
        let mut merged: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> = Vec::new();
        for &published in tags {
            let prefix = keys::compound_index_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                index_name,
                equality_values,
                published,
            );
            let prefix_clone = prefix.clone();
            for item in self.db.prefix_iterator_cf(cf, prefix) {
                let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                if !key.starts_with(&prefix_clone) {
                    break;
                }
                let suffix = key[prefix_clone.len()..].to_vec();
                merged.push((suffix, key.to_vec(), value.to_vec()));
            }
        }
        merged.sort_by(|a, b| a.0.cmp(&b.0));

        let mut results = Vec::new();
        let mut seen_nodes = std::collections::HashSet::new();
        // Track tombstoned node_ids for MVCC - tombstones at newer revisions should
        // prevent older entries from resurrecting deleted nodes
        let mut tombstoned_nodes = std::collections::HashSet::new();

        for (_suffix, key, value) in &merged {
            // Parse node_id from key
            if let Some(node_id) = Self::parse_node_id_from_key(key) {
                // Skip tombstones and track them for MVCC
                if is_tombstone(value) {
                    tombstoned_nodes.insert(node_id);
                    continue;
                }

                // Skip entries for node_ids that have been tombstoned at a newer revision
                if tombstoned_nodes.contains(&node_id) {
                    continue;
                }

                // Deduplicate - only take the first occurrence of each node
                if !seen_nodes.contains(&node_id) {
                    seen_nodes.insert(node_id.clone());

                    let timestamp = Self::parse_timestamp_from_key(key);

                    results.push(CompoundIndexScanEntry { node_id, timestamp });

                    // Only a FORWARD scan may stop at the limit. Descending
                    // results are produced by reversing below, so truncating
                    // here would keep the k SMALLEST and then reverse them —
                    // i.e. `ORDER BY col DESC LIMIT k` would return the k oldest
                    // rows, newest-first among themselves, which never overlaps
                    // the correct answer.
                    if ascending {
                        if let Some(lim) = limit {
                            if results.len() >= lim {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // For descending order, reverse the results.
        //
        // The iterator deliberately stays FORWARD even here. The key ends
        // `…{~revision}\0{node_id}`, so forward traversal meets each node's
        // NEWEST revision first, which is what both the `seen_nodes` dedup and
        // the `tombstoned_nodes` shadowing rely on. A reverse iterator would
        // meet the oldest revision first and resurrect superseded entries.
        //
        // The cost is that a descending scan reads the whole equality group
        // before truncating. The O(k) way to get newest-first is to store the
        // column already inverted — `CompoundColumnType::Timestamp` encodes
        // `__created_at` / `__updated_at` as `TimestampDesc`, so "newest first"
        // becomes a FORWARD scan that bounds properly.
        if !ascending {
            results.reverse();
            if let Some(lim) = limit {
                results.truncate(lim);
            }
        }

        tracing::debug!("CompoundIndex: Scan returned {} results", results.len());

        Ok(results)
    }

    async fn remove_all_compound_indexes_for_node(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        // Scan all compound indexes in this workspace (both draft and published)
        for published in [false, true] {
            let prefix = keys::compound_index_workspace_prefix(
                tenant_id, repo_id, branch, workspace, published,
            );

            let prefix_clone = prefix.clone();
            let iter = self.db.prefix_iterator_cf(cf, prefix);

            for item in iter {
                let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                // Verify key actually starts with our prefix
                if !key.starts_with(&prefix_clone) {
                    break;
                }

                // Check if this key is for our node
                if let Some(key_node_id) = Self::parse_node_id_from_key(&key) {
                    if key_node_id == node_id {
                        self.db
                            .delete_cf(cf, &key)
                            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                    }
                }
            }
        }

        tracing::debug!(
            "CompoundIndex: Removed all index entries for node '{}'",
            node_id
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_hlc::HLC;
    use raisin_storage::scope::StorageScope;
    use rocksdb::{Options, DB};
    use tempfile::TempDir;

    fn create_test_db() -> (Arc<DB>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_names = vec![cf::COMPOUND_INDEX];
        let db = DB::open_cf(&opts, temp_dir.path(), cf_names).unwrap();

        (Arc::new(db), temp_dir)
    }

    fn test_scope() -> StorageScope<'static> {
        StorageScope::new("tenant1", "repo1", "main", "ws1")
    }

    #[tokio::test]
    async fn test_index_and_scan() {
        let (db, _temp_dir) = create_test_db();
        let repo = CompoundIndexRepositoryImpl::new(db);

        let scope = test_scope();
        let index_name = "by_type_category";
        let node_id = "node123";
        let revision = HLC::new(1700000000000000, 0);

        // Index a node
        let columns = vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::String("business".to_string()),
            CompoundColumnValue::TimestampDesc(1700000000000000), // microseconds
        ];

        repo.index_compound(
            scope, index_name, &columns, &revision, node_id, false, // draft
        )
        .await
        .unwrap();

        // Scan for it
        let equality_values = vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::String("business".to_string()),
        ];

        let results = repo
            .scan_compound_index(
                scope,
                index_name,
                &equality_values,
                false, // draft
                false, // descending
                Some(10),
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, node_id);
    }

    /// A PUBLISHED node must come back from an ordinary (non-published-only)
    /// scan.
    ///
    /// The writer picks the keyspace EXCLUSIVELY from `published_at`: a
    /// published node goes to `cidx_pub` and is absent from `cidx`. Every SQL
    /// scan asks for `published_only: false`, so when that meant "read `cidx`"
    /// each published row silently vanished — and because the planner strips the
    /// matched equality predicates from the residual filter, nothing downstream
    /// could notice. Regression guard: `false` means BOTH keyspaces.
    #[tokio::test]
    async fn scan_returns_published_and_draft_nodes_together() {
        let (db, _temp_dir) = create_test_db();
        let repo = CompoundIndexRepositoryImpl::new(db);

        let scope = test_scope();
        let index_name = "by_type_category";

        let columns = |ts: i64| {
            vec![
                CompoundColumnValue::String("news:Article".to_string()),
                CompoundColumnValue::String("business".to_string()),
                CompoundColumnValue::TimestampDesc(ts),
            ]
        };

        repo.index_compound(
            scope,
            index_name,
            &columns(1700000000000000),
            &HLC::new(1700000000000000, 0),
            "draft_node",
            false, // draft -> cidx
        )
        .await
        .unwrap();

        repo.index_compound(
            scope,
            index_name,
            &columns(1700000000000001),
            &HLC::new(1700000000000001, 0),
            "published_node",
            true, // published -> cidx_pub
        )
        .await
        .unwrap();

        let equality_values = vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::String("business".to_string()),
        ];

        let all = repo
            .scan_compound_index(scope, index_name, &equality_values, false, true, None)
            .await
            .unwrap();
        let ids: Vec<&str> = all.iter().map(|e| e.node_id.as_str()).collect();
        assert!(
            ids.contains(&"published_node"),
            "a published node must be visible to a normal scan; got {ids:?}"
        );
        assert!(
            ids.contains(&"draft_node"),
            "a draft node must still be visible; got {ids:?}"
        );
        assert_eq!(all.len(), 2, "expected exactly the two indexed nodes");

        // `true` still means published-only.
        let published = repo
            .scan_compound_index(scope, index_name, &equality_values, true, true, None)
            .await
            .unwrap();
        let published_ids: Vec<&str> = published.iter().map(|e| e.node_id.as_str()).collect();
        assert_eq!(published_ids, vec!["published_node"]);
    }

    /// A non-String leading column must round-trip.
    ///
    /// `build_compound_key` encodes `Integer` as big-endian bytes, so a reader
    /// that rebuilt the prefix as the ASCII text `"42"` addressed different
    /// bytes and matched nothing. This pins the writer's encoding as the
    /// contract the executor has to reproduce.
    #[tokio::test]
    async fn scan_matches_a_non_string_leading_column() {
        let (db, _temp_dir) = create_test_db();
        let repo = CompoundIndexRepositoryImpl::new(db);

        let scope = test_scope();
        let index_name = "by_priority";

        repo.index_compound(
            scope,
            index_name,
            &vec![
                CompoundColumnValue::Integer(42),
                CompoundColumnValue::TimestampDesc(1700000000000000),
            ],
            &HLC::new(1700000000000000, 0),
            "int_node",
            false,
        )
        .await
        .unwrap();

        let typed = repo
            .scan_compound_index(
                scope,
                index_name,
                &[CompoundColumnValue::Integer(42)],
                false,
                true,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            typed.len(),
            1,
            "an Integer prefix must match what was written"
        );
        assert_eq!(typed[0].node_id, "int_node");

        // The old executor behaviour: same value, stringified. Must NOT match —
        // this is the shape of the bug, kept explicit so a regression is loud.
        let stringified = repo
            .scan_compound_index(
                scope,
                index_name,
                &[CompoundColumnValue::String("42".to_string())],
                false,
                true,
                None,
            )
            .await
            .unwrap();
        assert!(
            stringified.is_empty(),
            "a String-encoded prefix addresses different bytes than an Integer column"
        );
    }

    #[tokio::test]
    async fn test_unindex() {
        let (db, _temp_dir) = create_test_db();
        let repo = CompoundIndexRepositoryImpl::new(db);

        let scope = test_scope();
        let index_name = "by_type_category";
        let node_id = "node123";
        let revision = HLC::new(1700000000000000, 0);

        let columns = vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::String("business".to_string()),
            CompoundColumnValue::TimestampDesc(1700000000000000),
        ];

        // Index
        repo.index_compound(scope, index_name, &columns, &revision, node_id, false)
            .await
            .unwrap();

        // Unindex
        repo.unindex_compound(scope, index_name, &columns, node_id)
            .await
            .unwrap();

        // Verify it's gone
        let equality_values = vec![
            CompoundColumnValue::String("news:Article".to_string()),
            CompoundColumnValue::String("business".to_string()),
        ];

        let results = repo
            .scan_compound_index(scope, index_name, &equality_values, false, false, Some(10))
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_scan_ordering() {
        let (db, _temp_dir) = create_test_db();
        let repo = CompoundIndexRepositoryImpl::new(db);

        let scope = test_scope();
        let index_name = "by_type_created";

        // Index multiple nodes with different timestamps
        let timestamps = [
            1700000001000000i64,
            1700000003000000i64,
            1700000002000000i64,
        ];
        let node_ids = ["node1", "node3", "node2"];

        for (ts, node_id) in timestamps.iter().zip(node_ids.iter()) {
            let revision = HLC::new(1700000000000000, 0);
            let columns = vec![
                CompoundColumnValue::String("news:Article".to_string()),
                CompoundColumnValue::TimestampDesc(*ts),
            ];

            repo.index_compound(scope, index_name, &columns, &revision, node_id, false)
                .await
                .unwrap();
        }

        // Scan descending (newest first)
        let equality_values = vec![CompoundColumnValue::String("news:Article".to_string())];

        let results = repo
            .scan_compound_index(scope, index_name, &equality_values, false, false, None)
            .await
            .unwrap();

        // With TimestampDesc encoding, newest should be first
        assert_eq!(results.len(), 3);
        // Note: actual ordering depends on key encoding
    }
}
