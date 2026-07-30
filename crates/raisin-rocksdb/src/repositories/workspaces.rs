//! Workspace repository implementation

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::{Event, EventBus, WorkspaceEvent, WorkspaceEventKind};
use raisin_models::timestamp::StorageTimestamp;
use raisin_models::workspace::Workspace;
use raisin_storage::scope::RepoScope;
use raisin_storage::WorkspaceRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct WorkspaceRepositoryImpl {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
    /// Present only when replication is wired in. Without it a workspace record —
    /// and with it every workspace config, including the spatial index policy —
    /// is local to the node it was written on.
    operation_capture: Option<Arc<crate::OperationCapture>>,
}

impl WorkspaceRepositoryImpl {
    pub fn new(db: Arc<DB>, event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: None,
        }
    }

    /// Build a workspace repository that captures its writes for replication.
    ///
    /// `OperationCapture` is constructed after the repositories in
    /// `storage/init.rs`, so this is the same late-binding constructor the branch
    /// and schema repositories use rather than a setter.
    pub fn new_with_capture(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: Some(operation_capture),
        }
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

    /// Write a workspace record.
    ///
    /// Two things happen here that callers must not duplicate or work around:
    ///
    /// 1. **`created_at` / `updated_at` are stamped here**, at the single
    ///    low-level write path, the same way node authorship and timestamps are.
    ///    An update preserves the stored `created_at` and always advances
    ///    `updated_at`. This is not cosmetic: `updated_at` is the comparator the
    ///    replication applier uses to reject an out-of-order older record, so a
    ///    record that went out unstamped would be unorderable on its peers.
    /// 2. **The write is captured for replication** when replication is wired in.
    ///    Without it a workspace config change is silently local to one node.
    async fn put(&self, scope: RepoScope<'_>, ws: Workspace) -> Result<()> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let key = keys::workspace_key(tenant_id, repo_id, &ws.name);

        // Check if workspace already exists to determine event type
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;
        let existing = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        let is_new = existing.is_none();

        let mut ws = ws;
        if let Some(bytes) = existing {
            // Preserve the original creation timestamp; a client-supplied
            // created_at must not be able to rewrite history.
            if let Ok(previous) = rmp_serde::from_slice::<Workspace>(&bytes) {
                ws.created_at = previous.created_at;
            }
            ws.updated_at = Some(StorageTimestamp::now());
        }

        // Use to_vec_named to maintain compatibility with custom deserializers
        // that expect named fields (e.g., InitialNodeStructure, InitialChild)
        let value = rmp_serde::to_vec_named(&ws)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture for replication. Errors are logged, not propagated: a
        // replication hiccup must not fail a durable local write, which matches
        // every other capture call site.
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let branch = ws.config.default_branch.clone();
                if let Err(e) = capture
                    .capture_update_workspace(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch,
                        ws.name.clone(),
                        ws.clone(),
                        "system".to_string(),
                    )
                    .await
                {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        workspace = %ws.name,
                        error = %e,
                        "Failed to capture UpdateWorkspace operation; the workspace \
                         config will not reach peers until it is written again"
                    );
                }
            }
        }

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
