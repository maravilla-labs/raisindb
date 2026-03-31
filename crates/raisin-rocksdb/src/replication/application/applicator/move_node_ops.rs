//! MoveNode operation handler

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_replication::Operation;
use raisin_storage::BranchRepository;

use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a MoveNode operation
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) async fn apply_move_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
        new_parent_id: Option<&str>,
        position: Option<&str>,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying MoveNode: {} -> {:?} at position {:?} from node {}",
            node_id,
            new_parent_id,
            position,
            op.cluster_node_id
        );

        let new_revision = Self::op_revision(op)?;

        // Step 1: Read current node state
        let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, "default", node_id);
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;

        let mut iter = self.db.iterator_cf(
            cf_nodes,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );
        let mut current_node: Option<Node> = None;

        while let Some(Ok((key, value))) = iter.next() {
            if !key.starts_with(&prefix) {
                break;
            }
            if let Ok(node) = rmp_serde::from_slice::<Node>(&value) {
                current_node = Some(node);
                break;
            }
        }

        let mut node = match current_node {
            Some(n) => n,
            None => {
                tracing::warn!(
                    "Cannot apply MoveNode: node {} not found in database",
                    node_id
                );
                return Ok(());
            }
        };

        let old_parent_id = node.parent.clone();
        let old_order_key = node.order_key.clone();
        let node_name = node.name.clone();
        let workspace = node.workspace.as_deref().unwrap_or("default");

        // Step 2: Write tombstone for old ORDERED_CHILDREN entry
        if let Some(old_parent) = &old_parent_id {
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            let parent_key = if old_parent.is_empty() || old_parent == "/" {
                "/"
            } else {
                old_parent.as_str()
            };

            let old_ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_key,
                &old_order_key,
                &new_revision,
                node_id,
            );
            self.db
                .put_cf(cf_ordered, old_ordered_key, b"")
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        // Step 3: Write new ORDERED_CHILDREN entry
        if let Some(new_parent) = new_parent_id {
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            let parent_key = if new_parent.is_empty() || new_parent == "/" {
                "/"
            } else {
                new_parent
            };
            let new_order_key = position.unwrap_or(&old_order_key);

            let new_ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_key,
                new_order_key,
                &new_revision,
                node_id,
            );
            self.db
                .put_cf(cf_ordered, new_ordered_key, node_name.as_bytes())
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if position.is_some() {
                node.order_key = new_order_key.to_string();
            }
        }

        // Step 4: Update node's parent field and path
        let old_path = node.path.clone();
        node.parent = new_parent_id.map(|p| p.to_string());

        let new_path = self.calculate_new_path(
            tenant_id,
            repo_id,
            branch,
            workspace,
            new_parent_id,
            &node.name,
            &old_path,
            cf_nodes,
        )?;

        // Only update path and PATH_INDEX if the path actually changed
        if new_path != old_path {
            let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;

            // Tombstone old PATH_INDEX entry
            let old_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &old_path,
                &new_revision,
            );
            self.db
                .put_cf(cf_path, old_path_key, b"")
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            node.path = new_path.clone();

            let new_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &new_path,
                &new_revision,
            );
            self.db
                .put_cf(cf_path, new_path_key, node_id.as_bytes())
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        // Update timestamps and version
        use chrono::DateTime;
        let timestamp = DateTime::from_timestamp_millis(op.timestamp_ms as i64);
        node.updated_at = timestamp;
        node.updated_by = Some(op.actor.clone());
        node.version += 1;

        // Step 5: Write updated node with new revision
        let node_key = keys::node_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            &new_revision,
        );
        let node_value = rmp_serde::to_vec_named(&node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db
            .put_cf(cf_nodes, node_key, node_value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, new_revision)
            .await?;

        let workspace = node.workspace.as_deref().unwrap_or("default");
        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            Some(node.node_type.clone()),
            Some(node.path.clone()),
            &new_revision,
            raisin_events::NodeEventKind::Updated,
            "replication",
        );

        tracing::info!(
            "✅ MoveNode completed: {} moved from {:?} to {:?} (branch HEAD updated to revision {})",
            node_id,
            old_parent_id,
            new_parent_id,
            new_revision
        );

        Ok(())
    }

    /// Calculate the new path for a moved node
    fn calculate_new_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        new_parent_id: Option<&str>,
        node_name: &str,
        old_path: &str,
        cf_nodes: &rocksdb::ColumnFamily,
    ) -> Result<String> {
        let new_path = if let Some(new_parent) = new_parent_id {
            if new_parent == "/" || new_parent.is_empty() {
                format!("/{}", node_name)
            } else {
                let parent_prefix =
                    keys::node_key_prefix(tenant_id, repo_id, branch, workspace, new_parent);
                let mut parent_iter = self.db.iterator_cf(
                    cf_nodes,
                    rocksdb::IteratorMode::From(&parent_prefix, rocksdb::Direction::Forward),
                );

                let parent_path = if let Some(Ok((key, value))) = parent_iter.next() {
                    if key.starts_with(&parent_prefix) {
                        rmp_serde::from_slice::<Node>(&value).ok().map(|n| n.path)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(pp) = parent_path {
                    format!("{}/{}", pp, node_name)
                } else {
                    tracing::warn!(
                        "Parent node {} not found when calculating new path",
                        new_parent,
                    );
                    old_path.to_string()
                }
            }
        } else {
            format!("/{}", node_name)
        };

        Ok(new_path)
    }
}
