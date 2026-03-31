//! Workspace repository implementation

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::{Event, EventBus, WorkspaceEvent, WorkspaceEventKind};
use raisin_models::workspace::Workspace;
use raisin_storage::scope::RepoScope;
use raisin_storage::WorkspaceRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct WorkspaceRepositoryImpl {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
}

impl WorkspaceRepositoryImpl {
    pub fn new(db: Arc<DB>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }
}

impl WorkspaceRepository for WorkspaceRepositoryImpl {
    async fn get(&self, scope: RepoScope<'_>, id: &str) -> Result<Option<Workspace>> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let key = keys::workspace_key(tenant_id, repo_id, id);
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let workspace = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(workspace))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn put(&self, scope: RepoScope<'_>, ws: Workspace) -> Result<()> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let key = keys::workspace_key(tenant_id, repo_id, &ws.name);

        // Check if workspace already exists to determine event type
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;
        let is_new = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_none();

        // Use to_vec_named to maintain compatibility with custom deserializers
        // that expect named fields (e.g., InitialNodeStructure, InitialChild)
        let value = rmp_serde::to_vec_named(&ws)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

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

    async fn list(&self, scope: RepoScope<'_>) -> Result<Vec<Workspace>> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("workspaces")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::WORKSPACES)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut workspaces = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }
            let workspace: Workspace = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            workspaces.push(workspace);
        }

        Ok(workspaces)
    }
}
