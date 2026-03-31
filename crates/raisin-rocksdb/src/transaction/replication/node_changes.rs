//! Building replicated node changes from tracked changes and resolving parent IDs.

use super::super::RocksDBTransaction;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};
use raisin_storage::{NodeRepository, StorageScope};
use std::collections::HashMap;

impl RocksDBTransaction {
    /// Build replicated node changes from tracked changes
    pub(in super::super) async fn build_replicated_node_changes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        tracked_changes: &HashMap<String, crate::replication::NodeChanges>,
    ) -> Result<Vec<ReplicatedNodeChange>> {
        let mut replicated = Vec::new();

        for (node_id, change) in tracked_changes {
            let workspace = change.workspace.as_str();
            let revision = change.revision;

            if change.is_delete {
                if let Some(replicated_change) = self
                    .build_delete_change(
                        tenant_id, repo_id, branch, workspace, node_id, &revision, change,
                    )
                    .await?
                {
                    replicated.push(replicated_change);
                }
                continue;
            }

            if let Some(replicated_change) = self
                .build_upsert_change(tenant_id, repo_id, branch, workspace, node_id, &revision)
                .await?
            {
                replicated.push(replicated_change);
            }
        }

        // Sort replicated changes by order_key to ensure deterministic ordering across cluster
        // This is critical for distributed consistency - all peers must apply changes
        // in the same order to maintain consistent node ordering (ORDERED_CHILDREN index)
        replicated.sort_by(|a, b| {
            // Primary sort: by order_key (lexicographic)
            match a.node.order_key.cmp(&b.node.order_key) {
                std::cmp::Ordering::Equal => {
                    // Secondary sort: by node_id for stability when order_keys are equal
                    a.node.id.cmp(&b.node.id)
                }
                other => other,
            }
        });

        Ok(replicated)
    }

    /// Build a replicated change for a deleted node
    async fn build_delete_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &HLC,
        change: &crate::replication::NodeChanges,
    ) -> Result<Option<ReplicatedNodeChange>> {
        if let Some(mut node_snapshot) = change.node_data.clone() {
            if node_snapshot.workspace.is_none() {
                node_snapshot.workspace = Some(workspace.to_string());
            }
            let parent_id = self
                .resolve_parent_id_for_node(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node_snapshot,
                    Some(revision),
                )
                .await?;

            let cf_order_key = self.resolve_cf_order_key(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id.as_deref(),
                &node_snapshot.id,
            )?;

            Ok(Some(ReplicatedNodeChange {
                node: node_snapshot,
                parent_id,
                kind: ReplicatedNodeChangeKind::Delete,
                cf_order_key,
            }))
        } else {
            tracing::warn!(
                node_id = %node_id,
                "Unable to include delete in ApplyRevision: missing snapshot data"
            );
            Ok(None)
        }
    }

    /// Build a replicated change for an upserted node
    async fn build_upsert_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<ReplicatedNodeChange>> {
        let node_opt = self
            .node_repo
            .get(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                node_id,
                Some(revision),
            )
            .await?;

        if let Some(mut node_snapshot) = node_opt {
            if node_snapshot.workspace.is_none() {
                node_snapshot.workspace = Some(workspace.to_string());
            }
            let parent_id = self
                .resolve_parent_id_for_node(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node_snapshot,
                    Some(revision),
                )
                .await?;

            let cf_order_key = self.resolve_cf_order_key(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id.as_deref(),
                &node_snapshot.id,
            )?;

            tracing::debug!(
                node_id = %node_snapshot.id,
                cf_order_key = %cf_order_key,
                "📊 Captured full CF order key for replication"
            );

            Ok(Some(ReplicatedNodeChange {
                node: node_snapshot,
                parent_id,
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key,
            }))
        } else {
            tracing::warn!(
                node_id = %node_id,
                revision = %revision,
                "Node missing at revision when building replicated change"
            );
            Ok(None)
        }
    }

    /// Retrieve full CF order key from ORDERED_CHILDREN (with node_id suffix)
    fn resolve_cf_order_key(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: Option<&str>,
        node_id: &str,
    ) -> Result<String> {
        if let Some(pid) = parent_id {
            Ok(self
                .node_repo
                .get_order_label_for_child(tenant_id, repo_id, branch, workspace, pid, node_id)?
                .unwrap_or_else(|| {
                    tracing::warn!(
                        node_id = %node_id,
                        "⚠️ Missing CF order key during replication capture - using empty string"
                    );
                    String::new()
                }))
        } else {
            tracing::debug!(
                node_id = %node_id,
                "No parent_id for node during replication capture - using empty string"
            );
            Ok(String::new())
        }
    }

    /// Resolve parent ID for a node based on its parent_path
    pub(in super::super) async fn resolve_parent_id_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        if let Some(parent_path) = node.parent_path() {
            if parent_path == "/" {
                return Ok(Some("/".to_string()));
            }

            if let Some(parent) = self
                .node_repo
                .get_by_path_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &parent_path,
                    max_revision,
                )
                .await?
            {
                return Ok(Some(parent.id));
            }
        }

        Ok(None)
    }
}
