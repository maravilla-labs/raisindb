use std::sync::Arc;

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{scope::RepoScope, BranchRepository, Storage, WorkspaceRepository};

use super::transaction::Transaction;
use crate::workspace_structure_init::create_workspace_initial_structure;

pub struct WorkspaceService<S: Storage> {
    pub storage: Arc<S>,
}

impl<S: Storage> WorkspaceService<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }

    pub async fn get(
        &self,
        tenant_id: &str,
        repo_id: &str,
        name: &str,
    ) -> Result<Option<models::workspace::Workspace>> {
        self.storage
            .workspaces()
            .get(RepoScope::new(tenant_id, repo_id), name)
            .await
    }

    pub async fn put(
        &self,
        tenant_id: &str,
        repo_id: &str,
        mut ws: models::workspace::Workspace,
    ) -> Result<()>
    where
        S: raisin_storage::transactional::TransactionalStorage,
    {
        if ws.name.is_empty() {
            ws.name = "default".to_string();
        }

        // Check if this is a new workspace by trying to get it first
        let is_new = self
            .storage
            .workspaces()
            .get(RepoScope::new(tenant_id, repo_id), &ws.name)
            .await?
            .is_none();

        // Save the workspace
        self.storage
            .workspaces()
            .put(RepoScope::new(tenant_id, repo_id), ws.clone())
            .await?;

        // Bootstrap ROOT node for new workspaces
        if is_new {
            let branch = ws.config.default_branch.clone();

            // Check if this branch was created from an existing revision
            // If so, ROOT node already exists in the tree snapshot - don't create it
            let should_create_root = if let Some(branch_info) = self
                .storage
                .branches()
                .get_branch(tenant_id, repo_id, &branch)
                .await?
            {
                // Branch exists - check if it's pristine (created from revision)
                let is_pristine = branch_info.created_from.is_some();

                if is_pristine {
                    tracing::info!(
                        "Skipping ROOT node creation for workspace '{}' on pristine branch '{}' (created from revision {:?})",
                        ws.name, branch, branch_info.created_from
                    );
                    false // Skip ROOT creation - it exists in tree snapshot
                } else {
                    tracing::info!(
                        "Creating ROOT node for workspace '{}' on scratch branch '{}'",
                        ws.name,
                        branch
                    );
                    true // Create ROOT - this is a from-scratch branch
                }
            } else {
                // Branch doesn't exist yet - this is likely initial repository setup
                tracing::info!(
                    "Creating ROOT node for workspace '{}' (branch '{}' will be created)",
                    ws.name,
                    branch
                );
                true // Create ROOT - branch will be created during commit
            };

            if should_create_root {
                let root_node = models::nodes::Node {
                    id: models::nodes::ROOT_NODE_ID.to_string(),
                    name: "root".to_string(),
                    path: "/".to_string(),
                    parent: None,
                    node_type: "raisin:Folder".to_string(),
                    children: Vec::new(),
                    order_key: String::new(), // ROOT has no order_key (no siblings)
                    has_children: None,       // Computed at service layer
                    properties: std::collections::HashMap::new(),
                    archetype: None,
                    created_at: Some(chrono::Utc::now()),
                    updated_at: None,
                    created_by: Some("system".to_string()),
                    updated_by: None,
                    published_at: None,
                    published_by: None,
                    version: 1,
                    translations: None,
                    tenant_id: None,
                    workspace: None,
                    owner_id: None,
                    relations: Vec::new(),
                };

                // Create ROOT node in a transaction to establish rev0
                // This ensures the root node is part of the revision history
                let mut tx = Transaction::new(
                    self.storage.clone(),
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    branch,
                    ws.name.clone(),
                );

                tx.create(root_node);

                // Commit as rev0 with system actor
                tx.commit("system: initialize workspace", "system").await?;
            }

            // Initialize workspace initial_structure synchronously
            // This ensures the workspace is fully ready before returning to the client
            if let Err(e) = create_workspace_initial_structure(
                self.storage.clone(),
                tenant_id,
                repo_id,
                &ws.name,
            )
            .await
            {
                tracing::warn!(
                    "Failed to initialize workspace structure for '{}': {}",
                    ws.name,
                    e
                );
                // Continue even if structure init fails - workspace is still valid
                // The error is logged for debugging purposes
            }
        }

        Ok(())
    }

    pub async fn list(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Vec<models::workspace::Workspace>> {
        self.storage
            .workspaces()
            .list(RepoScope::new(tenant_id, repo_id))
            .await
    }
}
