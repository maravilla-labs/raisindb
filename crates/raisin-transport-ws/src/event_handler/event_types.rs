// SPDX-License-Identifier: BSL-1.1

//! Event type forwarding implementations.
//!
//! Contains the logic for forwarding different event types
//! (node, repository, workspace, replication, schema) to matching connections.

use raisin_events::{NodeEvent, ReplicationEvent, RepositoryEvent, SchemaEvent, WorkspaceEvent};
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde_json::Value;
use std::sync::Arc;

use super::WsEventHandler;

impl<S: Storage> WsEventHandler<S> {
    /// Forward a node event to matching connections
    pub(super) async fn forward_node_event(
        &self,
        event: &NodeEvent,
        connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
    ) {
        let workspace = &event.workspace_id;
        let path = event.path.as_deref().unwrap_or("");
        let node_type = event.node_type.as_deref();

        let event_type = match &event.kind {
            raisin_events::NodeEventKind::Created => "node:created",
            raisin_events::NodeEventKind::Updated => "node:updated",
            raisin_events::NodeEventKind::Deleted => "node:deleted",
            raisin_events::NodeEventKind::Reordered => "node:reordered",
            raisin_events::NodeEventKind::Published => "node:published",
            raisin_events::NodeEventKind::Unpublished => "node:unpublished",
            raisin_events::NodeEventKind::PropertyChanged { property: _ } => {
                "node:property_changed"
            }
            raisin_events::NodeEventKind::RelationAdded { .. } => "node:relation_added",
            raisin_events::NodeEventKind::RelationRemoved { .. } => "node:relation_removed",
        };

        tracing::info!(
            event_type = %event_type,
            node_id = %event.node_id,
            workspace = %workspace,
            path = %path,
            node_type = ?node_type,
            kind = ?event.kind,
            source = ?event.metadata.as_ref().and_then(|m| m.get("source")),
            "WsEventHandler received node event"
        );

        // Build event payload
        let mut payload = serde_json::json!({
            "tenant_id": event.tenant_id,
            "repository_id": event.repository_id,
            "branch": event.branch,
            "workspace_id": event.workspace_id,
            "node_id": event.node_id,
            "node_type": event.node_type,
            "revision": event.revision,
            "path": event.path,
            "kind": format!("{:?}", event.kind),
            "metadata": event.metadata,
        });

        if let Value::Object(ref mut map) = payload {
            match &event.kind {
                raisin_events::NodeEventKind::RelationAdded {
                    relation_type,
                    target_node_id,
                }
                | raisin_events::NodeEventKind::RelationRemoved {
                    relation_type,
                    target_node_id,
                } => {
                    map.insert(
                        "relation_type".to_string(),
                        Value::String(relation_type.clone()),
                    );
                    map.insert(
                        "target_node_id".to_string(),
                        Value::String(target_node_id.clone()),
                    );
                }
                raisin_events::NodeEventKind::PropertyChanged { property } => {
                    map.insert("property".to_string(), Value::String(property.clone()));
                }
                _ => {}
            }

            if let Some(metadata) = &event.metadata {
                if let Some(Value::String(related_id)) = metadata.get("related_node_id") {
                    map.insert(
                        "related_node_id".to_string(),
                        Value::String(related_id.clone()),
                    );
                }
                if let Some(Value::String(related_ws)) = metadata.get("related_workspace") {
                    map.insert(
                        "related_workspace".to_string(),
                        Value::String(related_ws.clone()),
                    );
                }
                if let Some(Value::String(direction)) = metadata.get("direction") {
                    map.insert(
                        "relation_direction".to_string(),
                        Value::String(direction.clone()),
                    );
                }
            }
        }

        // Fetch node for RLS evaluation if needed
        let node_for_rls: Option<Node> = self.resolve_node_for_rls(event, connections).await;

        self.forward_to_matching_connections(
            workspace,
            &event.branch,
            path,
            event_type,
            node_type,
            payload,
            connections,
            node_for_rls.as_ref(),
        );
    }

    /// Resolve a node for RLS evaluation from metadata or storage.
    async fn resolve_node_for_rls(
        &self,
        event: &NodeEvent,
        connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
    ) -> Option<Node> {
        let has_non_system_subscribers = connections.iter().any(|conn| {
            let conn = conn.read();
            if let Some(auth) = conn.auth_context() {
                !auth.is_system
            } else {
                true
            }
        });

        if !has_non_system_subscribers
            || matches!(event.kind, raisin_events::NodeEventKind::Deleted)
        {
            return None;
        }

        // Try metadata first (avoids DB read)
        let from_metadata = event
            .metadata
            .as_ref()
            .and_then(|m| m.get("node_data"))
            .and_then(|v| serde_json::from_value::<Node>(v.clone()).ok());

        if from_metadata.is_some() {
            tracing::trace!(
                node_id = %event.node_id,
                "Using node_data from event metadata for RLS (skipped DB read)"
            );
            return from_metadata;
        }

        // Fallback: fetch from DB
        match self
            .storage
            .nodes()
            .get(
                StorageScope::new(
                    &event.tenant_id,
                    &event.repository_id,
                    &event.branch,
                    &event.workspace_id,
                ),
                &event.node_id,
                Some(&event.revision),
            )
            .await
        {
            Ok(Some(node)) => Some(node),
            Ok(None) => {
                tracing::debug!(
                    node_id = %event.node_id,
                    "Node not found for RLS evaluation, skipping RLS filter"
                );
                None
            }
            Err(e) => {
                tracing::warn!(
                    node_id = %event.node_id,
                    error = %e,
                    "Failed to fetch node for RLS evaluation, skipping RLS filter"
                );
                None
            }
        }
    }

    /// Forward a repository event to matching connections
    pub(super) fn forward_repository_event(
        &self,
        event: &RepositoryEvent,
        connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
    ) {
        let workspace = event.workspace.as_deref().unwrap_or("");
        let branch = event.branch_name.as_deref().unwrap_or("main");
        let path = "";
        let node_type = None;

        let event_type = match event.kind {
            raisin_events::RepositoryEventKind::TenantCreated => "repository:tenant_created",
            raisin_events::RepositoryEventKind::Created => "repository:created",
            raisin_events::RepositoryEventKind::Updated => "repository:updated",
            raisin_events::RepositoryEventKind::Deleted => "repository:deleted",
            raisin_events::RepositoryEventKind::CommitCreated => "repository:commit_created",
            raisin_events::RepositoryEventKind::BranchCreated => "repository:branch_created",
            raisin_events::RepositoryEventKind::BranchUpdated => "repository:branch_updated",
            raisin_events::RepositoryEventKind::BranchDeleted => "repository:branch_deleted",
            raisin_events::RepositoryEventKind::TagCreated => "repository:tag_created",
            raisin_events::RepositoryEventKind::TagDeleted => "repository:tag_deleted",
        };

        let payload = serde_json::json!({
            "tenant_id": event.tenant_id,
            "repository_id": event.repository_id,
            "kind": format!("{:?}", event.kind),
            "workspace": event.workspace,
            "revision_id": event.revision_id,
            "branch_name": event.branch_name,
            "tag_name": event.tag_name,
            "message": event.message,
            "actor": event.actor,
            "metadata": event.metadata,
        });

        self.forward_to_matching_connections(
            workspace,
            branch,
            path,
            event_type,
            node_type,
            payload,
            connections,
            None,
        );
    }

    /// Forward a workspace event to matching connections
    pub(super) fn forward_workspace_event(
        &self,
        event: &WorkspaceEvent,
        connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
    ) {
        let workspace = &event.workspace;
        let branch = "main";
        let path = "";
        let node_type = None;

        let event_type = match event.kind {
            raisin_events::WorkspaceEventKind::Created => "workspace:created",
            raisin_events::WorkspaceEventKind::Updated => "workspace:updated",
            raisin_events::WorkspaceEventKind::Deleted => "workspace:deleted",
        };

        let payload = serde_json::json!({
            "tenant_id": event.tenant_id,
            "repository_id": event.repository_id,
            "workspace": event.workspace,
            "kind": format!("{:?}", event.kind),
            "metadata": event.metadata,
        });

        self.forward_to_matching_connections(
            workspace,
            branch,
            path,
            event_type,
            node_type,
            payload,
            connections,
            None,
        );
    }

    /// Forward a replication event to matching connections
    pub(super) fn forward_replication_event(
        &self,
        event: &ReplicationEvent,
        connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
    ) {
        let workspace = event.workspace.as_deref().unwrap_or("");
        let branch = event.branch.as_deref().unwrap_or("main");
        let path = "";
        let node_type = None;

        let event_type = match event.kind {
            raisin_events::ReplicationEventKind::OperationBatchApplied => {
                "replication:batch_applied"
            }
        };

        let payload = serde_json::json!({
            "tenant_id": event.tenant_id,
            "repository_id": event.repository_id,
            "branch": event.branch,
            "workspace": event.workspace,
            "operation_count": event.operation_count,
            "kind": format!("{:?}", event.kind),
            "metadata": event.metadata,
        });

        self.forward_to_matching_connections(
            workspace,
            branch,
            path,
            event_type,
            node_type,
            payload,
            connections,
            None,
        );
    }

    /// Forward a schema event to matching connections
    pub(super) fn forward_schema_event(
        &self,
        event: &SchemaEvent,
        _connections: &[Arc<parking_lot::RwLock<crate::connection::ConnectionState>>],
    ) {
        tracing::debug!(
            "Schema event received but not forwarded - schema_id: {}, schema_type: {}, kind: {:?}",
            event.schema_id,
            event.schema_type,
            event.kind
        );
    }
}
