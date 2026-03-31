// SPDX-License-Identifier: BSL-1.1

//! WebSocket event handler for RaisinDB
//!
//! This module subscribes to the internal event bus and forwards matching
//! events to WebSocket connections based on their subscriptions.
//! Includes row-level security (RLS) filtering.

mod event_types;
mod forwarding;

use raisin_events::{Event, EventHandler};
use raisin_storage::Storage;
use std::{future::Future, pin::Pin, sync::Arc};

use crate::registry::ConnectionRegistry;

/// Event handler that forwards RaisinDB events to WebSocket connections
///
/// This handler is registered with the EventBus and receives all events.
/// It then filters and forwards events to WebSocket connections based on
/// their subscription filters. RLS (Row-Level Security) is applied to ensure
/// users only receive events for nodes they have read access to.
pub struct WsEventHandler<S: Storage> {
    /// Registry of all active WebSocket connections
    registry: Arc<ConnectionRegistry>,

    /// Storage for fetching nodes for RLS evaluation
    storage: Arc<S>,
}

impl<S: Storage> WsEventHandler<S> {
    /// Create a new WebSocket event handler
    ///
    /// # Arguments
    /// * `registry` - The connection registry to use for looking up connections
    /// * `storage` - Storage for fetching nodes for RLS evaluation
    pub fn new(registry: Arc<ConnectionRegistry>, storage: Arc<S>) -> Self {
        Self { registry, storage }
    }

    /// Extract event information and forward to matching connections
    async fn forward_event(&self, event: &Event) {
        // Use workspace-indexed lookup for node events (most common case)
        // Falls back to get_all() for other event types
        let connections = match event {
            Event::Node(node_event) => {
                // Use workspace index for efficient lookup - only gets connections
                // subscribed to this workspace (plus wildcard subscribers)
                self.registry.get_by_workspace(&node_event.workspace_id)
            }
            _ => self.registry.get_all(),
        };

        tracing::debug!(
            "WsEventHandler received event - type: {:?}, candidate_connections: {}",
            match event {
                Event::Node(e) => format!("Node({:?})", e.kind),
                Event::Repository(e) => format!("Repository({:?})", e.kind),
                Event::Workspace(e) => format!("Workspace({:?})", e.kind),
                Event::Replication(e) => format!("Replication({:?})", e.kind),
                Event::Schema(e) => format!("Schema({:?})", e.kind),
            },
            connections.len()
        );

        if connections.is_empty() {
            tracing::trace!("No active WebSocket connections to forward event to");
            return;
        }

        // Extract event information based on event type
        match event {
            Event::Node(node_event) => {
                tracing::debug!(
                    "Forwarding node event - workspace: {}, node_id: {}, kind: {:?}",
                    node_event.workspace_id,
                    node_event.node_id,
                    node_event.kind
                );
                self.forward_node_event(node_event, &connections).await;
            }
            Event::Repository(repo_event) => {
                self.forward_repository_event(repo_event, &connections);
            }
            Event::Workspace(ws_event) => {
                self.forward_workspace_event(ws_event, &connections);
            }
            Event::Replication(repl_event) => {
                self.forward_replication_event(repl_event, &connections);
            }
            Event::Schema(schema_event) => {
                self.forward_schema_event(schema_event, &connections);
            }
        }
    }
}

impl<S: Storage + Send + Sync + 'static> EventHandler for WsEventHandler<S> {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.forward_event(event).await;
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "ws_event_handler"
    }
}

// Tests require a mock storage implementation
// TODO: Add integration tests with actual storage
#[cfg(test)]
mod tests {
    // Tests disabled until mock storage is implemented
    // The WsEventHandler now requires a Storage implementation for RLS
}
