//! Shared tombstone writing logic for cascade deletions
//!
//! This module delegates to `crate::tombstones::add_node_tombstones()` for all
//! tombstone writing. This ensures that both repository and transaction delete
//! paths write identical tombstones to all column families.
//!
//! # Single Source of Truth
//!
//! The actual tombstone logic lives in `crate::tombstones`. This module provides
//! a thin wrapper that adapts the repository's ColumnFamily handles to the shared
//! tombstone interface.

use super::super::super::NodeRepositoryImpl;
use crate::tombstones::{add_node_tombstones, TombstoneColumnFamilies, TombstoneContext};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, WriteBatch};

impl NodeRepositoryImpl {
    /// Add all tombstones for a single node to an existing WriteBatch
    ///
    /// This function delegates to `crate::tombstones::add_node_tombstones()` which is
    /// the SINGLE SOURCE OF TRUTH for all deletion tombstones. This ensures repository
    /// and transaction deletes write identical tombstones to all column families.
    ///
    /// # Arguments
    /// * `batch` - The WriteBatch to add tombstones to
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node` - The node to tombstone
    /// * `revision` - The revision to use for tombstone markers
    /// * `cf_nodes` - ColumnFamily handle for NODES (unused, kept for API compatibility)
    /// * `cf_path` - ColumnFamily handle for PATH_INDEX (unused, kept for API compatibility)
    /// * `cf_property` - ColumnFamily handle for PROPERTY_INDEX (unused, kept for API compatibility)
    /// * `cf_relation` - ColumnFamily handle for RELATION_INDEX (unused, kept for API compatibility)
    /// * `cf_ordered` - ColumnFamily handle for ORDERED_CHILDREN (unused, kept for API compatibility)
    /// * `cf_node_path` - ColumnFamily handle for NODE_PATH (unused, kept for API compatibility)
    /// * `cf_compound` - ColumnFamily handle for COMPOUND_INDEX (unused, kept for API compatibility)
    /// * `cf_spatial` - ColumnFamily handle for SPATIAL_INDEX (unused, kept for API compatibility)
    ///
    /// # Tombstones Written
    /// See `crate::tombstones::add_node_tombstones()` for the complete list of
    /// column families that receive tombstones (all 10 CFs).
    #[allow(clippy::too_many_arguments)]
    pub(in super::super::super) fn add_node_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        revision: &HLC,
        _cf_nodes: &ColumnFamily,
        _cf_path: &ColumnFamily,
        _cf_property: &ColumnFamily,
        _cf_relation: &ColumnFamily,
        _cf_ordered: &ColumnFamily,
        _cf_node_path: &ColumnFamily,
        _cf_compound: &ColumnFamily,
        _cf_spatial: &ColumnFamily,
    ) -> Result<()> {
        // Delegate to shared tombstone module - SINGLE SOURCE OF TRUTH
        let ctx = TombstoneContext::new(tenant_id, repo_id, branch, workspace);
        let cfs = TombstoneColumnFamilies::from_db(&self.db)?;

        add_node_tombstones(batch, &self.db, &ctx, &cfs, node, revision)
    }
}
