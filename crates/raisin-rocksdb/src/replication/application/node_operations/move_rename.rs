//! MoveNode and RenameNode operation handlers

use super::super::OperationApplicator;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::NodeEventKind;
use raisin_models::nodes::Node;
use raisin_replication::Operation;
use raisin_storage::BranchRepository;

/// Apply a RenameNode operation
pub(in crate::replication::application) async fn apply_rename_node(
    _applicator: &OperationApplicator,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    node_id: &str,
    new_name: &str,
    _op: &Operation,
) -> Result<()> {
    tracing::info!("Applying RenameNode: {} -> {}", node_id, new_name);
    // Simplified implementation
    Ok(())
}

/// Apply a MoveNode operation
#[allow(clippy::too_many_arguments)]
pub(in crate::replication::application) async fn apply_move_node(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    new_parent_id: Option<&str>,
    position: Option<&str>,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "Applying MoveNode: {} -> {:?} at position {:?} from node {}",
        node_id,
        new_parent_id,
        position,
        op.cluster_node_id
    );

    let new_revision = OperationApplicator::op_revision(op)?;

    // Read current node state
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, "default", node_id);
    let cf_nodes = cf_handle(&applicator.db, cf::NODES)?;

    let mut iter = applicator.db.iterator_cf(
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

    // Write tombstone for old ORDERED_CHILDREN entry
    if let Some(old_parent) = &old_parent_id {
        let cf_ordered = cf_handle(&applicator.db, cf::ORDERED_CHILDREN)?;

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

        applicator
            .db
            .put_cf(cf_ordered, old_ordered_key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::debug!(
            "Tombstone written for old ORDERED_CHILDREN: parent={}, order_key={}, child={}",
            parent_key,
            old_order_key,
            node_id
        );
    }

    // Write new ORDERED_CHILDREN entry
    if let Some(new_parent) = new_parent_id {
        let cf_ordered = cf_handle(&applicator.db, cf::ORDERED_CHILDREN)?;

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

        applicator
            .db
            .put_cf(cf_ordered, new_ordered_key, node_name.as_bytes())
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::debug!(
            "Added to ORDERED_CHILDREN index: parent={}, order_key={}, child={}",
            parent_key,
            new_order_key,
            node_id
        );

        if position.is_some() {
            node.order_key = new_order_key.to_string();
        }
    }

    // Update node's parent field and path
    let old_path = node.path.clone();
    node.parent = new_parent_id.map(|p| p.to_string());

    // Calculate new path
    let new_path = calculate_new_path(
        applicator,
        tenant_id,
        repo_id,
        branch,
        workspace,
        new_parent_id,
        &node.name,
        &old_path,
    )?;

    // Update PATH_INDEX if path changed
    if new_path != old_path {
        let cf_path = cf_handle(&applicator.db, cf::PATH_INDEX)?;

        // Tombstone old PATH_INDEX entry
        let old_path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &old_path,
            &new_revision,
        );
        applicator
            .db
            .put_cf(cf_path, old_path_key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::debug!("Tombstone written for old PATH_INDEX: path={}", old_path);

        // Update node path
        node.path = new_path.clone();

        // Create new PATH_INDEX entry
        let new_path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &new_path,
            &new_revision,
        );
        applicator
            .db
            .put_cf(cf_path, new_path_key, node_id.as_bytes())
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::debug!(
            "PATH_INDEX updated: old_path={} -> new_path={}",
            old_path,
            new_path
        );
    }

    // Update timestamps and version
    use chrono::DateTime;
    let timestamp = DateTime::from_timestamp_millis(op.timestamp_ms as i64);
    node.updated_at = timestamp;
    node.updated_by = Some(op.actor.clone());
    node.version += 1;

    // Write updated node with new revision
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

    applicator
        .db
        .put_cf(cf_nodes, node_key, node_value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    // Update branch HEAD
    applicator
        .branch_repo
        .update_head(tenant_id, repo_id, branch, new_revision)
        .await?;

    // Emit NodeEvent
    super::event_helpers::emit_node_event(
        &applicator.event_bus,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(node.node_type.clone()),
        Some(node.path.clone()),
        &new_revision,
        NodeEventKind::Updated,
        "replication",
    );

    tracing::info!(
        "MoveNode completed: {} moved from {:?} to {:?} (branch HEAD updated to revision {})",
        node_id,
        old_parent_id,
        new_parent_id,
        new_revision
    );

    Ok(())
}

/// Calculate the new path for a moved node by looking up the parent's path
#[allow(clippy::too_many_arguments)]
fn calculate_new_path(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    new_parent_id: Option<&str>,
    node_name: &str,
    old_path: &str,
) -> Result<String> {
    let new_path = if let Some(new_parent) = new_parent_id {
        if new_parent == "/" || new_parent.is_empty() {
            format!("/{}", node_name)
        } else {
            // Load parent node to get its path
            let cf_nodes = cf_handle(&applicator.db, cf::NODES)?;
            let parent_prefix =
                keys::node_key_prefix(tenant_id, repo_id, branch, workspace, new_parent);

            let mut parent_iter = applicator.db.iterator_cf(
                cf_nodes,
                rocksdb::IteratorMode::From(&parent_prefix, rocksdb::Direction::Forward),
            );

            let parent_path = if let Some(Ok((key, value))) = parent_iter.next() {
                if key.starts_with(&parent_prefix) {
                    if let Ok(parent_node) = rmp_serde::from_slice::<Node>(&value) {
                        Some(parent_node.path)
                    } else {
                        None
                    }
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
