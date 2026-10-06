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
//! 10. **SECRETS** - Vaulted `encrypted` field values
//! 11. **LOCALIZED_NAME_INDEX** - Localized URL segments (plan Phase 12)
//! 12. **BLOCK_TRANSLATIONS** - `T` at the delete for every block overlay
//!     live there (plan Phase 11c; hygiene, see below)
//! 13. **NODE_DELETES** - the delete as a key, so the translation read rule
//!     asks one seek instead of walking the node's history
//!     (`crate::node_delete_index`)
//!
//! A node delete ends its overlays — node AND block — by a READ rule
//! (`translation_read::ended_by_node_delete`); that is what makes them absent
//! in any arrival order. Not TRANSLATION_DATA: deriving node-overlay
//! tombstones at write time raced with translation writes, and their
//! `TRANSLATION_INDEX` / localized-name side effects are the read rule's
//! business. Block overlays have neither, and their `T` is materialized here
//! so retention GC can reclaim what the delete ended
//! (`translation_write::block_deletion`).

mod core_tombstones;
pub mod helpers;
mod index_tombstones;
mod secret_tombstones;

#[cfg(test)]
mod tests;

use crate::cf;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, WriteBatch, DB};
use std::sync::Arc;

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
    cf::SECRETS,
    cf::LOCALIZED_NAME_INDEX,
    cf::BLOCK_TRANSLATIONS,
    cf::NODE_DELETES,
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
    pub secrets: &'a ColumnFamily,
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
            secrets: cf_handle(db, cf::SECRETS)?,
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
///
/// The ORDERED_CHILDREN parent is resolved from PATH_INDEX; a caller that
/// already knows the parent's id uses [`add_node_tombstones_with_parent`].
pub fn add_node_tombstones(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    add_node_tombstones_with_parent(batch, db, ctx, cfs, node, revision, None)
}

/// [`add_node_tombstones`] with the ORDERED_CHILDREN parent key given:
/// `parent_index_id` is the parent node's ID (`/` for a root child), never its
/// name. The replicated delete passes the id its op carries, unconditionally.
pub fn add_node_tombstones_with_parent(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
    parent_index_id: Option<&str>,
) -> Result<()> {
    let is_published = node.published_at.is_some();

    // 1. NODES - Tombstone node data
    core_tombstones::tombstone_node_data(batch, ctx, cfs, node, revision);

    // 1b. NODE_DELETES - the delete as a key, in the SAME batch as the
    //     tombstone: on a `Ready` branch every tombstone must have its entry
    //     (`crate::node_delete_index`), and this funnel is every delete path
    //     — transaction, repository, cascade, cross-branch prune, merge, and
    //     the replication apply (derived: written locally for remote deletes
    //     too).
    crate::node_delete_index::stage_delete(
        batch,
        db,
        (ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace),
        &node.id,
        revision,
    )?;

    // 2. PATH_INDEX - Tombstone path index (never over another node's entry)
    core_tombstones::tombstone_path_index(batch, db, ctx, cfs, node, revision)?;

    // 3. NODE_PATH - Tombstone node-to-path reverse index
    core_tombstones::tombstone_node_path(batch, ctx, cfs, node, revision);

    // 4. PROPERTY_INDEX - Tombstone all property indexes (custom + system)
    index_tombstones::tombstone_property_indexes(batch, db, ctx, cfs, node, revision)?;

    // 5. REFERENCE_INDEX - Tombstone forward and reverse references
    index_tombstones::tombstone_reference_indexes(batch, ctx, cfs, node, revision, is_published);

    // 6. RELATION_INDEX - Tombstone forward and reverse relations
    // NOTE: Must scan RELATION_INDEX because node.relations is always empty on read!
    index_tombstones::tombstone_relation_indexes(batch, db, ctx, cfs, node, revision)?;

    // 7. ORDERED_CHILDREN - Tombstone child ordering entry
    core_tombstones::tombstone_ordered_children(
        batch,
        db,
        ctx,
        cfs,
        node,
        revision,
        parent_index_id,
    )?;

    // 8. COMPOUND_INDEX - derived from the node and its cached definitions
    //    (a workspace scan only when they are cold); tombstones at the delete
    //    revision, never written over the live key (plan Phase 8).
    crate::indexing::compound::tombstone_compound_for_delete(
        batch,
        db,
        &crate::indexing::IndexCtx::new(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace),
        node,
        revision,
    )?;

    // 9. SPATIAL_INDEX - Tombstone spatial index entries (derived from the node's
    //    own geometry; no scan)
    index_tombstones::tombstone_spatial_indexes(batch, db, ctx, cfs, node, revision)?;

    // 10. SECRETS - Retire every vaulted `encrypted` field value the node owns.
    //     Found by an EXACT key prefix (`node/{node_id}/`), not by walking the
    //     node's properties: a field cleared on an earlier revision left a
    //     secret the current properties no longer mention, and that one must be
    //     retired too. Prior versions survive — see `secret_tombstones`.
    secret_tombstones::tombstone_secrets(batch, db, ctx, cfs, node, revision)?;

    // 11. REGISTRY - Drop the virtual-mount registry entry (no-op for every
    //     other node type).
    //
    //     A real delete rather than a tombstone: the registry is a live derived
    //     set with no revision dimension, and the point of it is that the scan
    //     shrinks when a mount goes away. This is the ONE delete point, which is
    //     why it belongs here rather than in each caller — the transaction,
    //     repository and cascade delete paths all funnel through this function.
    crate::vmount_registry::record_node_delete(
        batch,
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        node,
    )?;

    // 12. LOCALIZED_NAME_INDEX - the node's live claims and reverse rows as of
    //     the delete (plan Phase 12). Hygiene: a lookup also checks the node
    //     is live, so a delete that arrives without this is never served.
    crate::localized_name::sync::stage_node_deleted(
        db,
        batch,
        crate::localized_name::keys::NameScope::new(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
        ),
        &node.id,
        revision,
    )?;

    // 13. BLOCK_TRANSLATIONS - `T` at the delete for every block overlay
    //     whose newest stored version at or before it is live (plan Phase
    //     11c). Hygiene: the read rule already ends them, so a version this
    //     read misses (committed below the delete meanwhile) is still absent,
    //     and the one writer or the `block_overlay_tombstones` repair stores
    //     its `T` later.
    //
    //     A `T` at the delete's revision is only right while the node STAYS
    //     deleted there: a live `NODES` record written later at the same
    //     revision would overwrite the tombstone and leave this `T` ending the
    //     overlays of a node that was never deleted. No path writes one — a
    //     transaction cannot recreate an id it deleted (`validate_create`
    //     checks the id against committed state; test
    //     `a_transaction_cannot_recreate_a_node_it_deleted`).
    crate::translation_write::materialize_block_deletion(
        db,
        batch,
        (ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace),
        &node.id,
        revision,
    )?;

    Ok(())
}
