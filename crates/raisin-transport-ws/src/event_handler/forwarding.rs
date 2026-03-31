// SPDX-License-Identifier: BSL-1.1

//! Event forwarding and RLS (Row-Level Security) filtering.

use raisin_core::services::rls_filter;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};
use raisin_storage::Storage;
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
    pub(super) fn forward_to_matching_connections(
        &self,
        workspace: &str,
        branch: &str,
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

        for connection in connections {
            let conn = connection.read();

            let all_subs = conn.get_subscriptions();
            tracing::debug!(
                "Connection {} has {} subscriptions",
                conn.connection_id,
                all_subs.len()
            );
            for (sub_id, filters) in &all_subs {
                tracing::debug!("   Subscription {}: workspace={:?}, path={:?}, event_types={:?}, node_type={:?}",
                    sub_id, filters.workspace, filters.path, filters.event_types, filters.node_type);
            }

            let matching_subs = conn.matches_subscription(workspace, path, event_type, node_type);

            tracing::debug!(
                "Connection {} has {} matching subscriptions",
                conn.connection_id,
                matching_subs.len()
            );

            if matching_subs.is_empty() {
                continue;
            }

            // RLS check: verify user can read this node
            if let Some(node) = node_for_rls {
                if let Some(auth) = conn.auth_context() {
                    if !auth.is_system
                        && !rls_filter::can_perform(node, Operation::Read, auth, &scope)
                    {
                        rls_filtered_count += 1;
                        tracing::debug!(
                            connection_id = %conn.connection_id,
                            user_id = ?auth.user_id,
                            node_id = %node.id,
                            "RLS filtered: user cannot read this node"
                        );
                        continue;
                    }
                } else {
                    rls_filtered_count += 1;
                    tracing::debug!(
                        connection_id = %conn.connection_id,
                        "RLS filtered: no auth context (anonymous)"
                    );
                    continue;
                }
            }

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
