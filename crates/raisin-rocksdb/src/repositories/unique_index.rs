//! Unique property constraint index repository implementation
//!
//! This module provides O(1) enforcement of unique property constraints at the storage layer.
//! It works by maintaining a dedicated RocksDB column family (UNIQUE_INDEX) that maps
//! unique property values to the node IDs that own them.
//!
//! Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
//! Value: {node_id}
//!
//! ## Design Rationale
//!
//! - Unique constraints are enforced per-workspace (tenant/repo/branch/workspace scope)
//! - NodeType is included in the key because different NodeTypes might have properties
//!   with the same name but different uniqueness constraints
//! - Revision-aware for MVCC time-travel support and tombstone handling
//! - Bloom filters enabled on the CF for efficient negative lookups

use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// Tombstone marker for deleted unique index entries
const TOMBSTONE: &[u8] = b"T";

/// Check if a value is a tombstone marker
#[inline]
fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE || value.is_empty()
}

/// Manager for unique property constraint enforcement
#[derive(Clone)]
pub struct UniqueIndexManager {
    db: Arc<DB>,
}

impl UniqueIndexManager {
    /// Create a new UniqueIndexManager
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Check if a unique property value conflicts with an existing node
    ///
    /// Performs an O(1) lookup in the UNIQUE_INDEX column family to check if
    /// the given property value is already owned by a different node.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier (unique scope)
    /// * `node_type` - NodeType name (e.g., "raisin:User")
    /// * `property_name` - Property name (e.g., "email")
    /// * `value_hash` - Hash of the property value
    /// * `current_node_id` - Node ID to exclude from conflict check (for updates)
    ///
    /// # Returns
    /// * `Ok(None)` - No conflict, the value is available
    /// * `Ok(Some(node_id))` - Conflict! Returns the node_id that owns this value
    /// * `Err(_)` - Storage error
    pub fn check_unique_conflict(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        property_name: &str,
        value_hash: &str,
        current_node_id: &str,
    ) -> Result<Option<String>> {
        let cf = cf_handle(&self.db, cf::UNIQUE_INDEX)?;

        // Build prefix to scan all revisions of this unique value
        let prefix = keys::unique_index_value_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_type,
            property_name,
            value_hash,
        );

        // Prefix iterator returns newest revision first (due to descending HLC encoding)
        let mut iter = self.db.prefix_iterator_cf(cf, prefix.clone());

        if let Some(item) = iter.next() {
            let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

            // Verify key matches our prefix
            if key.starts_with(&prefix) {
                // If tombstone, the value was deleted - no conflict
                if is_tombstone(&value) {
                    return Ok(None);
                }

                // Extract node_id from value
                let owning_node_id = String::from_utf8_lossy(&value).to_string();

                // Check if it's a different node
                if owning_node_id != current_node_id {
                    return Ok(Some(owning_node_id));
                }

                // Same node owns this value - no conflict (e.g., update without changing unique field)
                return Ok(None);
            }
        }

        // No entry found - value is available
        Ok(None)
    }

    /// Add a unique index entry to a WriteBatch
    ///
    /// This should be called when creating or updating a node with unique properties.
    /// The batch will be committed atomically with the node write.
    ///
    /// # Arguments
    /// * `batch` - WriteBatch to add the entry to
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_type` - NodeType name
    /// * `property_name` - Property name
    /// * `value_hash` - Hash of the property value
    /// * `revision` - HLC revision for MVCC
    /// * `node_id` - Node ID that owns this unique value
    pub fn add_unique_index_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        property_name: &str,
        value_hash: &str,
        revision: &HLC,
        node_id: &str,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::UNIQUE_INDEX)?;

        let key = keys::unique_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_type,
            property_name,
            value_hash,
            revision,
        );

        batch.put_cf(cf, key, node_id.as_bytes());
        Ok(())
    }

    /// Add a tombstone for a unique index entry to a WriteBatch
    ///
    /// This should be called when:
    /// - Deleting a node with unique properties
    /// - Updating a node where a unique property value has changed
    ///
    /// # Arguments
    /// * `batch` - WriteBatch to add the tombstone to
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_type` - NodeType name
    /// * `property_name` - Property name
    /// * `value_hash` - Hash of the property value being released
    /// * `revision` - HLC revision for MVCC
    pub fn add_unique_tombstone_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        property_name: &str,
        value_hash: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::UNIQUE_INDEX)?;

        let key = keys::unique_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_type,
            property_name,
            value_hash,
            revision,
        );

        batch.put_cf(cf, key, TOMBSTONE);
        Ok(())
    }

    /// Get the node ID that owns a unique property value
    ///
    /// Returns the node_id if the value is currently owned, None if available.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_type` - NodeType name
    /// * `property_name` - Property name
    /// * `value_hash` - Hash of the property value
    ///
    /// # Returns
    /// * `Ok(Some(node_id))` - The node_id that owns this value
    /// * `Ok(None)` - Value is not owned (available)
    pub fn get_owner(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        property_name: &str,
        value_hash: &str,
    ) -> Result<Option<String>> {
        let cf = cf_handle(&self.db, cf::UNIQUE_INDEX)?;

        let prefix = keys::unique_index_value_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_type,
            property_name,
            value_hash,
        );

        let mut iter = self.db.prefix_iterator_cf(cf, prefix.clone());

        if let Some(item) = iter.next() {
            let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

            if key.starts_with(&prefix) {
                if is_tombstone(&value) {
                    return Ok(None);
                }

                return Ok(Some(String::from_utf8_lossy(&value).to_string()));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};
    use tempfile::tempdir;

    fn create_test_db() -> Arc<DB> {
        let dir = tempdir().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_names = vec![cf::UNIQUE_INDEX];
        Arc::new(DB::open_cf(&opts, dir.path(), cf_names).unwrap())
    }

    #[test]
    fn test_no_conflict_when_empty() {
        let db = create_test_db();
        let manager = UniqueIndexManager::new(db);

        let result = manager
            .check_unique_conflict(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                "node-123",
            )
            .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_conflict_detection() {
        let db = create_test_db();
        let manager = UniqueIndexManager::new(db.clone());

        // Add a unique index entry
        let mut batch = WriteBatch::default();
        let revision = HLC::now();
        manager
            .add_unique_index_to_batch(
                &mut batch,
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                &revision,
                "node-existing",
            )
            .unwrap();
        db.write(batch).unwrap();

        // Check for conflict with different node
        let result = manager
            .check_unique_conflict(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                "node-new",
            )
            .unwrap();

        assert_eq!(result, Some("node-existing".to_string()));
    }

    #[test]
    fn test_no_conflict_same_node() {
        let db = create_test_db();
        let manager = UniqueIndexManager::new(db.clone());

        // Add a unique index entry
        let mut batch = WriteBatch::default();
        let revision = HLC::now();
        manager
            .add_unique_index_to_batch(
                &mut batch,
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                &revision,
                "node-same",
            )
            .unwrap();
        db.write(batch).unwrap();

        // Check for conflict with same node (update scenario)
        let result = manager
            .check_unique_conflict(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                "node-same",
            )
            .unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_tombstone_releases_value() {
        let db = create_test_db();
        let manager = UniqueIndexManager::new(db.clone());

        // Add a unique index entry
        let mut batch = WriteBatch::default();
        let revision1 = HLC::now();
        manager
            .add_unique_index_to_batch(
                &mut batch,
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                &revision1,
                "node-old",
            )
            .unwrap();
        db.write(batch).unwrap();

        // Add a tombstone (node deleted)
        let mut batch2 = WriteBatch::default();
        let revision2 = HLC::now();
        manager
            .add_unique_tombstone_to_batch(
                &mut batch2,
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                &revision2,
            )
            .unwrap();
        db.write(batch2).unwrap();

        // Value should now be available
        let result = manager
            .check_unique_conflict(
                "tenant1",
                "repo1",
                "main",
                "ws1",
                "raisin:User",
                "email",
                "test@example.com",
                "node-new",
            )
            .unwrap();

        assert!(result.is_none());
    }
}
