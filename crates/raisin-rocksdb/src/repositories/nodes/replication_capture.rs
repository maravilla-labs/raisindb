//! ApplyRevision snapshot capture for direct (non-transaction) repo writes.
//!
//! Direct `NodeRepository` writes (add/update/delete/move) replicate the same
//! way transaction commits do: a single `ApplyRevision` operation carrying
//! full node snapshots. Granular ops (`CreateNode`/`SetProperty`/`DeleteNode`)
//! could drop state the peer needs (path/name changes, removed properties,
//! moved subtrees), so every direct write path funnels through this helper.

use super::NodeRepositoryImpl;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};

impl NodeRepositoryImpl {
    /// Capture one `ApplyRevision` operation covering the given node changes.
    ///
    /// Call AFTER the atomic RocksDB batch write so parent/order-label lookups
    /// observe the committed state. Never fails the write path: capture errors
    /// are logged and swallowed, matching the previous granular capture.
    pub(crate) async fn capture_apply_revision_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        changes: Vec<(Node, ReplicatedNodeChangeKind)>,
        revision: HLC,
    ) {
        if !self.operation_capture.is_enabled() || changes.is_empty() {
            return;
        }

        let mut node_changes = Vec::with_capacity(changes.len());
        for (mut node, kind) in changes {
            if node.workspace.is_none() {
                node.workspace = Some(workspace.to_string());
            }

            let parent_id = match node.parent_path() {
                Some(parent_path) if parent_path == "/" => Some("/".to_string()),
                Some(parent_path) => match self
                    .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                    .await
                {
                    Ok(Some(parent)) => Some(parent.id),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            node_id = %node.id,
                            error = %e,
                            "Failed to resolve parent for replication capture"
                        );
                        None
                    }
                },
                None => None,
            };

            // Full CF order key (label + node-id suffix) so peers reproduce the
            // exact ORDERED_CHILDREN entry. On delete the entry may already be
            // tombstoned - fall back to the node's own order_key.
            let cf_order_key = match parent_id.as_deref() {
                Some(pid) => self
                    .get_order_label_for_child(tenant_id, repo_id, branch, workspace, pid, &node.id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| node.order_key.clone()),
                None => String::new(),
            };

            node_changes.push(ReplicatedNodeChange {
                node,
                parent_id,
                kind,
                cf_order_key,
            });
        }

        self.capture_apply_revision_prepared(
            tenant_id,
            repo_id,
            branch,
            node_changes,
            revision,
            "system",
        )
        .await;
    }

    /// Capture one `ApplyRevision` operation from already-built node changes.
    ///
    /// For callers that already know each change's parent_id and CF order key
    /// (e.g. tree/branch copy, which allocated them during the write) - skips
    /// the per-node lookups `capture_apply_revision_snapshot` performs.
    pub(crate) async fn capture_apply_revision_prepared(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_changes: Vec<ReplicatedNodeChange>,
        revision: HLC,
        actor: &str,
    ) {
        if !self.operation_capture.is_enabled() || node_changes.is_empty() {
            return;
        }

        let op_type = raisin_replication::OpType::ApplyRevision {
            branch_head: revision,
            node_changes,
        };

        if let Err(e) = self
            .operation_capture
            .capture_operation_with_revision(
                tenant_id.to_string(),
                repo_id.to_string(),
                branch.to_string(),
                op_type,
                actor.to_string(),
                None,
                true,
                Some(revision),
            )
            .await
        {
            tracing::warn!(error = %e, "Failed to capture ApplyRevision for direct write");
        }
    }
}
