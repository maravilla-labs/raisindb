// SPDX-License-Identifier: BSL-1.1

//! Global registry of active WebSocket connections
//!
//! This module provides a thread-safe registry for tracking all active WebSocket
//! connections. It's used by the event handler to forward events to subscribed clients.
//!
//! ## Workspace Index
//!
//! The registry maintains a workspace subscription index for efficient event routing.
//! Instead of checking all connections for each event, we can quickly look up which
//! connections are subscribed to a specific workspace.

use crate::connection::ConnectionState;
use dashmap::{DashMap, DashSet};
use parking_lot::RwLock;
use std::sync::Arc;

/// Special key for connections with wildcard subscriptions (no workspace filter)
const WILDCARD_WORKSPACE: &str = "*";

/// Global registry of active WebSocket connections with workspace indexing
///
/// This registry maintains a map of all active connections, allowing event handlers
/// to efficiently broadcast events to subscribed clients.
///
/// The workspace index provides O(1) lookup for connections interested in specific
/// workspaces, reducing the need to iterate all connections for each event.
pub struct ConnectionRegistry {
    /// Map of connection ID to connection state
    connections: DashMap<String, Arc<RwLock<ConnectionState>>>,
    /// Index: workspace -> set of connection IDs subscribed to that workspace
    /// The special key "*" contains connections with wildcard subscriptions
    workspace_subscribers: DashMap<String, DashSet<String>>,
}

impl ConnectionRegistry {
    /// Create a new empty connection registry
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            workspace_subscribers: DashMap::new(),
        }
    }

    /// Register a new connection
    ///
    /// # Arguments
    /// * `connection` - The connection state to register
    pub fn register(&self, connection: Arc<RwLock<ConnectionState>>) {
        let connection_id = connection.read().connection_id.clone();
        self.connections.insert(connection_id, connection);
    }

    /// Unregister a connection by ID
    ///
    /// # Arguments
    /// * `connection_id` - The ID of the connection to remove
    ///
    /// # Returns
    /// `true` if the connection was found and removed, `false` otherwise
    pub fn unregister(&self, connection_id: &str) -> bool {
        // Remove from workspace index
        self.workspace_subscribers.iter().for_each(|entry| {
            entry.value().remove(connection_id);
        });
        self.connections.remove(connection_id).is_some()
    }

    /// Get all registered connections
    ///
    /// Returns a vector of all active connection states. This creates a snapshot
    /// of the current connections, so it's safe to iterate even if connections
    /// are added or removed concurrently.
    pub fn get_all(&self) -> Vec<Arc<RwLock<ConnectionState>>> {
        self.connections
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    /// Get connections subscribed to a specific workspace (plus wildcard subscribers)
    ///
    /// This is much more efficient than `get_all()` when only a subset of connections
    /// are interested in events from a specific workspace.
    ///
    /// Returns connections that:
    /// - Have a subscription for the specific workspace, OR
    /// - Have a wildcard subscription (no workspace filter)
    pub fn get_by_workspace(&self, workspace: &str) -> Vec<Arc<RwLock<ConnectionState>>> {
        let connection_ids = DashSet::new();

        // Add connections subscribed to this specific workspace
        if let Some(subscribers) = self.workspace_subscribers.get(workspace) {
            for id in subscribers.iter() {
                connection_ids.insert(id.clone());
            }
        }

        // Add wildcard subscribers (no workspace filter = interested in all workspaces)
        if let Some(wildcard_subscribers) = self.workspace_subscribers.get(WILDCARD_WORKSPACE) {
            for id in wildcard_subscribers.iter() {
                connection_ids.insert(id.clone());
            }
        }

        // Resolve connection IDs to actual connections
        connection_ids
            .iter()
            .filter_map(|id| {
                self.connections
                    .get(id.as_str())
                    .map(|e| Arc::clone(e.value()))
            })
            .collect()
    }

    /// Register that a connection is subscribed to a workspace
    ///
    /// Call this when a subscription is added.
    /// For subscriptions with no workspace filter, pass `None`.
    pub fn add_workspace_subscription(&self, connection_id: &str, workspace: Option<&str>) {
        let key = workspace.unwrap_or(WILDCARD_WORKSPACE);
        self.workspace_subscribers
            .entry(key.to_string())
            .or_insert_with(DashSet::new)
            .insert(connection_id.to_string());
    }

    /// Unregister a workspace subscription
    ///
    /// Call this when a subscription is removed.
    pub fn remove_workspace_subscription(&self, connection_id: &str, workspace: Option<&str>) {
        let key = workspace.unwrap_or(WILDCARD_WORKSPACE);
        if let Some(subscribers) = self.workspace_subscribers.get(key) {
            subscribers.remove(connection_id);
        }
    }

    /// Get the number of active connections
    pub fn count(&self) -> usize {
        self.connections.len()
    }

    /// Get a specific connection by ID
    pub fn get(&self, connection_id: &str) -> Option<Arc<RwLock<ConnectionState>>> {
        self.connections
            .get(connection_id)
            .map(|entry| Arc::clone(entry.value()))
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_register_and_unregister() {
        let registry = ConnectionRegistry::new();
        let conn = Arc::new(RwLock::new(ConnectionState::new(
            "tenant1".to_string(),
            Some("repo1".to_string()),
            4,
            100,
        )));

        let connection_id = conn.read().connection_id.clone();

        // Register connection
        registry.register(Arc::clone(&conn));
        assert_eq!(registry.count(), 1);

        // Get connection
        let retrieved = registry.get(&connection_id);
        assert!(retrieved.is_some());

        // Unregister connection
        assert!(registry.unregister(&connection_id));
        assert_eq!(registry.count(), 0);

        // Unregister again should return false
        assert!(!registry.unregister(&connection_id));
    }

    #[test]
    fn test_registry_get_all() {
        let registry = ConnectionRegistry::new();

        let conn1 = Arc::new(RwLock::new(ConnectionState::new(
            "tenant1".to_string(),
            None,
            4,
            100,
        )));
        let conn2 = Arc::new(RwLock::new(ConnectionState::new(
            "tenant2".to_string(),
            None,
            4,
            100,
        )));

        registry.register(conn1);
        registry.register(conn2);

        let all = registry.get_all();
        assert_eq!(all.len(), 2);
    }
}
