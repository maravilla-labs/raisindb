//! Relation operation handlers for replication
//!
//! This module contains operation handlers for:
//! - apply_set_archetype
//! - apply_add_relation
//! - apply_remove_relation

use super::super::OperationApplicator;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::Operation;
use std::collections::HashMap;

/// Apply a SetArchetype operation
pub(super) async fn apply_set_archetype(
    _applicator: &OperationApplicator,
    _tenant_id: &str,
    _repo_id: &str,
    _branch: &str,
    node_id: &str,
    new_archetype: Option<&str>,
    _op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying SetArchetype: {} -> {:?}",
        node_id,
        new_archetype
    );
    // Simplified implementation
    Ok(())
}

/// Apply an AddRelation operation
///
/// Writes to both forward (source->target) and reverse (target->source) relation indexes.
/// Uses Last-Write-Wins (LWW) semantics based on HLC timestamps.
pub(super) async fn apply_add_relation(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_id: &str,
    source_workspace: &str,
    relation_type: &str,
    target_id: &str,
    target_workspace: &str,
    properties: &HashMap<String, PropertyValue>,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying AddRelation: {} --[{}]--> {}",
        source_id,
        relation_type,
        target_id
    );

    let revision = OperationApplicator::op_revision(op)?;

    // Serialize relation properties
    let value = rmp_serde::to_vec(&properties)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    // Write to forward index (source -> target)
    let forward_key = keys::relation_forward_key_versioned(
        tenant_id,
        repo_id,
        branch,
        source_workspace,
        source_id,
        relation_type,
        &revision,
        target_id,
    );

    let cf_relation = cf_handle(&applicator.db, cf::RELATION_INDEX)?;
    applicator
        .db
        .put_cf(cf_relation, forward_key, &value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    // Write to reverse index (target <- source)
    let reverse_key = keys::relation_reverse_key_versioned(
        tenant_id,
        repo_id,
        branch,
        target_workspace,
        target_id,
        relation_type,
        &revision,
        source_id,
    );

    applicator
        .db
        .put_cf(cf_relation, reverse_key, &value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!(
        "✅ Relation added: {} --[{}]--> {}",
        source_id,
        relation_type,
        target_id
    );

    // Emit websocket events for both source and target nodes
    emit_relation_events(
        applicator,
        tenant_id,
        repo_id,
        branch,
        source_workspace,
        source_id,
        target_workspace,
        target_id,
        relation_type,
        &revision,
        raisin_events::NodeEventKind::RelationAdded {
            relation_type: relation_type.to_string(),
            target_node_id: target_id.to_string(),
        },
    );

    Ok(())
}

/// Apply a RemoveRelation operation
///
/// Removes from both forward and reverse relation indexes.
/// Uses Last-Write-Wins (LWW) semantics based on HLC timestamps.
pub(super) async fn apply_remove_relation(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_id: &str,
    source_workspace: &str,
    relation_type: &str,
    target_id: &str,
    target_workspace: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying RemoveRelation: {} --[{}]--> {}",
        source_id,
        relation_type,
        target_id
    );
    eprintln!(
        "📥 APPLY: RemoveRelation: {} --[{}]--> {} (tenant={}, repo={}, branch={})",
        source_id, relation_type, target_id, tenant_id, repo_id, branch
    );

    let revision = OperationApplicator::op_revision(op)?;

    // Delete from forward index
    let forward_key = keys::relation_forward_key_versioned(
        tenant_id,
        repo_id,
        branch,
        source_workspace,
        source_id,
        relation_type,
        &revision,
        target_id,
    );

    let cf_relation = cf_handle(&applicator.db, cf::RELATION_INDEX)?;
    applicator
        .db
        .delete_cf(cf_relation, forward_key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    // Delete from reverse index
    let reverse_key = keys::relation_reverse_key_versioned(
        tenant_id,
        repo_id,
        branch,
        target_workspace,
        target_id,
        relation_type,
        &revision,
        source_id,
    );

    applicator
        .db
        .delete_cf(cf_relation, reverse_key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!(
        "✅ Relation removed: {} --[{}]--> {}",
        source_id,
        relation_type,
        target_id
    );

    // Emit websocket events for both source and target nodes
    emit_relation_events(
        applicator,
        tenant_id,
        repo_id,
        branch,
        source_workspace,
        source_id,
        target_workspace,
        target_id,
        relation_type,
        &revision,
        raisin_events::NodeEventKind::RelationRemoved {
            relation_type: relation_type.to_string(),
            target_node_id: target_id.to_string(),
        },
    );

    Ok(())
}

/// Helper function to emit websocket events for relation operations
///
/// Emits two events: one outgoing event on the source node and one incoming event on the target node.
/// This ensures that websocket clients listening to either node will receive real-time updates.
pub(super) fn emit_relation_events(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_workspace: &str,
    source_id: &str,
    target_workspace: &str,
    target_id: &str,
    relation_type: &str,
    revision: &raisin_hlc::HLC,
    event_kind: raisin_events::NodeEventKind,
) {
    use raisin_events::{Event, NodeEvent};

    tracing::info!(
        "🎬 emit_relation_events CALLED: {} --[{}]--> {} (source_ws={}, target_ws={})",
        source_id,
        relation_type,
        target_id,
        source_workspace,
        target_workspace
    );

    // Fetch source node metadata
    let source_node = match applicator.load_latest_node(tenant_id, repo_id, branch, source_id) {
        Ok(Some(node)) => node,
        Ok(None) => {
            tracing::warn!(
                "Skipping relation event emission: source node {} not found",
                source_id
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                "Failed to fetch source node metadata for relation event: {}",
                e
            );
            return;
        }
    };

    // Fetch target node metadata
    let target_node = match applicator.load_latest_node(tenant_id, repo_id, branch, target_id) {
        Ok(Some(node)) => node,
        Ok(None) => {
            tracing::warn!(
                "Skipping relation event emission: target node {} not found",
                target_id
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                "Failed to fetch target node metadata for relation event: {}",
                e
            );
            return;
        }
    };

    // Emit outgoing event on source node
    let mut outgoing_meta = HashMap::new();
    outgoing_meta.insert(
        "source".to_string(),
        serde_json::Value::String("replication".to_string()),
    );
    outgoing_meta.insert(
        "related_node_id".to_string(),
        serde_json::Value::String(target_id.to_string()),
    );
    outgoing_meta.insert(
        "related_workspace".to_string(),
        serde_json::Value::String(target_workspace.to_string()),
    );
    outgoing_meta.insert(
        "direction".to_string(),
        serde_json::Value::String("outgoing".to_string()),
    );

    let outgoing_event = NodeEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: source_workspace.to_string(),
        node_id: source_id.to_string(),
        node_type: Some(source_node.node_type.clone()),
        revision: *revision,
        kind: event_kind.clone(),
        path: Some(source_node.path.clone()),
        metadata: Some(outgoing_meta),
    };

    tracing::info!(
        node_id = %source_id,
        relation_type = %relation_type,
        target = %target_id,
        direction = "outgoing",
        "Emitting relation event for replicated operation"
    );

    tracing::info!(
        event_type = "node:relation_event",
        node_id = %source_id,
        workspace = %source_workspace,
        workspace_id = %outgoing_event.workspace_id,
        node_type = ?outgoing_event.node_type,
        path = ?outgoing_event.path,
        direction = "outgoing",
        relation_type = %relation_type,
        target = %target_id,
        revision = %revision,
        "🔔 Publishing replicated relation event to EventBus"
    );
    applicator.event_bus.publish(Event::Node(outgoing_event));

    // Emit incoming event on target node
    let mut incoming_meta = HashMap::new();
    incoming_meta.insert(
        "source".to_string(),
        serde_json::Value::String("replication".to_string()),
    );
    incoming_meta.insert(
        "related_node_id".to_string(),
        serde_json::Value::String(source_id.to_string()),
    );
    incoming_meta.insert(
        "related_workspace".to_string(),
        serde_json::Value::String(source_workspace.to_string()),
    );
    incoming_meta.insert(
        "direction".to_string(),
        serde_json::Value::String("incoming".to_string()),
    );

    let incoming_event = NodeEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: target_workspace.to_string(),
        node_id: target_id.to_string(),
        node_type: Some(target_node.node_type.clone()),
        revision: *revision,
        kind: event_kind,
        path: Some(target_node.path.clone()),
        metadata: Some(incoming_meta),
    };

    tracing::info!(
        node_id = %target_id,
        relation_type = %relation_type,
        source = %source_id,
        direction = "incoming",
        "Emitting relation event for replicated operation"
    );

    tracing::info!(
        event_type = "node:relation_event",
        node_id = %target_id,
        workspace = %target_workspace,
        workspace_id = %incoming_event.workspace_id,
        node_type = ?incoming_event.node_type,
        path = ?incoming_event.path,
        direction = "incoming",
        relation_type = %relation_type,
        source = %source_id,
        revision = %revision,
        "🔔 Publishing replicated relation event to EventBus"
    );
    applicator.event_bus.publish(Event::Node(incoming_event));
}
