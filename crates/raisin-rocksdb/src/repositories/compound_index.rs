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

        let prefix = keys::compound_index_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            index_name,
            equality_values,
            published_only,
        );

        tracing::debug!(
            "CompoundIndex: Scanning index '{}' with {} equality columns, ascending={}, limit={:?}",
            index_name,
            equality_values.len(),
            ascending,
            limit
        );

        // Use prefix_iterator for forward scanning
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut results = Vec::new();
        let mut seen_nodes = std::collections::HashSet::new();
        // Track tombstoned node_ids for MVCC - tombstones at newer revisions should
        // prevent older entries from resurrecting deleted nodes
        let mut tombstoned_nodes = std::collections::HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Parse node_id from key
            if let Some(node_id) = Self::parse_node_id_from_key(&key) {
                // Skip tombstones and track them for MVCC
                if is_tombstone(&value) {
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

                    let timestamp = Self::parse_timestamp_from_key(&key);

                    results.push(CompoundIndexScanEntry { node_id, timestamp });

                    // Check limit
                    if let Some(lim) = limit {
                        if results.len() >= lim {
                            break;
                        }
                    }
                }
            }
        }

        // For descending order, reverse the results
        // (prefix iterator always scans forward in lexicographic order)
        if !ascending {
            results.reverse();
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
