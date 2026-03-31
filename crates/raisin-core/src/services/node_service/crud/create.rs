//! Create operations for NodeService
//!
//! Contains create and put (deprecated) methods for creating new nodes.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{
    scope::BranchScope, transactional::TransactionalStorage, BranchRepository, NodeRepository,
    NodeTypeRepository, Storage,
};

use super::super::NodeService;
use crate::sanitize_name;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Creates or updates a node.
    ///
    /// This method performs comprehensive validation:
    /// - Validates the NodeType exists
    /// - Validates the node against its NodeType schema
    /// - Sanitizes the node name and path
    /// - Prevents NodeType changes on existing nodes
    ///
    /// # Arguments
    ///
    /// * `node` - The node to create or update
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The NodeType doesn't exist
    /// - Schema validation fails
    /// - An attempt is made to change the NodeType of an existing node
    #[deprecated(
        since = "0.2.0",
        note = "Use create(), update(), or upsert() instead. put() will be removed in a future version."
    )]
    pub async fn put(&self, mut node: models::nodes::Node) -> Result<()> {
        // Track if this is a create or update operation
        let is_new_node = self
            .storage
            .nodes()
            .get(self.scope(), &node.id, self.revision.as_ref())
            .await?
            .is_none();

        tracing::info!(
            node_id = %node.id,
            node_type = %node.node_type,
            path = %node.path,
            workspace_id = %self.workspace_id,
            branch = %self.branch,
            is_new_node = is_new_node,
            "NodeService::put called"
        );

        // CRITICAL: Check if node exists and prevent NodeType changes
        if let Some(existing) = self
            .storage
            .nodes()
            .get(self.scope(), &node.id, self.revision.as_ref())
            .await?
        {
            if existing.node_type != node.node_type {
                return Err(raisin_error::Error::Validation(format!(
                    "Cannot change node_type from '{}' to '{}'. NodeType changes are not allowed.",
                    existing.node_type, node.node_type
                )));
            }
        }

        // Validate NodeType exists and validate node against schema
        // NOTE: RocksDB transaction layer also validates, but InMemoryStorage doesn't,
        // so we validate here as well for consistent behavior across backends.
        self.validator
            .validate_node_type_exists(&node.node_type)
            .await?;
        self.validator
            .validate_node(&self.workspace_id, &node)
            .await?;

        // Validate and sanitize path
        if node.path.is_empty() {
            return Err(raisin_error::Error::Validation(
                "Node path cannot be empty".to_string(),
            ));
        }

        if let Some((parent, leaf)) = node.path.rsplit_once('/') {
            if leaf.is_empty() {
                return Err(raisin_error::Error::Validation(
                    "Node path cannot end with '/'".to_string(),
                ));
            }
            let clean = sanitize_name(leaf)?;
            node.name = clean.clone();
            node.path = if parent.is_empty() || parent == "/" {
                format!("/{}", clean)
            } else {
                format!("{}/{}", parent, clean)
            };
        } else {
            return Err(raisin_error::Error::Validation(
                "Node path must contain at least one '/' separator".to_string(),
            ));
        }
        if let Some(ws) = &node.workspace {
            if ws != &self.workspace_id {
                node.workspace = Some(self.workspace_id.clone());
            }
        } else {
            node.workspace = Some(self.workspace_id.clone());
        }

        // Check if this is a root node based on PATH, not parent field
        // Root nodes have parent_path() = Some("/") or None
        let is_root = match node.parent_path() {
            Some(pp) => pp == "/",
            None => true,
        };

        // Create automatic commit for this operation
        // This triggers tree building and creates a new revision
        let operation = if is_new_node { "Created" } else { "Updated" };
        let commit_message = format!("{} node: {}", operation, node.id);
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        // Set transaction context for revision tracking
        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_message(&commit_message)?;

        tracing::info!(
            node_id = %node.id,
            workspace_id = %self.workspace_id,
            is_new_node = is_new_node,
            "NodeService: Calling transaction put_node"
        );

        // CRITICAL: Put the node inside the transaction so the commit includes it
        ctx.put_node(&self.workspace_id, &node).await?;

        // If this is a root-level node, update the ROOT node's children array
        // CRITICAL: Do this inside the same transaction so the commit includes both updates
        if is_root {
            // Try transaction cache first to get the most recent ROOT state
            let root_node_opt = ctx.get_node_by_path(&self.workspace_id, "/").await?;

            // Fallback to committed storage if not in transaction cache
            let root_node_opt = if root_node_opt.is_none() {
                self.storage
                    .nodes()
                    .get_by_path(self.scope(), "/", self.revision.as_ref())
                    .await?
            } else {
                root_node_opt
            };

            if let Some(mut root_node) = root_node_opt {
                eprintln!(
                    "DEBUG: ROOT node before update: children = {:?}",
                    root_node.children
                );
                if !root_node.children.contains(&node.id) {
                    root_node.children.push(node.id.clone());
                    eprintln!(
                        "DEBUG: ROOT node after update: children = {:?}",
                        root_node.children
                    );
                    // Put ROOT node update in the transaction
                    ctx.put_node(&self.workspace_id, &root_node).await?;
                } else {
                    eprintln!("DEBUG: Node {} already in ROOT children", node.id);
                }
            } else {
                eprintln!(
                    "WARNING: ROOT node not found at path '/' for workspace {}",
                    self.workspace_id
                );
            }
        }

        if let Some(a) = &self.audit {
            a.write(&node, AuditLogAction::Update, None).await?;
        }

        tracing::info!(
            node_id = %node.id,
            is_new_node = is_new_node,
            "NodeService: Committing transaction"
        );

        ctx.commit().await?;

        tracing::info!(
            node_id = %node.id,
            is_new_node = is_new_node,
            "NodeService::put completed successfully"
        );

        // Get the current revision after commit
        let current_revision = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(|| HLC::new(0, 0));

        // Emit NodeCreated or NodeUpdated event
        let event_kind = if is_new_node {
            raisin_storage::NodeEventKind::Created
        } else {
            raisin_storage::NodeEventKind::Updated
        };

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
                kind: event_kind,
                path: Some(node.path.clone()),
                metadata: None,
            }));

        Ok(())
    }

    /// Create a new node
    ///
    /// Use this method to create a new node. Fails if a node with the same ID already exists.
    /// The name and path will be sanitized.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A node with the same ID already exists
    /// - The NodeType doesn't exist
    /// - Schema validation fails
    /// - User doesn't have create permission for the path/node_type
    pub async fn create(&self, mut node: models::nodes::Node) -> Result<()> {
        // Check if node already exists
        if self
            .storage
            .nodes()
            .get(self.scope(), &node.id, self.revision.as_ref())
            .await?
            .is_some()
        {
            return Err(raisin_error::Error::Validation(format!(
                "Node with ID '{}' already exists. Use update() to modify existing nodes.",
                node.id
            )));
        }

        // Validate NodeType exists and validate node against schema
        // NOTE: RocksDB transaction layer also validates, but InMemoryStorage doesn't,
        // so we validate here as well for consistent behavior across backends.
        self.validator
            .validate_node_type_exists(&node.node_type)
            .await?;
        self.validator
            .validate_node(&self.workspace_id, &node)
            .await?;

        // Validate and sanitize path
        if node.path.is_empty() {
            return Err(raisin_error::Error::Validation(
                "Node path cannot be empty".to_string(),
            ));
        }

        if let Some((parent, leaf)) = node.path.rsplit_once('/') {
            if leaf.is_empty() {
                return Err(raisin_error::Error::Validation(
                    "Node path cannot end with '/'".to_string(),
                ));
            }
            let clean = sanitize_name(leaf)?;
            node.name = clean.clone();
            node.path = if parent.is_empty() || parent == "/" {
                format!("/{}", clean)
            } else {
                format!("{}/{}", parent, clean)
            };
        } else {
            return Err(raisin_error::Error::Validation(
                "Node path must contain at least one '/' separator".to_string(),
            ));
        }

        // RLS Authorization: Check if user can create at this path with this node_type
        if !self.check_rls_create_permission(&node.path, &node.node_type) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot create node of type '{}' at path '{}'",
                node.node_type, node.path
            )));
        }

        // Set workspace
        node.workspace = Some(self.workspace_id.clone());

        // Check if this is a root node
        let is_root = match node.parent_path() {
            Some(pp) => pp == "/",
            None => true,
        };

        // Create commit
        let commit_message = format!("Created node: {}", node.id);
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_message(&commit_message)?;

        // Use add_node for create operations (optimized path for new nodes)
        ctx.add_node(&self.workspace_id, &node).await?;

        // Update ROOT children if this is a root-level node
        if is_root {
            if let Some(mut root_node) = self.get_root_node(ctx.as_ref()).await? {
                if !root_node.children.contains(&node.id) {
                    root_node.children.push(node.id.clone());
                    // Use put_node for update since ROOT already exists
                    ctx.put_node(&self.workspace_id, &root_node).await?;
                }
            }
        }

        if let Some(a) = &self.audit {
            a.write(&node, AuditLogAction::Update, None).await?;
        }

        ctx.commit().await?;

        // Auto-create initial_structure children if defined in NodeType
        let node_type = self
            .storage
            .node_types()
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                &node.node_type,
                None,
            )
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("NodeType '{}' not found", node.node_type))
            })?;

        if let Some(initial_structure) = &node_type.initial_structure {
            if let Some(children) = &initial_structure.children {
                // Create all initial children recursively
                match self.create_initial_children(&node, children).await {
                    Ok(_) => {
                        // Successfully created initial children
                    }
                    Err(e) => {
                        // Rollback: delete the parent node since initial children failed
                        let _ = self
                            .storage
                            .nodes()
                            .delete(
                                self.scope(),
                                &node.id,
                                raisin_storage::DeleteNodeOptions::default(),
                            )
                            .await;
                        return Err(raisin_error::Error::Validation(format!(
                            "Failed to create initial_structure children: {}",
                            e
                        )));
                    }
                }
            }
        }

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
                kind: raisin_storage::NodeEventKind::Created,
                path: Some(node.path.clone()),
                metadata: None,
            }));

        Ok(())
    }
}
