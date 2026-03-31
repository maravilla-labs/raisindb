// TODO(v0.2): Tombstone utilities for deletion
#![allow(dead_code)]

//! Centralized node deletion tombstone logic - SINGLE SOURCE OF TRUTH
//!
//! This module provides a single source of truth for all tombstones that must be
//! written when deleting a node. Both repository and transaction delete paths
//! MUST use this module to ensure consistent deletion behavior.
//!
//! # Column Families Requiring Tombstones
//!
//! When a node is deleted, tombstones must be written to these column families:
//!
//! 1. **NODES** - Node data itself
//! 2. **PATH_INDEX** - Path -> node_id mapping
//! 3. **NODE_PATH** - Node_id -> path reverse mapping
//! 4. **PROPERTY_INDEX** - Property indexes (custom + system properties)
//! 5. **REFERENCE_INDEX** - Forward and reverse reference indexes
//! 6. **RELATION_INDEX** - Forward and reverse relation indexes
//! 7. **ORDERED_CHILDREN** - Child ordering entries
//! 8. **COMPOUND_INDEX** - Multi-column compound indexes
//! 9. **SPATIAL_INDEX** - Geohash-based spatial indexes
//! 10. **TRANSLATION_DATA** - Locale overlay data

mod core_tombstones;
pub mod helpers;
mod index_tombstones;

#[cfg(test)]
mod tests;

use crate::cf;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, WriteBatch, DB};
use std::sync::Arc;

pub use helpers::{extract_references, ExtractedReference};

/// Tombstone marker (single byte 'T' for debugging visibility)
pub const TOMBSTONE: &[u8] = b"T";

/// All column families requiring tombstones during node deletion
pub const DELETION_COLUMN_FAMILIES: &[&str] = &[
    cf::NODES,
    cf::PATH_INDEX,
    cf::NODE_PATH,
    cf::PROPERTY_INDEX,
    cf::REFERENCE_INDEX,
    cf::RELATION_INDEX,
    cf::ORDERED_CHILDREN,
    cf::COMPOUND_INDEX,
    cf::SPATIAL_INDEX,
    cf::TRANSLATION_DATA,
];

/// Context for tombstone operations
#[derive(Debug, Clone)]
pub struct TombstoneContext<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

impl<'a> TombstoneContext<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str, branch: &'a str, workspace: &'a str) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace,
        }
    }
}

/// Column family handles for tombstone operations
pub struct TombstoneColumnFamilies<'a> {
    pub nodes: &'a ColumnFamily,
    pub path_index: &'a ColumnFamily,
    pub node_path: &'a ColumnFamily,
    pub property_index: &'a ColumnFamily,
    pub reference_index: &'a ColumnFamily,
    pub relation_index: &'a ColumnFamily,
    pub ordered_children: &'a ColumnFamily,
    pub compound_index: &'a ColumnFamily,
    pub spatial_index: &'a ColumnFamily,
    pub translation_data: &'a ColumnFamily,
}

impl<'a> TombstoneColumnFamilies<'a> {
    /// Get all column family handles from a database
    pub fn from_db(db: &'a DB) -> Result<Self> {
        use crate::cf_handle;
        Ok(Self {
            nodes: cf_handle(db, cf::NODES)?,
            path_index: cf_handle(db, cf::PATH_INDEX)?,
            node_path: cf_handle(db, cf::NODE_PATH)?,
            property_index: cf_handle(db, cf::PROPERTY_INDEX)?,
            reference_index: cf_handle(db, cf::REFERENCE_INDEX)?,
            relation_index: cf_handle(db, cf::RELATION_INDEX)?,
            ordered_children: cf_handle(db, cf::ORDERED_CHILDREN)?,
            compound_index: cf_handle(db, cf::COMPOUND_INDEX)?,
            spatial_index: cf_handle(db, cf::SPATIAL_INDEX)?,
            translation_data: cf_handle(db, cf::TRANSLATION_DATA)?,
        })
    }

    /// Get all column family handles from an Arc<DB>
    pub fn from_arc_db(db: &'a Arc<DB>) -> Result<Self> {
        Self::from_db(db.as_ref())
    }
}

/// Add ALL required tombstones for a node deletion to a WriteBatch
///
/// This is the SINGLE SOURCE OF TRUTH for node deletion tombstones.
/// All code paths (repository, transaction, cascade) MUST use this function.
///
/// # Arguments
///
/// * `batch` - WriteBatch to add tombstones to
/// * `db` - Database reference for prefix scans (compound/spatial indexes)
/// * `ctx` - Context (tenant, repo, branch, workspace)
/// * `cfs` - Column family handles
/// * `node` - The node being deleted
/// * `revision` - Revision for tombstone markers
pub fn add_node_tombstones(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let is_published = node.published_at.is_some();

    // 1. NODES - Tombstone node data
    core_tombstones::tombstone_node_data(batch, ctx, cfs, node, revision);

    // 2. PATH_INDEX - Tombstone path index
    core_tombstones::tombstone_path_index(batch, ctx, cfs, node, revision);

    // 3. NODE_PATH - Tombstone node-to-path reverse index
    core_tombstones::tombstone_node_path(batch, ctx, cfs, node, revision);

    // 4. PROPERTY_INDEX - Tombstone all property indexes (custom + system)
    index_tombstones::tombstone_property_indexes(batch, ctx, cfs, node, revision, is_published);

    // 5. REFERENCE_INDEX - Tombstone forward and reverse references
    index_tombstones::tombstone_reference_indexes(batch, ctx, cfs, node, revision, is_published);

    // 6. RELATION_INDEX - Tombstone forward and reverse relations
    // NOTE: Must scan RELATION_INDEX because node.relations is always empty on read!
    index_tombstones::tombstone_relation_indexes(batch, db, ctx, cfs, node, revision)?;

    // 7. ORDERED_CHILDREN - Tombstone child ordering entry
    core_tombstones::tombstone_ordered_children(batch, ctx, cfs, node, revision);

    // 8. COMPOUND_INDEX - Tombstone compound index entries (prefix scan)
    index_tombstones::tombstone_compound_indexes(batch, db, ctx, cfs, node)?;

    // 9. SPATIAL_INDEX - Tombstone spatial index entries (prefix scan)
    index_tombstones::tombstone_spatial_indexes(batch, db, ctx, cfs, node)?;

    // 10. TRANSLATION_DATA - Tombstone translation data (prefix scan)
    index_tombstones::tombstone_translation_data(batch, db, ctx, cfs, node, revision)?;

    Ok(())
}
