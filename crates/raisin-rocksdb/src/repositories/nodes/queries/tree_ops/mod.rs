//! Tree operations for moving and renaming nodes
//!
//! This module provides functions for:
//! - Moving single nodes
//! - Moving entire node trees
//! - Renaming nodes
//!
//! # StorageNode Optimization
//!
//! Since nodes are stored as StorageNode (without path), move operations
//! only need to update indexes, not rewrite node blobs. This gives O(1)
//! cost per node for path changes (vs O(N) with embedded paths).
//!
//! Move operations update:
//! - PATH_INDEX: tombstone old path, write new path -> node_id
//! - NODE_PATH: write node_id -> new path
//! - ORDERED_CHILDREN: only if parent changes (root node only for tree moves)

mod move_tree;

use super::super::NodeRepositoryImpl;
use raisin_error::Result;

impl NodeRepositoryImpl {
    /// Rename node
    pub(in crate::repositories::nodes) async fn rename_node_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        old_path: &str,
        new_name: &str,
    ) -> Result<()> {
        let parent_path = old_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
        let new_path = if parent_path.is_empty() {
            format!("/{}", new_name)
        } else {
            format!("{}/{}", parent_path, new_name)
        };

        let node = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, old_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        // TODO: Create OperationMeta::Rename for audit trail
        // Route through the tree move so descendants are re-pathed AND move
        // events are emitted (fulltext/reference index maintenance).
        self.move_node_tree_impl(
            tenant_id, repo_id, branch, workspace, &node.id, &new_path, None,
        )
        .await
    }

    /// Publish `NodeEvent::Updated` for every node touched by a move, so the
    /// subscribed job handler runs the same index maintenance (fulltext,
    /// triggers, embeddings, reference retarget) it runs for the SQL/transaction
    /// move path. The direct repository move path (editor/API) writes indexes
    /// atomically but does not go through transaction-commit event emission, so
    /// without this the moved nodes stay searchable/referenced at their OLD path.
    ///
    /// Mirrors `RocksDBTransaction::emit_node_events`: `kind = Updated`,
    /// `source = "local"`, `node_data` embedded (lets the handler skip a re-read),
    /// plus `moved = true` + `old_path` so move-specific handlers (reference
    /// retarget) can act only on real moves. Best-effort: a node that can't be
    /// re-read is skipped with a warning rather than failing the move.
    pub(in crate::repositories::nodes) async fn emit_move_node_events(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &raisin_hlc::HLC,
        moved: &[(String, String)], // (node_id, old_path)
        actor: &str,
    ) {
        use raisin_events::{Event, EventBus, NodeEvent, NodeEventKind};

        for (node_id, old_path) in moved {
            // HEAD is now the move revision, so the node reads back at its NEW path.
            let node = match self
                .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
                .await
            {
                Ok(Some(n)) => n,
                Ok(None) => {
                    tracing::warn!(
                        node_id = %node_id,
                        workspace = %workspace,
                        "Node not found after move; skipping move event"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        node_id = %node_id,
                        error = %e,
                        "Failed to read node for move event; skipping"
                    );
                    continue;
                }
            };

            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String("local".to_string()),
            );
            metadata.insert(
                "actor".to_string(),
                serde_json::Value::String(actor.to_string()),
            );
            metadata.insert("moved".to_string(), serde_json::Value::Bool(true));
            metadata.insert(
                "old_path".to_string(),
                serde_json::Value::String(old_path.clone()),
            );
            if let Ok(data) = serde_json::to_value(&node) {
                metadata.insert("node_data".to_string(), data);
            }

            let event = NodeEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                branch: branch.to_string(),
                workspace_id: workspace.to_string(),
                node_id: node_id.clone(),
                node_type: Some(node.node_type.clone()),
                revision: *revision,
                kind: NodeEventKind::Updated,
                path: Some(node.path.clone()),
                metadata: Some(metadata),
            };
            self.event_bus.publish(Event::Node(event));
        }
    }
}
