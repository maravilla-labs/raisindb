//! Workspace initialization event handler
//!
//! Listens for RepositoryCreated events and automatically initializes
//! built-in workspaces (e.g., "default") for new repositories.

use anyhow::Result;
use raisin_core::workspace_init::init_repository_workspaces;
use raisin_storage::{Event, EventHandler, Storage};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Event handler that initializes workspaces when a repository is created
pub struct WorkspaceInitHandler<S: Storage> {
    storage: Arc<S>,
}

impl<S: Storage> WorkspaceInitHandler<S> {
    /// Create a new WorkspaceInitHandler
    pub fn new(storage: Arc<S>) -> Self {
        tracing::info!("WorkspaceInitHandler created and ready to handle events");
        Self { storage }
    }
}

impl<S: Storage + 'static> EventHandler for WorkspaceInitHandler<S> {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::debug!("=== WorkspaceInitHandler.handle() called ===");
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

            // Only handle RepositoryCreated events
            if let Event::Repository(repo_event) = event {
                tracing::debug!("✓ Event is Repository event");
                tracing::debug!("  Repository event kind: {:?}", repo_event.kind);
                tracing::debug!(
                    "  Tenant: {}, Repo: {}",
                    repo_event.tenant_id,
                    repo_event.repository_id
                );

                if matches!(
                    repo_event.kind,
                    raisin_storage::RepositoryEventKind::Created
                ) {
                    tracing::info!(
                        "✓ Processing RepositoryCreated event for {}/{}",
                        repo_event.tenant_id,
                        repo_event.repository_id
                    );

                    // Initialize workspaces for the new repository
                    if let Err(e) = init_repository_workspaces(
                        self.storage.clone(),
                        &repo_event.tenant_id,
                        &repo_event.repository_id,
                    )
                    .await
                    {
                        tracing::error!(
                            "Failed to initialize workspaces for repository {}/{}: {}",
                            repo_event.tenant_id,
                            repo_event.repository_id,
                            e
                        );
                        return Err(e.into());
                    }
                }
            }

            Ok(())
        })
    }

    fn name(&self) -> &str {
        "WorkspaceInitHandler"
    }
}

#[cfg(all(test, feature = "store-memory"))]
mod tests {
    use super::*;
    use raisin_hlc::HLC;
    use raisin_storage::{RepositoryEvent, RepositoryEventKind, WorkspaceRepository};
    use raisin_storage_memory::InMemoryStorage;

    #[tokio::test]
    async fn test_workspace_init_on_repository_created() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = WorkspaceInitHandler::new(storage.clone());

        // Simulate RepositoryCreated event
        let event = Event::Repository(RepositoryEvent {
            tenant_id: "test-tenant".to_string(),
            repository_id: "test-repo".to_string(),
            kind: RepositoryEventKind::Created,
            workspace: None,
            revision_id: None,
            branch_name: Some("main".to_string()),
            tag_name: None,
            message: None,
            actor: None,
            metadata: None,
        });

        // Handle the event
        handler.handle(&event).await.unwrap();

        // Verify default workspace was created
        let workspace_repo = storage.workspaces();
        let default_ws = workspace_repo
            .get("test-tenant", "test-repo", "default")
            .await
            .unwrap();

        assert!(default_ws.is_some());
        let ws = default_ws.unwrap();
        assert_eq!(ws.name, "default");
        assert!(ws.allowed_node_types.contains(&"raisin:Folder".to_string()));
    }

    #[tokio::test]
    async fn test_ignores_non_repository_events() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = WorkspaceInitHandler::new(storage.clone());

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

        // Verify no workspaces were created
        let workspace_repo = storage.workspaces();
        let default_ws = workspace_repo.get("test", "test", "default").await.unwrap();
        assert!(default_ws.is_none());
    }

    #[tokio::test]
    async fn test_idempotent_initialization() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = WorkspaceInitHandler::new(storage.clone());

        let event = Event::Repository(RepositoryEvent {
            tenant_id: "test-tenant".to_string(),
            repository_id: "test-repo".to_string(),
            kind: RepositoryEventKind::Created,
            workspace: None,
            revision_id: None,
            branch_name: Some("main".to_string()),
            tag_name: None,
            message: None,
            actor: None,
            metadata: None,
        });

        // Handle event twice
        handler.handle(&event).await.unwrap();
        handler.handle(&event).await.unwrap();

        // Should still only have one workspace
        let workspace_repo = storage.workspaces();
        let default_ws = workspace_repo
            .get("test-tenant", "test-repo", "default")
            .await
            .unwrap();

        assert!(default_ws.is_some());
    }
}
