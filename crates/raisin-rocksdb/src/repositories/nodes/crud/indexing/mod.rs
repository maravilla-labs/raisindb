//! Shared indexing operations for nodes
//!
//! This module contains reusable functions for building index entries that are shared
//! between create and update operations, following DRY principles.

mod compound_indexes;
pub(crate) mod in_place_guard;
pub(crate) mod node_record;
mod property_write;
pub(crate) mod reference_indexes;
mod relation_indexes;
pub(crate) mod unique_delta;
pub(crate) mod unique_guard;
pub(crate) mod unique_indexes;

use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
pub(crate) use property_write::PropertyWrite;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add all index entries for a node to a WriteBatch
    ///
    /// This is the main entry point for indexing a node. It:
    /// - Stores the node blob (as StorageNode - path is NOT in the blob)
    /// - Adds path index (path -> node_id)
    /// - Adds node_path index (node_id -> path) for O(1) path materialization
    /// - Adds all property indexes (regular + system properties)
    /// - Adds reference indexes (forward + reverse)
    /// - Adds relation indexes (forward + reverse)
    ///
    /// Note: This does NOT add ORDERED_CHILDREN index - use add_ordered_children_to_batch for that
    ///
    /// # StorageNode Optimization
    ///
    /// The node blob is stored as `StorageNode` which excludes the `path` field.
    /// This enables O(1) move operations since only the root node blob needs updating,
    /// while descendant blobs remain unchanged (only path indexes are updated).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_node_indexes_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        write: PropertyWrite<'_>,
    ) -> Result<()> {
        self.add_node_indexes_to_batch_with_parent_id(
            batch, node, tenant_id, repo_id, branch, workspace, revision, None, write,
        )
    }

    /// Rewrite an EXISTING node's record at `revision`: index `new` through
    /// [`Self::add_node_indexes_to_batch_with_parent_id`] with a FULL
    /// property-index put against `old` (every value `old` carried that `new`
    /// no longer does is tombstoned).
    ///
    /// The writers that re-stamp a node record outside `update_impl` — reorder,
    /// rebalance and tree move all set a fresh `updated_at`, and a move can
    /// change `name` — used to write only the NEW entries, leaving the old
    /// `__updated_at` / `__name` entry live beside them. The timestamp reader
    /// dedupes per node, so it met the stale entry first, rejected it on the
    /// residual re-check, and then skipped the live one: `ORDER BY updated_at`
    /// silently dropped every reordered node.
    ///
    /// The COMPOUND entries follow too (plan Phase 8): a re-stamp changes
    /// `updated_at`, and a compound column over `__updated_at` kept the old
    /// position live beside nothing at the new one.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn rewrite_node_record_to_batch(
        &self,
        batch: &mut WriteBatch,
        old: &Node,
        new: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        parent_id: Option<String>,
    ) -> Result<()> {
        // A re-stamp is a full put against the record it replaces.
        self.add_node_indexes_to_batch_with_parent_id(
            batch,
            new,
            tenant_id,
            repo_id,
            branch,
            workspace,
            revision,
            parent_id,
            PropertyWrite::full(Some(old)),
        )?;
        self.add_compound_delta_to_batch(
            batch,
            &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
            crate::indexing::Baseline::Full(Some(old)),
            new,
            revision,
        )
        .await
    }

    /// Add all index entries for a node with an optional parent_id
    ///
    /// This variant allows passing a known parent_id to avoid lookups.
    /// The parent_id is stored in the StorageNode blob for efficient parent resolution.
    pub(crate) fn add_node_indexes_to_batch_with_parent_id(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
        parent_id: Option<String>,
        write: PropertyWrite<'_>,
    ) -> Result<()> {
        // 1 + 3. The node record — StorageNode blob (no path) and its
        // NODE_PATH entry — through the ONE record writer.
        node_record::write_node_record(
            &self.db, batch, tenant_id, repo_id, branch, workspace, node, parent_id, revision,
        )?;

        // 2. Index by path with versioned key (path -> node_id)
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, &node.path, revision,
        );
        batch.put_cf(cf_path, path_key, node.id.as_bytes());

        // 4 + 5. Property and pseudo-property entries, IS_A / HAS_MIXIN
        // membership included, through the ONE writer (`property_delta`).
        self.write_property_entries(
            batch, node, tenant_id, repo_id, branch, workspace, revision, write,
        )?;

        // 6. Add reference indexes (every reference, never skipped)
        self.add_reference_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 7. Add relation indexes
        self.add_relation_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 8. Add spatial indexes.
        //
        // This path (`storage.nodes().add(...)`) wrote node blob, path, node_path,
        // property, system-property, reference and relation indexes and NO spatial
        // index — so any caller of the repository API produced geometry that was
        // invisible to `ST_DWITHIN`, with no error. Delegates to the ONE shared
        // spatial writer, which is also what the transaction and replication paths
        // call.
        self.add_spatial_indexes(batch, node, tenant_id, repo_id, branch, workspace, revision)?;

        // 9. Virtual-mount registry.
        //
        // Same reasoning as the spatial writer above, and the same batch: this
        // repository path is a second node writer, so a registry entry written
        // only on the transaction path would be missing for every mount created
        // through `storage.nodes().add(...)`/`update(...)` — and a missing entry
        // is silent, the mount simply never syncs. See `crate::vmount_registry`.
        crate::vmount_registry::record_node_write(
            batch, &self.db, tenant_id, repo_id, branch, node,
        )?;

        Ok(())
    }

    /// Stage spatial index entries for every geometry-valued property of `node`.
    ///
    /// The policy is read from the LOCAL index-state records, which cache the
    /// resolved configuration per property — this path is synchronous and cannot
    /// perform the async schema load the transaction path does.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_spatial_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        // The geometry PATHS of the whole property tree, not the top level.
        //
        // Both the early exit and the state-record loop below range over this one
        // set. A flat `node.properties` scan here skipped nodes whose ONLY
        // geometry is nested (indexing them not at all) and, for nodes that also
        // had a top-level one, wrote entries for the nested paths while creating a
        // state record for none of them — which reads as `NotBuilt` and pins every
        // nested query to a full scan forever.
        let geometry_paths = crate::indexing::indexed_geometry_paths(&node.properties);
        if geometry_paths.is_empty() {
            return Ok(());
        }

        let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
        let spatial_state = crate::spatial_state::SpatialStateStore::new(self.db.clone());
        let policies =
            crate::indexing::NodeSpatialPolicies::from_local_state(&spatial_state, &ctx, node);
        let targets = crate::indexing::SpatialIndexTargets::from_db(self.db.as_ref())?;

        crate::indexing::write_node_spatial_indexes(
            batch, &targets, &ctx, node, revision, &policies,
        )?;

        for property_path in &geometry_paths {
            spatial_state.ensure_for_write(
                batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_path,
                policies.for_property(property_path),
                *revision,
            )?;
        }

        Ok(())
    }

    /// Tombstone the spatial entries `old_node` holds that `new_node` supersedes.
    ///
    /// `new_node == None` means the node is being deleted. An unchanged geometry is
    /// left alone: the re-write reproduces byte-identical keys and values, so
    /// tombstoning it would be pure MVCC churn on every update to any other property.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_spatial_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        old_node: &Node,
        new_node: Option<&Node>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        // Nested geometry counts here too: a flat guard let a node whose only
        // geometry sits in a section keep matching its old position forever,
        // because the tombstoner was never reached.
        if crate::indexing::walk_geometries(&old_node.properties).is_empty() {
            return Ok(());
        }

        let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
        let spatial_state = crate::spatial_state::SpatialStateStore::new(self.db.clone());
        let policies =
            crate::indexing::NodeSpatialPolicies::from_local_state(&spatial_state, &ctx, old_node);
        let targets = crate::indexing::SpatialIndexTargets::from_db(self.db.as_ref())?;

        crate::indexing::tombstone_superseded_spatial_indexes(
            batch, &targets, &ctx, old_node, new_node, revision, &policies,
        )
    }
}
