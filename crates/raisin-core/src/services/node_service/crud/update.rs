//! Update operations for NodeService
//!
//! Contains update_node and upsert methods for modifying existing nodes.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_models::permissions::Operation;
use raisin_storage::{
    transactional::TransactionalStorage, BranchRepository, NodeRepository, Storage,
};

use super::super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Update an existing node
    ///
    /// Use this method to update an existing node. Fails if the node doesn't exist.
    /// **Important**: The name and path are preserved from the existing node. Use `rename_node()`
    /// to change the name/path.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The node doesn't exist (use create() for new nodes)
    /// - The NodeType doesn't exist
    /// - Schema validation fails
    /// - An attempt is made to change the NodeType
    /// - User doesn't have update permission for the node
    pub async fn update_node(&self, mut node: models::nodes::Node) -> Result<()> {
        // Fetch existing node - required for update
        let existing = self
            .storage
            .nodes()
            .get(self.scope(), &node.id, self.revision.as_ref())
            .await?
            .ok_or_else(|| {
                raisin_error::Error::Validation(format!(
                    "Node with ID '{}' does not exist. Use create() for new nodes.",
                    node.id
                ))
            })?;

        // RLS Authorization: Check if user can update this node
        if !self.check_rls_permission(&existing, Operation::Update) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot update node '{}' at path '{}'",
                node.id, existing.path
            )));
        }

        // Prevent NodeType changes
        if existing.node_type != node.node_type {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot change node_type from '{}' to '{}'. NodeType changes are not allowed.",
                existing.node_type, node.node_type
            )));
        }

        // CRITICAL: Preserve original name and path - use rename_node() to change these
        node.name = existing.name.clone();
        node.path = existing.path.clone();

        // Validate NodeType exists and validate node against schema
        // NOTE: RocksDB transaction layer also validates, but InMemoryStorage doesn't,
        // so we validate here as well for consistent behavior across backends.
        self.validator
            .validate_node_type_exists(&node.node_type)
            .await?;
        self.validator
            .validate_node(&self.workspace_id, &node)
            .await?;

        // Set workspace
        node.workspace = Some(self.workspace_id.clone());

        // Create commit
        let commit_message = format!("Updated node: {}", node.id);
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_message(&commit_message)?;

        ctx.put_node(&self.workspace_id, &node).await?;

        if let Some(a) = &self.audit {
            a.write(&node, AuditLogAction::Update, None).await?;
        }

        ctx.commit().await?;

        // Get current revision and emit event
        let current_revision = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(|| HLC::new(0, 0));

        self.storage
            .event_bus()
            .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                tenant_id: self.tenant_id.clone(),
                repository_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace_id: self.workspace_id.clone(),
                node_id: node.id.clone(),
                node_type: Some(node.node_type.clone()),
                revision: current_revision,
                kind: raisin_storage::NodeEventKind::Updated,
                path: Some(node.path.clone()),
                metadata: None,
            }));

        Ok(())
    }

    /// Create or update a node (upsert)
    ///
    /// Use this method when you need to create a node if it doesn't exist, or update it if it does.
    /// This is useful for operations like `create_folder_if_missing()`.
    ///
    /// For new nodes: name/path will be sanitized.
    /// For existing nodes: name/path are preserved.
    pub async fn upsert(&self, node: models::nodes::Node) -> Result<()> {
        let exists = self
            .storage
            .nodes()
            .get(self.scope(), &node.id, self.revision.as_ref())
            .await?
            .is_some();

        if exists {
            self.update_node(node).await
        } else {
            self.create(node).await
        }
    }
}
