//! Parent-id resolution and stale-placement tombstones for cross-branch copy.

use super::super::super::super::helpers::TOMBSTONE;
use super::super::super::super::NodeRepositoryImpl;
use super::parent_path_of;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Resolve a parent path to the parent id used by the ORDERED_CHILDREN
    /// index: `"/"` for root-level nodes, otherwise the parent node's id.
    pub(super) async fn resolve_parent_id_opt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Option<String>> {
        if parent_path == "/" {
            return Ok(Some("/".to_string()));
        }
        Ok(self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
            .await?
            .map(|p| p.id))
    }

    /// Retire a node that occupies the destination path on the target branch
    /// under a DIFFERENT id, so the copy replaces it instead of shadowing it.
    ///
    /// PATH_INDEX is keyed by `(branch, workspace, path, ~revision)` with the
    /// node id in the VALUE, so a copy whose id differs from the current
    /// occupant's simply overwrites the mapping and strands the previous node:
    /// its blob and every other index entry stay live, reachable by a table
    /// scan and by nothing else. Once that has happened the path is
    /// unrecoverable through normal means — `get_by_path` returns one
    /// generation, `CHILD_OF` resolves the parent to that generation and misses
    /// children filed under the others, a path-qualified DELETE reports zero
    /// rows, and an id-qualified DELETE fails resolving its parent by path.
    ///
    /// This is not hypothetical: a publish branch three days old had 88 nodes
    /// across 34 paths, and an event page rendered an empty programme because
    /// its tracks hung off a superseded copy of their parent.
    ///
    /// It happens whenever the SOURCE re-creates a node with a fresh id —
    /// `raisindb package deploy --install` does exactly that — and the result is
    /// then promoted onto a branch that still holds the previous generation.
    ///
    /// CALL THIS BEFORE staging the new node. The tombstone set includes
    /// PATH_INDEX at `(path, revision)`, which is the SAME key the incoming
    /// node writes; a WriteBatch applies in order, so tombstone-then-put leaves
    /// the new owner live, while put-then-tombstone would erase the path it
    /// just claimed.
    pub(super) async fn displace_path_occupant(
        &self,
        batch: &mut WriteBatch,
        occupant: &Node,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        // The shared tombstone module, not a hand-rolled CF list: it is the
        // single source of truth used by the repository, cascade and
        // transaction delete paths, and it covers NODE_PATH / COMPOUND_INDEX /
        // SPATIAL_INDEX, which the copy path's own prune helper still misses.
        let ctx = crate::tombstones::TombstoneContext::new(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
        );
        let cfs = crate::tombstones::TombstoneColumnFamilies::from_db(&self.db)?;
        crate::tombstones::add_node_tombstones(batch, &self.db, &ctx, &cfs, occupant, revision)?;

        // Unique indexes need a NodeType lookup, so they live outside the
        // shared module and have to be retired explicitly.
        self.add_unique_tombstones_to_batch(
            batch,
            occupant,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            revision,
        )
        .await?;

        Ok(())
    }

    /// Tombstone the stale PATH_INDEX and ORDERED_CHILDREN entries left on the
    /// target branch when a node moved/renamed/reordered on the source since
    /// the last promotion.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn tombstone_stale_placement(
        &self,
        batch: &mut WriteBatch,
        old_dst: &Node,
        new_node: &Node,
        new_parent_id: &str,
        new_label: &str,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        if old_dst.path != new_node.path {
            let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
            let old_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_dst.path,
                revision,
            );
            batch.put_cf(cf_path, old_path_key, TOMBSTONE);
        }

        let old_parent_path = parent_path_of(&old_dst.path);
        let old_parent_id = self
            .resolve_parent_id_opt(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_parent_path,
            )
            .await?;
        if let Some(old_pid) = old_parent_id {
            if let Some(old_label) = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_pid,
                &new_node.id,
            )? {
                if old_pid != new_parent_id || old_label != new_label {
                    let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
                    let ordered_key = keys::ordered_child_key_versioned(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        &old_pid,
                        &old_label,
                        revision,
                        &new_node.id,
                    );
                    batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
                }
            }
        }

        Ok(())
    }
}
