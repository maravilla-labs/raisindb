//! Workspace structure initialization event handler
//!
//! Listens for WorkspaceCreated events and automatically creates
//! initial root-level nodes defined in the workspace's initial_structure field.
//!
//! Note: During initial repository setup, workspace structures are created directly
//! by NodeTypeInitHandler (after NodeTypes are initialized) to ensure correct ordering.
//! This handler is primarily for workspaces created after initial repository setup.

use anyhow::Result;
use raisin_core::workspace_structure_init::create_workspace_initial_structure;
use raisin_storage::{transactional::TransactionalStorage, Event, EventHandler, Storage};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Event handler that creates initial_structure nodes when a workspace is created
pub struct WorkspaceStructureInitHandler<S: Storage + TransactionalStorage> {
    storage: Arc<S>,
}

impl<S: Storage + TransactionalStorage> WorkspaceStructureInitHandler<S> {
    /// Create a new WorkspaceStructureInitHandler
    pub fn new(storage: Arc<S>) -> Self {
        tracing::info!("WorkspaceStructureInitHandler created and ready to handle events");
        Self { storage }
    }
}

impl<S: Storage + TransactionalStorage + 'static> EventHandler
    for WorkspaceStructureInitHandler<S>
{
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::debug!("=== WorkspaceStructureInitHandler.handle() called ===");
            tracing::debug!(
                "Event type: {}",
                match event {
                    Event::Node(_) => "Node",
                    Event::Workspace(_) => "Workspace",
                    Event::Repository(_) => "Repository",
                    Event::Replication(_) => "Replication",
                    Event::Schema(_) => "Schema",
                }
            );

            // Only handle WorkspaceCreated events
            if let Event::Workspace(workspace_event) = event {
                tracing::debug!("✓ Event is Workspace event");
                tracing::debug!("  Workspace event kind: {:?}", workspace_event.kind);
                tracing::debug!(
                    "  Tenant: {}, Repo: {}, Workspace: {}",
                    workspace_event.tenant_id,
                    workspace_event.repository_id,
                    workspace_event.workspace
                );

                if matches!(
                    workspace_event.kind,
                    raisin_storage::WorkspaceEventKind::Created
                ) {
                    tracing::info!(
                        "✓ Processing WorkspaceCreated event for {}/{}/{}",
                        workspace_event.tenant_id,
                        workspace_event.repository_id,
                        workspace_event.workspace
                    );

                    // Create initial_structure nodes for the new workspace
                    if let Err(e) = create_workspace_initial_structure(
                        self.storage.clone(),
                        &workspace_event.tenant_id,
                        &workspace_event.repository_id,
                        &workspace_event.workspace,
                    )
                    .await
                    {
                        tracing::error!(
                            "Failed to create initial_structure for workspace {}/{}/{}: {}",
                            workspace_event.tenant_id,
                            workspace_event.repository_id,
                            workspace_event.workspace,
                            e
                        );
                        // Don't fail the event - log the error but continue
                        // This allows the workspace to be created even if initial_structure fails
                    }
                }
            }

            Ok(())
        })
    }

    fn name(&self) -> &str {
        "WorkspaceStructureInitHandler"
    }
}

#[cfg(all(test, feature = "store-memory"))]
mod tests {
    use super::*;
    use raisin_hlc::HLC;
    use raisin_models::nodes::types::initial_structure::{InitialChild, InitialNodeStructure};
    use raisin_storage::{WorkspaceEvent, WorkspaceEventKind, WorkspaceRepository};
    use raisin_storage_memory::InMemoryStorage;

    #[tokio::test]
    async fn test_workspace_structure_init_on_workspace_created() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = WorkspaceStructureInitHandler::new(storage.clone());

        // Create a workspace with initial_structure
        let mut workspace = raisin_models::workspace::Workspace::new("test-ws".to_string());
        workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
        workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];
        workspace.initial_structure = Some(InitialNodeStructure {
            properties: None,
            children: Some(vec![
                InitialChild {
                    name: "folder1".to_string(),
                    node_type: "raisin:Folder".to_string(),
                    archetype: None,
                    properties: None,
                    translations: None,
                    children: None,
                },
                InitialChild {
                    name: "folder2".to_string(),
                    node_type: "raisin:Folder".to_string(),
                    archetype: None,
                    properties: None,
                    translations: None,
                    children: None,
                },
            ]),
        });

        storage
            .workspaces()
            .put("test-tenant", "test-repo", workspace)
            .await
            .unwrap();

        // Simulate WorkspaceCreated event
        let event = Event::Workspace(WorkspaceEvent {
            tenant_id: "test-tenant".to_string(),
            repository_id: "test-repo".to_string(),
            workspace: "test-ws".to_string(),
            kind: WorkspaceEventKind::Created,
            metadata: None,
        });

        // Handle the event
        handler.handle(&event).await.unwrap();

        // Verify nodes were created
        let node1 = storage
            .nodes()
            .get_by_path("test-tenant", "test-repo", "main", "test-ws", "/folder1")
            .await
            .unwrap();

        let node2 = storage
            .nodes()
            .get_by_path("test-tenant", "test-repo", "main", "test-ws", "/folder2")
            .await
            .unwrap();

        assert!(node1.is_some());
        assert!(node2.is_some());
        assert_eq!(node1.unwrap().name, "folder1");
        assert_eq!(node2.unwrap().name, "folder2");
    }

    #[tokio::test]
    async fn test_ignores_non_workspace_events() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = WorkspaceStructureInitHandler::new(storage.clone());

        // Create a NodeEvent (should be ignored)
        let event = Event::Node(raisin_storage::NodeEvent {
            tenant_id: "test".to_string(),
            repository_id: "test".to_string(),
            branch: "main".to_string(),
            workspace_id: "test-workspace".to_string(),
            node_id: "node1".to_string(),
            kind: raisin_storage::NodeEventKind::Created,
            node_type: None,
            path: None,
            revision: HLC::new(0, 0),
            metadata: None,
        });

        // Should not error, just ignore
        assert!(handler.handle(&event).await.is_ok());
    }

    #[tokio::test]
    async fn test_idempotent_initialization() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = WorkspaceStructureInitHandler::new(storage.clone());

        // Create a workspace with initial_structure
        let mut workspace = raisin_models::workspace::Workspace::new("test-ws".to_string());
        workspace.allowed_node_types = vec!["raisin:Folder".to_string()];
        workspace.allowed_root_node_types = vec!["raisin:Folder".to_string()];
        workspace.initial_structure = Some(InitialNodeStructure {
            properties: None,
            children: Some(vec![InitialChild {
                name: "folder1".to_string(),
                node_type: "raisin:Folder".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            }]),
        });

        storage
            .workspaces()
            .put("test-tenant", "test-repo", workspace)
            .await
            .unwrap();

        let event = Event::Workspace(WorkspaceEvent {
            tenant_id: "test-tenant".to_string(),
            repository_id: "test-repo".to_string(),
            workspace: "test-ws".to_string(),
            kind: WorkspaceEventKind::Created,
            metadata: None,
        });

        // Handle event twice
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();

        // Should still only have one node
        let node = storage
            .nodes()
            .get_by_path("test-tenant", "test-repo", "main", "test-ws", "/folder1")
            .await
            .unwrap();

        assert!(node.is_some());
    }
}
