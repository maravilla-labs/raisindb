//! SetProperty operation handler

use super::super::OperationApplicator;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::NodeEventKind;
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_replication::Operation;
use raisin_storage::BranchRepository;

/// Apply a SetProperty operation
pub(in crate::replication::application) async fn apply_set_property(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    property_name: &str,
    value: &PropertyValue,
    op: &Operation,
) -> Result<()> {
    tracing::debug!(
        "Applying SetProperty: {} -> {:?} on node {}",
        property_name,
        value,
        node_id
    );

    let new_revision = OperationApplicator::op_revision(op)?;

    // Find the latest revision of this node
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, "default", node_id);
    let cf_nodes = cf_handle(&applicator.db, cf::NODES)?;

    let mut iter = applicator.db.iterator_cf(
        cf_nodes,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    );

    let mut latest_node: Option<Node> = None;

    while let Some(Ok((key, value))) = iter.next() {
        if !key.starts_with(&prefix) {
            break;
        }

        if let Ok(node) = rmp_serde::from_slice::<Node>(&value) {
            latest_node = Some(node);
            break;
        }
    }

    let mut node = match latest_node {
        Some(n) => n,
        None => {
            tracing::warn!(
                "Cannot apply SetProperty: node {} not found in database",
                node_id
            );
            return Ok(());
        }
    };

    // Update the property
    node.properties
        .insert(property_name.to_string(), value.clone());

    // Update timestamps
    use chrono::DateTime;
    let timestamp = DateTime::from_timestamp_millis(op.timestamp_ms as i64);
    node.updated_at = timestamp;
    node.updated_by = Some(op.actor.clone());
    node.version += 1;

    // Write new revision
    let key = keys::node_key_versioned(
        tenant_id,
        repo_id,
        branch,
        "default",
        node_id,
        &new_revision,
    );
    let value = rmp_serde::to_vec_named(&node)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    applicator
        .db
        .put_cf(cf_nodes, key, value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    // Update branch HEAD
    applicator
        .branch_repo
        .update_head(tenant_id, repo_id, branch, new_revision)
        .await?;

    tracing::info!(
        "SetProperty applied: property={}, node={} (branch HEAD updated to revision {})",
        property_name,
        node_id,
        new_revision
    );

    // Emit NodeEvent
    let workspace = node.workspace.as_deref().unwrap_or("default");
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

    Ok(())
}
