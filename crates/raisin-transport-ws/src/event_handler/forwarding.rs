// SPDX-License-Identifier: BSL-1.1

//! Event forwarding and RLS (Row-Level Security) filtering.

use raisin_core::services::rls_filter;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};
use raisin_storage::{scope::BranchScope, Storage};
use std::sync::Arc;
use tracing::{debug, trace, warn};

use crate::protocol::EventMessage;

use super::WsEventHandler;

impl<S: Storage> WsEventHandler<S> {
    /// Forward an event to all connections with matching subscriptions
    ///
    /// # Arguments
    /// * `workspace` - The workspace ID for scope-based RLS checks
    /// * `branch` - The branch for scope-based RLS checks
    /// * `path` - The node path for subscription matching
    /// * `event_type` - The event type string
    /// * `node_type` - Optional node type for subscription matching
    /// * `payload` - The event payload to send
    /// * `connections` - All active connections to check
    /// * `node_for_rls` - Optional node for RLS evaluation (None skips RLS)
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn forward_to_matching_connections(
        &self,
        workspace: &str,
        branch: &str,
        tenant_id: &str,
        repo_id: &str,
        path: &str,
        event_type: &str,
        node_type: Option<&str>,
        payload: serde_json::Value,
        connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
        node_for_rls: Option<&Node>,
    ) {
        let mut forwarded_count = 0;
        let mut error_count = 0;
        let mut rls_filtered_count = 0;

        tracing::info!(
            "Checking subscriptions - workspace: {}, path: {}, event_type: {}, node_type: {:?}, has_rls_node: {}",
            workspace,
            path,
            event_type,
            node_type,
            node_for_rls.is_some()
        );

        let scope = PermissionScope::new(workspace, branch);

        // Build a cache-backed graph resolver once (borrows self.storage) so
        // `RELATES … VIA` RLS conditions can be evaluated. Only needed when
        // there is a node to authorize. Evaluated at the latest committed state.
        let revision = raisin_hlc::HLC::now();
        let resolver = if node_for_rls.is_some() {
            self.storage
                .graph_resolver(BranchScope::new(tenant_id, repo_id, branch), &revision)
        } else {
            None
        };

        for connection in connections {
            // Phase 1: snapshot subscription-match + auth under the read guard,
            // then release it before any await (parking_lot guards are not
            // async-safe and must not be held across `.await`).
            let (auth_opt, has_matching) = {
                let conn = connection.read();
                // Tenant boundary: a connection may only ever receive events
                // for its own tenant. `ConnectionRegistry::get_by_workspace`
                // indexes purely by workspace name across all tenants (e.g.
                // every tenant's inbox lives under the same
                // "raisin:access_control" workspace), so this check is the
                // one place that actually enforces tenant isolation for the
                // WS event fan-out. Do not remove it without re-introducing
                // a cross-tenant leak.
                if conn.tenant_id != tenant_id {
                    (None, false)
                } else {
                    let has_matching = !conn
                        .matches_subscription(workspace, path, event_type, node_type)
                        .is_empty();
                    (conn.auth_context().cloned(), has_matching)
                }
            };

            if !has_matching {
                continue;
            }

            // Phase 2: RLS check (async; guard already released).
            if let Some(node) = node_for_rls {
                let allowed = match &auth_opt {
                    Some(auth) if auth.is_system => true,
                    Some(auth) => {
                        rls_filter::can_perform_async(
                            node,
                            Operation::Read,
                            auth,
                            &scope,
                            resolver.as_deref(),
                        )
                        .await
                    }
                    None => false,
                };
                if !allowed {
                    rls_filtered_count += 1;
                    tracing::debug!(
                        node_id = %node.id,
                        "RLS filtered: connection cannot read this node"
                    );
                    continue;
                }
            }

            // Phase 3: re-acquire the read guard and forward. No `.await` is
            // held across the guard here.
            let conn = connection.read();
            let matching_subs = conn.matches_subscription(workspace, path, event_type, node_type);

            // Serialize node once if any subscription wants it (optimization)
            let node_json: Option<serde_json::Value> =
                if matching_subs.iter().any(|(_, f)| f.include_node) {
                    node_for_rls.and_then(|n| serde_json::to_value(n).ok())
                } else {
                    None
                };

            // Forward event to each matching subscription
            for (subscription_id, filters) in matching_subs {
                let event_payload = if filters.include_node {
                    if let Some(ref node_value) = node_json {
                        let mut p = payload.clone();
                        if let serde_json::Value::Object(ref mut map) = p {
                            map.insert("node".to_string(), node_value.clone());
                        }
                        p
                    } else {
                        payload.clone()
                    }
                } else {
                    payload.clone()
                };

                let event_message = EventMessage::new(
                    subscription_id.clone(),
                    event_type.to_string(),
                    event_payload,
                );

                match conn.send_event(event_message) {
                    Ok(_) => {
                        forwarded_count += 1;
                        trace!(
                            connection_id = %conn.connection_id,
                            subscription_id = %subscription_id,
                            event_type = %event_type,
                            include_node = %filters.include_node,
                            "Forwarded event to WebSocket connection"
                        );
                    }
                    Err(e) => {
                        error_count += 1;
                        match e {
                            crate::connection::SendError::ChannelClosed => {
                                debug!(
                                    connection_id = %conn.connection_id,
                                    "Failed to send event: connection closed"
                                );
                            }
                            _ => {
                                warn!(
                                    connection_id = %conn.connection_id,
                                    error = %e,
                                    "Failed to send event to WebSocket connection"
                                );
                            }
                        }
                    }
                }
            }
        }

        if forwarded_count > 0 || rls_filtered_count > 0 {
            debug!(
                event_type = %event_type,
                forwarded_count = forwarded_count,
                rls_filtered_count = rls_filtered_count,
                error_count = error_count,
                "Event forwarding completed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::connection::ConnectionState;
    use crate::event_handler::WsEventHandler;
    use crate::protocol::SubscriptionFilters;
    use crate::registry::ConnectionRegistry;
    use parking_lot::RwLock;
    use raisin_events::{Event, EventHandler, NodeEvent, NodeEventKind};
    use raisin_storage_memory::InMemoryStorage;
    use std::sync::Arc;

    /// Registers a connection for `tenant_id`, subscribed (no path/type filter)
    /// to `workspace`, and returns its event receiver.
    fn register_subscriber(
        registry: &ConnectionRegistry,
        tenant_id: &str,
        workspace: &str,
    ) -> tokio::sync::mpsc::UnboundedReceiver<crate::protocol::EventMessage> {
        let conn = ConnectionState::new(tenant_id.to_string(), None, 4, 100);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        conn.set_event_channel(tx);
        let connection_id = conn.connection_id.clone();
        conn.add_subscription(
            "sub-1".to_string(),
            SubscriptionFilters {
                workspace: Some(workspace.to_string()),
                path: None,
                event_types: None,
                node_type: None,
                include_node: false,
            },
        );
        let conn = Arc::new(RwLock::new(conn));
        registry.register(conn);
        registry.add_workspace_subscription(&connection_id, Some(workspace));
        rx
    }

    fn node_event_for_tenant(tenant_id: &str) -> NodeEvent {
        NodeEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: "repo1".to_string(),
            branch: "main".to_string(),
            workspace_id: "raisin:access_control".to_string(),
            node_id: "task-1".to_string(),
            node_type: Some("raisin:InboxTask".to_string()),
            revision: raisin_hlc::HLC::now(),
            kind: NodeEventKind::Created,
            path: Some("/mtex/home/inbox/task-1".to_string()),
            metadata: None,
        }
    }

    /// A node event created in tenant "mtex" must never reach a connection
    /// belonging to a different tenant ("solutas"), even though both are
    /// subscribed to the exact same (tenant-agnostic) workspace name. This is
    /// the regression test for the cross-tenant inbox-notification leak.
    #[tokio::test]
    async fn node_event_is_not_forwarded_across_tenants() {
        let registry = Arc::new(ConnectionRegistry::new());
        let mut mtex_rx = register_subscriber(&registry, "mtex", "raisin:access_control");
        let mut solutas_rx = register_subscriber(&registry, "solutas", "raisin:access_control");

        let storage = Arc::new(InMemoryStorage::default());
        let handler = WsEventHandler::new(registry, storage);

        let event = Event::Node(node_event_for_tenant("mtex"));
        handler.handle(&event).await.expect("event handling failed");

        assert!(
            mtex_rx.try_recv().is_ok(),
            "same-tenant connection should receive its own tenant's event"
        );
        assert!(
            solutas_rx.try_recv().is_err(),
            "cross-tenant connection must NOT receive another tenant's event"
        );
    }

    /// End of one subscription, others in the same workspace keep delivering.
    ///
    /// A connection appears in a workspace's routing index ONCE however many
    /// subscriptions it holds there, so giving the entry up on the first
    /// unsubscribe stopped the connection being considered for that workspace
    /// at all — the remaining subscriptions still matched the event, but the
    /// event was never offered to them. A client that subscribes and
    /// unsubscribes as its views come and go went silent on the first teardown
    /// and only recovered by reconnecting.
    #[tokio::test]
    async fn a_remaining_subscription_still_receives_after_a_sibling_unsubscribes() {
        let registry = Arc::new(ConnectionRegistry::new());

        let conn = ConnectionState::new("tenant-a".to_string(), None, 4, 100);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        conn.set_event_channel(tx);
        let connection_id = conn.connection_id.clone();
        let workspace = "raisin:access_control";
        for (id, path) in [("sub-keep", "/**"), ("sub-drop", "/folder/**")] {
            conn.add_subscription(
                id.to_string(),
                SubscriptionFilters {
                    workspace: Some(workspace.to_string()),
                    path: Some(path.to_string()),
                    event_types: None,
                    node_type: None,
                    include_node: false,
                },
            );
            registry.add_workspace_subscription(&connection_id, Some(workspace));
        }
        registry.register(Arc::new(RwLock::new(conn.clone())));

        // One view closes: its subscription goes, the other stays.
        assert!(conn.remove_subscription("sub-drop"));
        registry.remove_workspace_subscription(&connection_id, Some(workspace));

        let storage = Arc::new(InMemoryStorage::default());
        let handler = WsEventHandler::new(Arc::clone(&registry), storage);
        handler
            .handle(&Event::Node(node_event_for_tenant("tenant-a")))
            .await
            .expect("event handling failed");

        assert!(
            rx.try_recv().is_ok(),
            "the subscription that is still open must still receive the event"
        );
    }
}
