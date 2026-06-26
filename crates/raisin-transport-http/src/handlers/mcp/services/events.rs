// SPDX-License-Identifier: BSL-1.1

//! [`EventSource`] bridging the in-process event bus onto the MCP engine's
//! [`NodeChange`] stream.

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use raisin_events::{Event, EventBus, EventBusExt, EventFilter, NodeEvent, NodeEventKind};
use raisin_mcp::{McpIdentity, NodeChange, NodeChangeStream};
use tokio::sync::mpsc;

/// Source of live node-change events for MCP `resources/subscribe`.
///
/// Wraps the storage event bus. Each [`subscribe`](EventSource::subscribe)
/// registers a filtered handler that forwards every [`NodeEvent`] in the
/// session's tenant / repository scope onto a channel; the engine then narrows
/// the resulting stream by workspace, branch, and path.
pub(in crate::handlers::mcp) struct BusEventSource {
    bus: Arc<dyn EventBus>,
    tenant_id: String,
    repo: String,
}

impl BusEventSource {
    /// Wrap the storage event bus, scoped to a tenant / repository.
    pub(in crate::handlers::mcp) fn new(
        bus: Arc<dyn EventBus>,
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
    ) -> Self {
        Self {
            bus,
            tenant_id: tenant_id.into(),
            repo: repo.into(),
        }
    }
}

impl raisin_mcp::EventSource for BusEventSource {
    fn subscribe(&self, _identity: &McpIdentity) -> NodeChangeStream {
        let (tx, mut rx) = mpsc::unbounded_channel::<NodeChange>();
        let tenant_id = self.tenant_id.clone();
        let repo = self.repo.clone();

        // A unique handler name per subscription so the bus keeps each distinct.
        let name = format!("mcp-resource-sub-{}", uuid::Uuid::new_v4());

        self.bus
            .subscribe_fn(name, EventFilter::AllNode, move |event| {
                let mapped = match event {
                    Event::Node(node_event)
                        if node_event.tenant_id == tenant_id
                            && node_event.repository_id == repo =>
                    {
                        map_node_event(node_event)
                    }
                    _ => None,
                };
                let tx = tx.clone();
                Box::pin(async move {
                    if let Some(change) = mapped {
                        // A closed receiver (subscriber went away) is not an error.
                        let _ = tx.send(change);
                    }
                    Ok(())
                })
            });

        let stream = async_stream::stream! {
            while let Some(change) = rx.recv().await {
                yield change;
            }
        };
        Box::pin(stream) as Pin<Box<dyn Stream<Item = NodeChange> + Send>>
    }
}

/// Project a core [`NodeEvent`] onto the engine-facing [`NodeChange`].
fn map_node_event(event: &NodeEvent) -> Option<NodeChange> {
    let kind = match &event.kind {
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

    Some(NodeChange {
        workspace: event.workspace_id.clone(),
        branch: event.branch.clone(),
        node_id: event.node_id.clone(),
        path: event.path.clone(),
        node_type: event.node_type.clone(),
        kind: kind.to_string(),
    })
}
