use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use raisin_error::Result;
use raisin_events::{Event, WorkspaceEvent, WorkspaceEventKind};
use raisin_models as models;
use raisin_storage::scope::RepoScope;
use raisin_storage::{EventBus, WorkspaceRepository};

#[derive(Clone)]
pub struct InMemoryWorkspaceRepo {
    // key: "tenant_id:repo_id:workspace_name"
    pub(crate) workspaces: Arc<RwLock<HashMap<String, models::workspace::Workspace>>>,
    event_bus: Arc<dyn EventBus>,
}

impl InMemoryWorkspaceRepo {
    pub fn new(event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            workspaces: Default::default(),
            event_bus,
        }
    }

    fn make_key(tenant_id: &str, repo_id: &str, name: &str) -> String {
        format!("{}:{}:{}", tenant_id, repo_id, name)
    }

    fn make_prefix(tenant_id: &str, repo_id: &str) -> String {
        format!("{}:{}:", tenant_id, repo_id)
    }
}

impl WorkspaceRepository for InMemoryWorkspaceRepo {
    fn get(
        &self,
        scope: RepoScope<'_>,
        id: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::workspace::Workspace>>> + Send
    {
        let key = Self::make_key(scope.tenant_id, scope.repo_id, id);
        async move {
            let map = self.workspaces.read().await;
            Ok(map.get(&key).cloned())
        }
    }

    async fn put(&self, scope: RepoScope<'_>, ws: models::workspace::Workspace) -> Result<()> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let key = Self::make_key(tenant_id, repo_id, &ws.name);
        let mut map = self.workspaces.write().await;
        let is_new = !map.contains_key(&key);
        map.insert(key, ws.clone());
        drop(map);

        // Emit WorkspaceCreated or WorkspaceUpdated event
        let event = if is_new {
            Event::Workspace(WorkspaceEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                workspace: ws.name.clone(),
                kind: WorkspaceEventKind::Created,
                metadata: None,
            })
        } else {
            Event::Workspace(WorkspaceEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                workspace: ws.name.clone(),
                kind: WorkspaceEventKind::Updated,
                metadata: None,
            })
        };
        self.event_bus.publish(event);
        Ok(())
    }

    async fn list(&self, scope: RepoScope<'_>) -> Result<Vec<models::workspace::Workspace>> {
        let prefix = Self::make_prefix(scope.tenant_id, scope.repo_id);
        let map = self.workspaces.read().await;
        Ok(map
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v.clone())
            .collect())
    }
}
