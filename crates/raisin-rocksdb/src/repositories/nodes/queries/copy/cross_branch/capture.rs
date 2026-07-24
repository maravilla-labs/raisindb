//! Replication capture for cross-branch copy (ApplyRevision + prune deletes).

use super::super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Capture replication operations for a cross-branch copy (post-commit):
    /// one ApplyRevision snapshot covering all copied nodes (same shape as
    /// transaction commits), plus DeleteNode per pruned node (only ids are
    /// known here; peers resolve the node locally with delete-wins).
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
        deleted_ids: &HashSet<String>,
    ) {
        use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};

        if !self.operation_capture.is_enabled() {
            return;
        }

        let node_changes = nodes_for_replication
            .iter()
            .map(|(node, parent_id, order_label)| {
                let mut node = node.clone();
                if node.workspace.is_none() {
                    node.workspace = Some(workspace.to_string());
                }
                ReplicatedNodeChange {
                    node,
                    parent_id: Some(parent_id.clone()),
                    kind: ReplicatedNodeChangeKind::Upsert,
                    cf_order_key: order_label.clone(),
                }
            })
            .collect();

        self.capture_apply_revision_prepared(
            tenant_id,
            repo_id,
            target_branch,
            node_changes,
            *revision,
            actor,
        )
        .await;

        for node_id in deleted_ids {
            let op_type = raisin_replication::OpType::DeleteNode {
                node_id: node_id.clone(),
            };
            let _ = self
                .operation_capture
                .capture_operation_with_revision(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    target_branch.to_string(),
                    op_type,
                    actor.to_string(),
                    None,
                    true,
                    Some(*revision),
                )
                .await;
        }
    }
}
