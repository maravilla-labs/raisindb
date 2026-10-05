//! Replication capture for cross-branch copy: ONE `ApplyRevision` carrying
//! the target nodes the promotion deleted (prunes and displacements) and the
//! copied nodes (upserts), in that order.

use super::super::super::super::NodeRepositoryImpl;
use super::prune::PrunedNode;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Capture replication operations for a cross-branch copy (post-commit):
    /// one ApplyRevision covering all copied nodes AND every target node the
    /// promotion deleted — pruned by `delete_missing` or displaced from its
    /// path by a different id (same shape as transaction commits — a delete
    /// is a `Delete` change carrying the pre-delete node and its placement, so
    /// a peer tombstones every index family from the full node without
    /// looking it up; decomposed, the `DeleteNodeSnapshot` carries both).
    ///
    /// Deletes come FIRST, mirroring the origin's batch: a displaced node and
    /// its replacement share a PATH_INDEX key at this revision, and a peer
    /// applying the changes in order must end the old mapping before the new
    /// one is written. (Applied in the other order — an oplog entry an earlier
    /// binary captured upserts-first — the delete tombstoner's ownership check
    /// keeps a delete from erasing a path another node already holds.)
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn capture_cross_branch_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        actor: &str,
        revision: &HLC,
        nodes_for_replication: &[(Node, String, String)],
        deleted: impl Iterator<Item = &PrunedNode>,
    ) {
        use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};

        if !self.operation_capture.is_enabled() {
            return;
        }

        let in_workspace = |node: &Node| {
            let mut node = node.clone();
            if node.workspace.is_none() {
                node.workspace = Some(workspace.to_string());
            }
            node
        };
        let upserts = nodes_for_replication
            .iter()
            .map(|(node, parent_id, order_label)| ReplicatedNodeChange {
                node: in_workspace(node),
                parent_id: Some(parent_id.clone()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: order_label.clone(),
            });
        let deletes = deleted.map(|p| ReplicatedNodeChange {
            node: in_workspace(&p.node),
            parent_id: p.placement.parent_id.clone(),
            kind: ReplicatedNodeChangeKind::Delete,
            cf_order_key: p.placement.label.clone(),
        });
        let node_changes = deletes.chain(upserts).collect();

        self.capture_apply_revision_prepared(
            tenant_id,
            repo_id,
            target_branch,
            node_changes,
            *revision,
            crate::repositories::nodes::WriteAttribution::actor(Some(actor)),
        )
        .await;
    }
}
