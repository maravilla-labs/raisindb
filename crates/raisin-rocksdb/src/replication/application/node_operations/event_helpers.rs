//! Event emission helpers for node operations

use raisin_events::NodeEventKind;
use raisin_hlc::HLC;
use raisin_replication::Operation;
use std::collections::HashMap;

/// Who a replicated write is attributed to, carried verbatim from the
/// originating cluster node.
///
/// RaisinDB is masterless: a peer replaying an operation is not a lesser copy of
/// the node that first accepted it, so its audit entry must record the SAME
/// identity. Both halves ride on the [`Operation`] itself, so this is a pure
/// read — nothing is re-derived (and nothing can be re-derived correctly here:
/// the local auth context on a replica belongs to whoever is replicating, not to
/// whoever wrote).
#[derive(Clone, Copy)]
pub(in crate::replication::application) struct EventAttribution<'a> {
    /// The user or system actor that performed the write, as recorded upstream.
    pub actor: &'a str,
    /// The non-human principal that initiated it (`mcp:…`, `agent:…`,
    /// `trigger:…`), when there was one.
    pub agent: Option<&'a str>,
}

impl<'a> EventAttribution<'a> {
    pub fn from_op(op: &'a Operation) -> Self {
        Self {
            actor: &op.actor,
            agent: op.agent.as_deref(),
        }
    }
}

/// Emit a node event for both local and replicated operations
///
/// This ensures consistent event emission across all node operations.
/// Uses the same pattern as local operations in commit.rs to guarantee
/// websocket event delivery works identically for local and replicated changes.
#[allow(clippy::too_many_arguments)]
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
    attribution: EventAttribution<'_>,
) {
    use raisin_events::{Event, NodeEvent};

    let mut metadata = HashMap::new();
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String(source.to_string()),
    );
    // Key names and value shapes must match `commit/events.rs` byte-for-byte:
    // the audit subscriber (`raisin-core/src/audit_events.rs`) reads exactly
    // "actor" and "agent" and does NOT branch on "source", so a replicated
    // event stamped this way produces an audit row identical to the local one.
    metadata.insert(
        "actor".to_string(),
        serde_json::Value::String(attribution.actor.to_string()),
    );
    if let Some(agent) = attribution.agent {
        metadata.insert(
            "agent".to_string(),
            serde_json::Value::String(agent.to_string()),
        );
    }

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
