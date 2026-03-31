//! NodeService struct definition and constructors
//!
//! Contains the core NodeService struct and its constructors.

use std::sync::Arc;

use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::services::node_validation::NodeValidator;
use crate::traits::Audit;

/// Service for managing nodes within workspaces.
///
/// Provides CRUD operations, validation, tree management, and publication workflows
/// for nodes. Nodes are validated against NodeType schemas and organized in a
/// hierarchical tree structure within workspaces.
///
/// # Features
///
/// - Node CRUD operations with automatic validation
/// - Tree structure management (move, rename, copy)
/// - Property access and updates via path notation
/// - Publishing/unpublishing workflows
/// - Optional versioning and audit logging
///
/// # Multi-Tenancy
///
/// The new repository-first architecture automatically scopes operations to a specific
/// tenant, repository, branch, and workspace through the context stored in the service.
pub struct NodeService<S: Storage + TransactionalStorage> {
    pub(crate) storage: Arc<S>,
    pub(crate) tenant_id: String,
    pub(crate) repo_id: String,
    pub(crate) branch: String,
    pub(crate) workspace_id: String,
    pub(crate) revision: Option<HLC>, // Optional: view repository at specific revision
    pub(crate) audit: Option<Arc<dyn Audit>>,
    pub(crate) validator: NodeValidator<S>,
    /// Authentication context for permission checks (RLS, field-level security)
    pub(crate) auth_context: Option<AuthContext>,
}

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Create a new NodeService with repository context
    ///
    /// This constructor is used internally by NodeServiceBuilder to create
    /// a context-aware service instance.
    pub fn new_with_context(
        storage: Arc<S>,
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
    ) -> Self {
        let validator = NodeValidator::new(
            storage.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
        );
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            revision: None, // Default to HEAD
            audit: None,
            validator,
            auth_context: None,
        }
    }

    /// Create a new NodeService in single-tenant mode (DEPRECATED)
    ///
    /// This is the old constructor for embedded usage. Use the connection API instead:
    /// ```rust,ignore
    /// let conn = RaisinConnection::with_storage(storage);
    /// let service = conn.tenant("default").repository("app").workspace("main").nodes();
    /// ```
    #[deprecated(note = "Use RaisinConnection API instead")]
    pub fn new(storage: Arc<S>) -> Self {
        let validator = NodeValidator::new(
            storage.clone(),
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
        );
        Self {
            storage,
            tenant_id: "default".to_string(),
            repo_id: "default".to_string(),
            branch: "main".to_string(),
            workspace_id: "default".to_string(),
            revision: None, // Default to HEAD
            audit: None,
            validator,
            auth_context: None,
        }
    }

    /// Configures optional audit logging.
    ///
    /// When audit logging is enabled, all node operations will be logged for
    /// compliance and tracking purposes.
    pub fn with_audit(mut self, a: Arc<dyn Audit>) -> Self {
        self.audit = Some(a);
        self
    }

    /// Set the authentication context for permission enforcement.
    ///
    /// When set, all operations will have RLS (row-level security) and
    /// field-level security enforced based on the user's permissions.
    ///
    /// # Arguments
    ///
    /// * `auth` - The authentication context containing user identity and resolved permissions
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let service = node_service
    ///     .with_auth(AuthContext::for_user("user123").with_permissions(resolved));
    /// ```
    pub fn with_auth(mut self, auth: AuthContext) -> Self {
        self.auth_context = Some(auth);
        self
    }

    /// Get the current authentication context (if set).
    pub fn auth_context(&self) -> Option<&AuthContext> {
        self.auth_context.as_ref()
    }

    /// Get the current storage scope as a `StorageScope` struct.
    ///
    /// Useful for passing the (tenant, repo, branch, workspace) tuple
    /// to functions that accept `StorageScope` instead of four separate args.
    pub fn scope(&self) -> raisin_storage::StorageScope<'_> {
        raisin_storage::StorageScope::new(
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            &self.workspace_id,
        )
    }

    /// Get the branch-level scope (tenant, repo, branch).
    pub fn branch_scope(&self) -> raisin_storage::BranchScope<'_> {
        raisin_storage::BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch)
    }

    /// Get a reference to the underlying storage
    ///
    /// This is useful for advanced operations and testing scenarios where
    /// direct storage access is needed.
    pub fn storage(&self) -> &Arc<S> {
        &self.storage
    }
}
