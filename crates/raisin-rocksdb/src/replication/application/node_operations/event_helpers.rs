//! Event emission helpers for node operations

use raisin_events::NodeEventKind;
use raisin_hlc::HLC;
use std::collections::HashMap;

/// Emit a node event for both local and replicated operations
///
/// This ensures consistent event emission across all node operations.
/// Uses the same pattern as local operations in commit.rs to guarantee
/// websocket event delivery works identically for local and replicated changes.
pub(in crate::replication::application) fn emit_node_event(
    event_bus: &std::sync::Arc<dyn raisin_events::EventBus>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    node_type: Option<String>,
    path: Option<String>,
    revision: &HLC,
    kind: NodeEventKind,
    source: &str, // "local" or "replication"
) {
    use raisin_events::{Event, NodeEvent};

    let mut metadata = HashMap::new();
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String(source.to_string()),
    );

    // Derive event_type string from kind (matching WsEventHandler logic)
    let event_type = match &kind {
        NodeEventKind::Created => "node:created",
        NodeEventKind::Updated => "node:updated",
        NodeEventKind::Deleted => "node:deleted",
        NodeEventKind::Reordered => "node:reordered",
        NodeEventKind::Published => "node:published",
        NodeEventKind::Unpublished => "node:unpublished",
        NodeEventKind::PropertyChanged { .. } => "node:property_changed",
        NodeEventKind::RelationAdded { .. } => "node:relation_added",
        NodeEventKind::RelationRemoved { .. } => "node:relation_removed",
    };

    tracing::info!(
        event_type = %event_type,
        node_id = %node_id,
        workspace = %workspace,
        node_type = ?node_type,
        path = ?path,
        kind = ?kind,
        revision = %revision,
        source = %source,
        "Publishing node event to EventBus"
    );

    let event = NodeEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: workspace.to_string(),
        node_id: node_id.to_string(),
        node_type,
        revision: *revision,
        kind,
        path,
        metadata: Some(metadata),
    };

    event_bus.publish(Event::Node(event));
}
