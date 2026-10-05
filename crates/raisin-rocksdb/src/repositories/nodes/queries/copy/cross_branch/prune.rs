//! Retiring target-branch nodes during a promotion — the `delete_missing`
//! prune and the same-path displacement (`stage.rs`) — through ONE body.

use super::super::super::super::NodeRepositoryImpl;
use super::parent_path_of;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{CrossBranchNodeChange, NodeChangeInfo};
use rocksdb::WriteBatch;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Delete one target-branch node (`node` is its pre-delete state) into
    /// the shared cross-branch batch at `revision`.
    ///
    /// The shared delete tombstoner, not a hand-rolled CF list: it is the one
    /// body the repository, cascade, transaction, merge and replicated delete
    /// paths use, and a replica applies this very delete through it (the
    /// promotion replicates it as a `Delete` change). A copy here that missed
    /// NODE_PATH / COMPOUND / SPATIAL / SECRETS / the vmount registry left those
    /// live on the origin only — a pruned geometry kept matching `ST_DWITHIN`
    /// forever while every replica had dropped it. UNIQUE claims need a
    /// NodeType read, so they are retired explicitly beside it.
    ///
    /// The placement is resolved FIRST, from committed (pre-promotion) state:
    /// it is what the replicated `Delete` change carries, and the tombstoner
    /// is handed the same parent id.
    ///
    /// CALL THIS BEFORE staging any copied node. PATH_INDEX and UNIQUE keys
    /// carry no node id, so a replacement claiming the same path or value at
    /// this revision writes the same key; a WriteBatch applies in order, so
    /// delete-then-put leaves the new owner live, while put-then-delete erased
    /// the path (and claim) it had just taken.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn retire_target_node(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<PrunedPlacement> {
        let mut placement = PrunedPlacement::default();
        let parent_path = parent_path_of(&node.path);
        if let Some(parent_id) = self
            .resolve_parent_id_opt(tenant_id, repo_id, target_branch, workspace, &parent_path)
            .await?
        {
            if let Some(label) = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &parent_id,
                &node.id,
            )? {
                placement.label = label;
            }
            placement.parent_id = Some(parent_id);
        }

        let ctx =
            crate::tombstones::TombstoneContext::new(tenant_id, repo_id, target_branch, workspace);
        let cfs = crate::tombstones::TombstoneColumnFamilies::from_db(&self.db)?;
        crate::tombstones::add_node_tombstones_with_parent(
            batch,
            &self.db,
            &ctx,
            &cfs,
            node,
            revision,
            placement.parent_id.as_deref(),
        )?;
        self.add_unique_tombstones_to_batch(
            batch,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            revision,
        )
        .await?;

        Ok(placement)
    }

    /// `delete_missing`: tombstone every target-branch node under the copied
    /// roots that no longer exists in the copied source set. Runs BEFORE any
    /// node is staged (see [`Self::retire_target_node`]). Returns the pruned
    /// nodes (pre-delete state) with their placement, for the `Delete` changes
    /// of the copy's replicated `ApplyRevision`.
    ///
    /// A root is looked up on the target by its id AND by its path: a root the
    /// source re-created under a fresh id (`deploy --install`) leaves the
    /// previous generation at that path, and its subtree is pruned too —
    /// otherwise whatever the source no longer has stayed live beneath a parent
    /// id nothing resolves to any more.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prune_missing_targets(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
        root_ctxs: &[super::RootContext],
        src_ids: &HashSet<String>,
        changes: &mut Vec<CrossBranchNodeChange>,
        change_infos: &mut Vec<NodeChangeInfo>,
    ) -> Result<Vec<PrunedNode>> {
        let mut deleted_ids: HashSet<String> = HashSet::new();
        let mut pruned: Vec<PrunedNode> = Vec::new();
        for rc in root_ctxs {
            // The pre-copy target trees (nothing is staged yet, and the batch
            // is not committed, so these reads see the previous state).
            let mut subtree_roots: Vec<String> = Vec::new();
            if self
                .get_impl(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &rc.node.id,
                    false,
                )
                .await?
                .is_some()
            {
                subtree_roots.push(rc.node.id.clone());
            }
            if let Some(occupant) = self
                .get_by_path_impl(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &rc.node.path,
                    None,
                )
                .await?
            {
                if occupant.id != rc.node.id {
                    subtree_roots.push(occupant.id);
                }
            }
            for root_id in subtree_roots {
                let dst_set = self.scan_descendants_ordered_impl(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &root_id,
                    None,
                )?;
                for (dst_node, _) in dst_set {
                    if src_ids.contains(&dst_node.id) || !deleted_ids.insert(dst_node.id.clone()) {
                        continue;
                    }
                    let placement = self
                        .retire_target_node(
                            batch,
                            &dst_node,
                            tenant_id,
                            repo_id,
                            target_branch,
                            workspace,
                            revision,
                        )
                        .await?;
                    changes.push(CrossBranchNodeChange {
                        node_id: dst_node.id.clone(),
                        path: dst_node.path.clone(),
                        node_type: dst_node.node_type.clone(),
                        operation: ChangeOperation::Deleted,
                    });
                    change_infos.push(NodeChangeInfo {
                        node_id: dst_node.id.clone(),
                        workspace: workspace.to_string(),
                        operation: ChangeOperation::Deleted,
                        translation_locale: None,
                    });
                    pruned.push(PrunedNode {
                        node: dst_node,
                        placement,
                    });
                }
            }
        }

        Ok(pruned)
    }
}

/// Where a retired node sat among its siblings on the target.
#[derive(Debug, Default)]
pub(super) struct PrunedPlacement {
    /// The ORDERED_CHILDREN parent key (`/` for a root child), when resolved.
    pub(super) parent_id: Option<String>,
    /// Its ORDERED_CHILDREN label, or empty when it had none.
    pub(super) label: String,
}

/// A target node the promotion deleted — pruned by `delete_missing` or
/// displaced from its path by a node with a different id — as it was before.
#[derive(Debug)]
pub(super) struct PrunedNode {
    pub(super) node: Node,
    pub(super) placement: PrunedPlacement,
}
