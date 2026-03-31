//! NodeType initialization event handler
//!
//! Listens for RepositoryCreated events and automatically initializes
//! all built-in NodeTypes from embedded YAML files for new repositories.

use anyhow::Result;
use raisin_storage::{
    scope::RepoScope, transactional::TransactionalStorage, Event, EventHandler, Storage,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Event handler that initializes NodeTypes when a repository is created
pub struct NodeTypeInitHandler<S: Storage + TransactionalStorage> {
    storage: Arc<S>,
}

impl<S: Storage + TransactionalStorage> NodeTypeInitHandler<S> {
    /// Create a new NodeTypeInitHandler
    pub fn new(storage: Arc<S>) -> Self {
        tracing::info!("NodeTypeInitHandler created and ready to handle events");
        Self { storage }
    }

    /// Initialize built-in NodeTypes for a repository
    async fn init_nodetypes_for_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<()> {
        // Use the centralized nodetype initialization from raisin-core
        raisin_core::nodetype_init::init_repository_nodetypes(
            self.storage.clone(),
            tenant_id,
            repo_id,
            branch,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to initialize NodeTypes: {}", e))?;

        tracing::info!(
            "✓ NodeTypes initialized for {}/{}, now initializing workspace structures",
            tenant_id,
            repo_id
        );

        // After NodeTypes are initialized, create initial_structure for all workspaces
        // This ensures NodeTypes exist before we try to create nodes
        self.init_workspace_structures(tenant_id, repo_id).await?;

        Ok(())
    }

    /// Initialize workspace structures after NodeTypes are ready
    async fn init_workspace_structures(&self, tenant_id: &str, repo_id: &str) -> Result<()> {
        use raisin_storage::WorkspaceRepository;

        // Load all workspaces for this repository
        let workspace_repo = self.storage.workspaces();
        let workspaces = workspace_repo
            .list(RepoScope::new(tenant_id, repo_id))
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to list workspaces for {}/{}: {}",
                    tenant_id,
                    repo_id,
                    e
                )
            })?;

        tracing::info!(
            "Creating initial_structure for {} workspace(s) in {}/{}",
            workspaces.len(),
            tenant_id,
            repo_id
        );

        // Initialize initial_structure for each workspace
        for workspace in workspaces {
            if let Err(e) =
                raisin_core::workspace_structure_init::create_workspace_initial_structure(
                    self.storage.clone(),
                    tenant_id,
                    repo_id,
                    &workspace.name,
                )
                .await
            {
                tracing::error!(
                    "Failed to create initial_structure for workspace {}/{}/{}: {}",
                    tenant_id,
                    repo_id,
                    workspace.name,
                    e
                );
                // Continue with other workspaces even if one fails
            }
        }

        Ok(())
    }
}

impl<S: Storage + TransactionalStorage + 'static> EventHandler for NodeTypeInitHandler<S> {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::debug!("=== NodeTypeInitHandler.handle() called ===");
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
            tracing::debug!("Full event: {:?}", event);

            // Only handle RepositoryCreated events
            if let Event::Repository(repo_event) = event {
                tracing::debug!("✓ Event is Repository event");
                tracing::debug!("  Repository event kind: {:?}", repo_event.kind);
                tracing::debug!(
                    "  Tenant: {}, Repo: {}",
                    repo_event.tenant_id,
                    repo_event.repository_id
                );
                tracing::debug!("  Branch: {:?}", repo_event.branch_name);

                if matches!(
                    repo_event.kind,
                    raisin_storage::RepositoryEventKind::Created
                ) {
                    tracing::info!(
                        "✓ Processing RepositoryCreated event for {}/{}",
                        repo_event.tenant_id,
                        repo_event.repository_id
                    );

                    // Initialize NodeTypes for the new repository
                    // Use the default branch from the event or fall back to "main"
                    let branch = repo_event.branch_name.as_deref().unwrap_or("main");

                    tracing::debug!("Will initialize NodeTypes on branch: {}", branch);

                    if let Err(e) = self
                        .init_nodetypes_for_repository(
                            &repo_event.tenant_id,
                            &repo_event.repository_id,
                            branch,
                        )
                        .await
                    {
                        tracing::error!(
                            "Failed to initialize NodeTypes for repository {}/{}: {}",
                            repo_event.tenant_id,
                            repo_event.repository_id,
                            e
                        );
                        return Err(e);
                    }
                }
            }

            Ok(())
        })
    }

    fn name(&self) -> &str {
        "NodeTypeInitHandler"
    }
}

#[cfg(all(test, feature = "store-memory"))]
mod tests {
    use super::*;
    use raisin_hlc::HLC;
    use raisin_storage::{NodeEventKind, NodeRepository, RepositoryEvent, RepositoryEventKind};
    use raisin_storage_memory::InMemoryStorage;

    #[tokio::test]
    async fn test_nodetype_init_on_repository_created() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = NodeTypeInitHandler::new(storage.clone());

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

        // Verify built-in content NodeTypes were created
        let repo = storage.node_types();
        let folder = repo
            .get("test-tenant", "test-repo", "main", "raisin:Folder", None)
            .await
            .unwrap();
        assert!(folder.is_some(), "raisin:Folder should be initialized");

        let page = repo
            .get("test-tenant", "test-repo", "main", "raisin:Page", None)
            .await
            .unwrap();
        assert!(page.is_some(), "raisin:Page should be initialized");

        let asset = repo
            .get("test-tenant", "test-repo", "main", "raisin:Asset", None)
            .await
            .unwrap();
        assert!(asset.is_some(), "raisin:Asset should be initialized");

        // Verify access control NodeTypes were created
        let user = repo
            .get("test-tenant", "test-repo", "main", "raisin:User", None)
            .await
            .unwrap();
        assert!(user.is_some(), "raisin:User should be initialized");

        let role = repo
            .get("test-tenant", "test-repo", "main", "raisin:Role", None)
            .await
            .unwrap();
        assert!(role.is_some(), "raisin:Role should be initialized");

        let group = repo
            .get("test-tenant", "test-repo", "main", "raisin:Group", None)
            .await
            .unwrap();
        assert!(group.is_some(), "raisin:Group should be initialized");
    }

    #[tokio::test]
    async fn test_ignores_non_repository_events() {
        let storage = Arc::new(InMemoryStorage::default());
        let handler = NodeTypeInitHandler::new(storage.clone());

        // Create a NodeEvent (should be ignored)
        let event = Event::Node(raisin_storage::NodeEvent {
            tenant_id: "test".to_string(),
            repository_id: "test".to_string(),
            branch: "main".to_string(),
            workspace_id: "test-workspace".to_string(),
            node_id: "node1".to_string(),
            kind: NodeEventKind::Created,
            node_type: None,
            path: None,
            revision: HLC::new(0, 0),
            metadata: None,
        });

        // Should not error, just ignore
        assert!(handler.handle(&event).await.is_ok());

        // Verify no NodeTypes were created
        let repo = storage.node_types();
        let folder = repo
            .get("test", "test", "main", "raisin:Folder", None)
            .await
            .unwrap();
        assert!(folder.is_none());
    }
}
