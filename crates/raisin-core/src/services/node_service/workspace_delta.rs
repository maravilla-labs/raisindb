//! Workspace delta overlay helpers for NodeService
//!
//! Contains methods for merging workspace deltas with committed nodes.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{
    scope::StorageScope, transactional::TransactionalStorage, NodeRepository, Storage,
};

use super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Helper function to overlay workspace deltas on a list of committed nodes
    ///
    /// This merges workspace deltas (draft changes) with committed nodes:
    /// - Upserted nodes override committed nodes with same ID
    /// - Deleted nodes (tombstones) filter out committed nodes
    ///
    /// Returns the merged list of nodes representing the current workspace state.
    pub(crate) async fn overlay_workspace_deltas(
        &self,
        mut committed_nodes: Vec<models::nodes::Node>,
    ) -> Result<Vec<models::nodes::Node>> {
        // Get all workspace deltas for this branch
        let deltas = self
            .storage
            .list_workspace_deltas(StorageScope::new(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
            ))
            .await?;

        // Build lookup sets for efficient processing
        use std::collections::{HashMap, HashSet};
        let mut deleted_ids: HashSet<String> = HashSet::new();
        let mut upserted_nodes: HashMap<String, models::nodes::Node> = HashMap::new();

        // Process deltas
        for delta in deltas {
            match delta {
                raisin_models::workspace::DeltaOp::Delete { node_id, .. } => {
                    deleted_ids.insert(node_id);
                }
                raisin_models::workspace::DeltaOp::Upsert(node) => {
                    upserted_nodes.insert(node.id.clone(), *node);
                }
            }
        }

        // Filter out deleted nodes from committed list
        committed_nodes.retain(|n| !deleted_ids.contains(&n.id));

        // Override committed nodes with upserted versions (same ID)
        for node in &mut committed_nodes {
            if let Some(upserted) = upserted_nodes.remove(&node.id) {
                *node = upserted;
            }
        }

        // Add remaining upserted nodes (new nodes not in committed storage)
        committed_nodes.extend(upserted_nodes.into_values());

        Ok(committed_nodes)
    }

    /// Helper to get the ROOT node
    pub(crate) async fn get_root_node(
        &self,
        ctx: &dyn raisin_storage::transactional::TransactionalContext,
    ) -> Result<Option<models::nodes::Node>> {
        // Try transaction cache first
        if let Some(root) = ctx.get_node_by_path(&self.workspace_id, "/").await? {
            return Ok(Some(root));
        }
        // Fallback to committed storage
        self.storage
            .nodes()
            .get_by_path(self.scope(), "/", self.revision.as_ref())
            .await
    }
}
