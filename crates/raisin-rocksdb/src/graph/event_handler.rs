// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Event handler for graph projection invalidation
//!
//! Listens to `RelationAdded`/`RelationRemoved` events and marks affected
//! graph projections as stale in the GRAPH_PROJECTION column family.
//! Stale projections are lazily rebuilt on next algorithm access.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;

use raisin_events::{Event, EventHandler, NodeEventKind, ReplicationEventKind};

use super::background_compute::GraphComputeTask;
use super::projection_cache::GraphProjectionStore;
use crate::RocksDBStorage;

/// Marks graph projections as stale when relations change.
///
/// On `RelationAdded`/`RelationRemoved`: marks all projections on the
/// affected branch as stale in RocksDB. Only processes "outgoing"
/// direction events to avoid double-invalidation.
///
/// On `ReplicationEvent::OperationBatchApplied`: marks projections stale
/// so new cluster nodes rebuild projections after catching up.
pub struct GraphProjectionEventHandler {
    storage: Arc<RocksDBStorage>,
}

impl GraphProjectionEventHandler {
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }
}

impl EventHandler for GraphProjectionEventHandler {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                Event::Node(node_event) => {
                    match &node_event.kind {
                        NodeEventKind::RelationAdded { relation_type, .. }
                        | NodeEventKind::RelationRemoved { relation_type, .. } => {
                            // Only process outgoing direction to avoid double-invalidation
                            let is_outgoing = node_event
                                .metadata
                                .as_ref()
                                .and_then(|m| m.get("direction"))
                                .and_then(|v| v.as_str())
                                .map(|d| d == "outgoing")
                                .unwrap_or(true);

                            if is_outgoing {
                                // Mark projections stale in GRAPH_PROJECTION CF
                                if let Err(e) = GraphProjectionStore::mark_branch_stale(
                                    &node_event.tenant_id,
                                    &node_event.repository_id,
                                    &node_event.branch,
                                    &self.storage,
                                ) {
                                    tracing::warn!("Failed to mark projections stale: {}", e);
                                }

                                // Also mark algorithm results stale in GRAPH_CACHE CF
                                // so the background tick picks up the change
                                if let Err(e) = GraphComputeTask::mark_branch_stale(
                                    &self.storage,
                                    &node_event.tenant_id,
                                    &node_event.repository_id,
                                    &node_event.branch,
                                ) {
                                    tracing::warn!("Failed to mark cache stale: {}", e);
                                }

                                tracing::debug!(
                                    tenant = %node_event.tenant_id,
                                    repo = %node_event.repository_id,
                                    branch = %node_event.branch,
                                    relation_type = %relation_type,
                                    node_id = %node_event.node_id,
                                    "Graph projections and cache marked stale due to relation change"
                                );
                            }
                        }
                        _ => {} // Ignore non-relation events
                    }
                }
                Event::Replication(repl_event) => {
                    if matches!(repl_event.kind, ReplicationEventKind::OperationBatchApplied) {
                        if let Some(branch) = &repl_event.branch {
                            // Mark both projections and cache stale
                            let _ = GraphProjectionStore::mark_branch_stale(
                                &repl_event.tenant_id,
                                &repl_event.repository_id,
                                branch,
                                &self.storage,
                            );
                            let _ = GraphComputeTask::mark_branch_stale(
                                &self.storage,
                                &repl_event.tenant_id,
                                &repl_event.repository_id,
                                branch,
                            );

                            tracing::debug!(
                                tenant = %repl_event.tenant_id,
                                repo = %repl_event.repository_id,
                                branch = %branch,
                                "Graph projections marked stale after replication batch"
                            );
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "graph_projection_event_handler"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_events::{Event, NodeEvent, NodeEventKind, ReplicationEvent, ReplicationEventKind};
    use raisin_hlc::HLC;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn create_test_storage() -> (Arc<RocksDBStorage>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        (storage, temp_dir)
    }

    fn test_hlc() -> HLC {
        HLC::new(1_000_000, 0)
    }

    fn make_relation_event(kind: NodeEventKind, direction: Option<&str>) -> Event {
        let metadata = direction.map(|d| {
            let mut map = HashMap::new();
            map.insert(
                "direction".to_string(),
                serde_json::Value::String(d.to_string()),
            );
            map
        });

        Event::Node(NodeEvent {
            tenant_id: "t1".into(),
            repository_id: "r1".into(),
            branch: "main".into(),
            workspace_id: "ws1".into(),
            node_id: "node-a".into(),
            node_type: Some("Post".into()),
            revision: test_hlc(),
            kind,
            path: None,
            metadata,
        })
    }

    fn store_test_projection(storage: &RocksDBStorage) {
        use super::super::projection_cache::ProjectionKey;
        use raisin_graph_algorithms::GraphProjection;

        let key = ProjectionKey {
            tenant_id: "t1".into(),
            repo_id: "r1".into(),
            branch: "main".into(),
            config_id: "pagerank".into(),
        };
        let projection = GraphProjection::from_parts(
            vec!["a".to_string(), "b".to_string()],
            vec![("a".to_string(), "b".to_string())],
        );
        GraphProjectionStore::store(&key, &projection, "rev1".to_string(), storage).unwrap();
    }

    #[tokio::test]
    async fn test_relation_added_marks_stale() {
        use super::super::projection_cache::ProjectionKey;

        let (storage, _dir) = create_test_storage();
        store_test_projection(&storage);

        let handler = GraphProjectionEventHandler::new(Arc::clone(&storage));

        let event = make_relation_event(
            NodeEventKind::RelationAdded {
                relation_type: "FOLLOWS".into(),
                target_node_id: "node-b".into(),
            },
            Some("outgoing"),
        );

        handler.handle(&event).await.unwrap();

        // Projection should be stale now
        let key = ProjectionKey {
            tenant_id: "t1".into(),
            repo_id: "r1".into(),
            branch: "main".into(),
            config_id: "pagerank".into(),
        };
        let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
        assert!(
            loaded.is_none(),
            "Stale projection should return None from load()"
        );
    }

    #[tokio::test]
    async fn test_incoming_direction_skipped() {
        use super::super::projection_cache::ProjectionKey;

        let (storage, _dir) = create_test_storage();
        store_test_projection(&storage);

        let handler = GraphProjectionEventHandler::new(Arc::clone(&storage));

        let event = make_relation_event(
            NodeEventKind::RelationAdded {
                relation_type: "FOLLOWS".into(),
                target_node_id: "node-b".into(),
            },
            Some("incoming"),
        );

        handler.handle(&event).await.unwrap();

        let key = ProjectionKey {
            tenant_id: "t1".into(),
            repo_id: "r1".into(),
            branch: "main".into(),
            config_id: "pagerank".into(),
        };
        let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
        assert!(
            loaded.is_some(),
            "Incoming events should NOT mark projection stale"
        );
    }

    #[tokio::test]
    async fn test_non_relation_event_ignored() {
        use super::super::projection_cache::ProjectionKey;

        let (storage, _dir) = create_test_storage();
        store_test_projection(&storage);

        let handler = GraphProjectionEventHandler::new(Arc::clone(&storage));

        let event = Event::Node(NodeEvent {
            tenant_id: "t1".into(),
            repository_id: "r1".into(),
            branch: "main".into(),
            workspace_id: "ws1".into(),
            node_id: "node-a".into(),
            node_type: Some("Post".into()),
            revision: test_hlc(),
            kind: NodeEventKind::Updated,
            path: None,
            metadata: None,
        });

        handler.handle(&event).await.unwrap();

        let key = ProjectionKey {
            tenant_id: "t1".into(),
            repo_id: "r1".into(),
            branch: "main".into(),
            config_id: "pagerank".into(),
        };
        let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
        assert!(
            loaded.is_some(),
            "Non-relation events should NOT mark stale"
        );
    }

    #[tokio::test]
    async fn test_replication_batch_marks_stale() {
        use super::super::projection_cache::ProjectionKey;

        let (storage, _dir) = create_test_storage();
        store_test_projection(&storage);

        let handler = GraphProjectionEventHandler::new(Arc::clone(&storage));

        let event = Event::Replication(ReplicationEvent {
            tenant_id: "t1".into(),
            repository_id: "r1".into(),
            branch: Some("main".into()),
            workspace: None,
            operation_count: 42,
            kind: ReplicationEventKind::OperationBatchApplied,
            metadata: None,
        });

        handler.handle(&event).await.unwrap();

        let key = ProjectionKey {
            tenant_id: "t1".into(),
            repo_id: "r1".into(),
            branch: "main".into(),
            config_id: "pagerank".into(),
        };
        let loaded = GraphProjectionStore::load(&key, &storage).unwrap();
        assert!(
            loaded.is_none(),
            "Replication batch should mark projection stale"
        );
    }

    #[test]
    fn test_handler_name() {
        let (storage, _dir) = create_test_storage();
        let handler = GraphProjectionEventHandler::new(storage);
        assert_eq!(handler.name(), "graph_projection_event_handler");
    }
}
