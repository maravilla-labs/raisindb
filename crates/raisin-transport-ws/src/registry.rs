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
    ///
    /// SECURITY: this index is keyed by workspace name ONLY, with no tenant
    /// dimension — every tenant's connections subscribed to a given workspace
    /// name (e.g. every tenant's inbox lives under the same
    /// "raisin:access_control" workspace) land in the same bucket. `get_by_workspace`
    /// therefore returns candidates across ALL tenants; it is NOT itself a
    /// tenant boundary. The actual enforcement point is the `tenant_id`
    /// equality check in `event_handler::forwarding::forward_to_matching_connections`
    /// (and in `broadcast_permissions_changed`) — do not treat this index as
    /// tenant-safe, and do not remove that downstream check.
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
    /// Call this when a subscription is added. Idempotent: the index is a set,
    /// so a connection that already holds subscriptions for this workspace stays
    /// listed exactly once. Call it for EVERY added subscription, including one
    /// that deduplicated onto an existing id — skipping the call means the entry
    /// is never restored if the index has since been given up.
    ///
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
    /// Call this AFTER the subscription has been removed from the connection's
    /// own subscription list, since that list is what decides whether anything
    /// is left.
    ///
    /// The index holds a connection ONCE per workspace however many
    /// subscriptions it has there, so dropping it on the first removal would
    /// take every REMAINING subscription on that connection out of the
    /// workspace's fan-out with it: the connection still matches those filters,
    /// but `get_by_workspace` stops offering it the workspace's events, so they
    /// go silent while still appearing to be subscribed. The entry is therefore
    /// given up only once the connection holds nothing more for this workspace.
    ///
    /// That question is answered from the connection's own subscriptions rather
    /// than from a parallel counter, so the index cannot drift out of step with
    /// what actually matches.
    pub fn remove_workspace_subscription(&self, connection_id: &str, workspace: Option<&str>) {
        if self.connection_subscribes_to(connection_id, workspace) {
            return;
        }
        let key = workspace.unwrap_or(WILDCARD_WORKSPACE);
        if let Some(subscribers) = self.workspace_subscribers.get(key) {
            subscribers.remove(connection_id);
        }
    }

    /// Does this connection still hold a subscription filtered to `workspace`?
    ///
    /// `None` means the unfiltered (wildcard) subscriptions, matching the key
    /// `add_workspace_subscription` indexes them under.
    fn connection_subscribes_to(&self, connection_id: &str, workspace: Option<&str>) -> bool {
        // Take the `Arc` out and let the map guard go before locking the
        // connection, so a shard guard is never held across another lock.
        let Some(connection) = self.get(connection_id) else {
            return false;
        };
        let subscriptions = connection.read().get_subscriptions();
        subscriptions
            .iter()
            .any(|(_, filters)| filters.workspace.as_deref() == workspace)
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
    use crate::protocol::SubscriptionFilters;

    /// A registered connection, plus its id.
    fn registered(registry: &ConnectionRegistry) -> (Arc<RwLock<ConnectionState>>, String) {
        let conn = Arc::new(RwLock::new(ConnectionState::new(
            "tenant1".to_string(),
            None,
            4,
            100,
        )));
        let id = conn.read().connection_id.clone();
        registry.register(Arc::clone(&conn));
        (conn, id)
    }

    fn filters(workspace: Option<&str>, path: &str) -> SubscriptionFilters {
        SubscriptionFilters {
            workspace: workspace.map(str::to_string),
            path: Some(path.to_string()),
            event_types: None,
            node_type: None,
            include_node: false,
        }
    }

    /// Add a subscription the way the subscribe handler does: to the connection
    /// AND to the workspace index. Returns the id the connection kept.
    fn subscribe(
        registry: &ConnectionRegistry,
        conn: &Arc<RwLock<ConnectionState>>,
        id: &str,
        filters: SubscriptionFilters,
    ) -> String {
        let workspace = filters.workspace.clone();
        let (connection_id, actual_id) = {
            let guard = conn.read();
            let actual = guard.add_subscription(id.to_string(), filters);
            (guard.connection_id.clone(), actual)
        };
        registry.add_workspace_subscription(&connection_id, workspace.as_deref());
        actual_id
    }

    /// Remove a subscription the way the unsubscribe handler does.
    fn unsubscribe(
        registry: &ConnectionRegistry,
        conn: &Arc<RwLock<ConnectionState>>,
        id: &str,
    ) -> bool {
        let (removed, connection_id, workspace) = {
            let guard = conn.read();
            let workspace = guard
                .get_subscriptions()
                .iter()
                .find(|(sub_id, _)| sub_id == id)
                .and_then(|(_, filters)| filters.workspace.clone());
            (
                guard.remove_subscription(id),
                guard.connection_id.clone(),
                workspace,
            )
        };
        if removed {
            registry.remove_workspace_subscription(&connection_id, workspace.as_deref());
        }
        removed
    }

    fn indexed(registry: &ConnectionRegistry, workspace: &str, connection_id: &str) -> bool {
        registry
            .get_by_workspace(workspace)
            .iter()
            .any(|c| c.read().connection_id == connection_id)
    }

    /// The regression this index existed to cause: a connection holds several
    /// subscriptions in ONE workspace, and is listed there once. Ending one of
    /// them must not take the others out of the workspace's fan-out — they still
    /// match, and a connection that is subscribed but never offered the event is
    /// silent with no way to tell.
    #[test]
    fn ending_one_subscription_keeps_the_others_receiving() {
        let registry = ConnectionRegistry::new();
        let (conn, id) = registered(&registry);

        subscribe(&registry, &conn, "sub-a", filters(Some("ws"), "/**"));
        subscribe(&registry, &conn, "sub-b", filters(Some("ws"), "/folder/**"));
        assert!(indexed(&registry, "ws", &id));

        assert!(unsubscribe(&registry, &conn, "sub-b"));
        assert!(
            indexed(&registry, "ws", &id),
            "the remaining subscription must still be offered the workspace's events"
        );

        // Only when nothing is left does the connection give up the bucket.
        assert!(unsubscribe(&registry, &conn, "sub-a"));
        assert!(!indexed(&registry, "ws", &id));
    }

    /// Workspaces are independent: ending the last subscription in one leaves
    /// another workspace's subscriptions on the same connection alone.
    #[test]
    fn ending_a_workspace_does_not_touch_another() {
        let registry = ConnectionRegistry::new();
        let (conn, id) = registered(&registry);

        subscribe(&registry, &conn, "sub-a", filters(Some("one"), "/**"));
        subscribe(&registry, &conn, "sub-b", filters(Some("two"), "/**"));

        assert!(unsubscribe(&registry, &conn, "sub-a"));
        assert!(!indexed(&registry, "one", &id));
        assert!(indexed(&registry, "two", &id));
    }

    /// Unfiltered subscriptions are indexed under the wildcard key and follow
    /// the same rule.
    #[test]
    fn wildcard_subscriptions_are_counted_the_same_way() {
        let registry = ConnectionRegistry::new();
        let (conn, id) = registered(&registry);

        subscribe(&registry, &conn, "sub-a", filters(None, "/**"));
        subscribe(&registry, &conn, "sub-b", filters(None, "/folder/**"));

        assert!(unsubscribe(&registry, &conn, "sub-a"));
        assert!(
            indexed(&registry, "any-workspace", &id),
            "a wildcard subscriber hears every workspace until its last subscription ends"
        );

        assert!(unsubscribe(&registry, &conn, "sub-b"));
        assert!(!indexed(&registry, "any-workspace", &id));
    }

    /// Identical filters deduplicate onto one subscription id, so the second
    /// subscribe adds no entry — but it must still index, because the entry it
    /// matched may have been given up in between. Re-subscribing after the
    /// workspace was released puts the connection back in the fan-out.
    #[test]
    fn a_deduplicated_subscribe_still_indexes_its_workspace() {
        let registry = ConnectionRegistry::new();
        let (conn, id) = registered(&registry);

        let first = subscribe(&registry, &conn, "sub-a", filters(Some("ws"), "/**"));
        let second = subscribe(&registry, &conn, "sub-b", filters(Some("ws"), "/**"));
        assert_eq!(first, second, "identical filters share one subscription");

        // Simulate an index entry lost for any reason, then re-subscribe.
        registry.unregister(&id);
        registry.register(Arc::clone(&conn));
        assert!(!indexed(&registry, "ws", &id));

        subscribe(&registry, &conn, "sub-c", filters(Some("ws"), "/**"));
        assert!(indexed(&registry, "ws", &id));
    }

    /// A connection that is gone keeps nothing: removal must not be blocked by
    /// a subscription list that can no longer be read.
    #[test]
    fn an_unregistered_connection_is_dropped_from_the_index() {
        let registry = ConnectionRegistry::new();
        let (conn, id) = registered(&registry);

        subscribe(&registry, &conn, "sub-a", filters(Some("ws"), "/**"));
        registry.unregister(&id);

        registry.remove_workspace_subscription(&id, Some("ws"));
        assert!(!indexed(&registry, "ws", &id));
    }

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
