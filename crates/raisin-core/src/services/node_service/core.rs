//! NodeService struct definition and constructors
//!
//! Contains the core NodeService struct and its constructors.

use std::sync::Arc;

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{
    transactional::TransactionalStorage, BranchScope, NodeTypeRepository, Storage,
};

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

    /// Write an audit-log entry for `node`, gated on the NodeType `auditable`
    /// flag.
    ///
    /// Audit logging happens only when (a) an audit sink is configured AND
    /// (b) the node's NodeType is marked `auditable = true`. The flag is the
    /// opt-in switch — non-auditable types produce no audit-log entries.
    ///
    /// Note: this is independent of revision history. The structural MVCC
    /// version history (see [`Self::history`]) is always available regardless
    /// of `auditable`.
    ///
    /// If the NodeType can't be resolved, auditing is skipped rather than
    /// failing the surrounding write.
    pub(crate) async fn audit_write(
        &self,
        node: &raisin_models::nodes::Node,
        action: AuditLogAction,
        details: Option<String>,
    ) -> Result<()> {
        let Some(audit) = &self.audit else {
            return Ok(());
        };

        let auditable = self
            .storage
            .node_types()
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                &node.node_type,
                None,
            )
            .await
            .ok()
            .flatten()
            .map(|nt| nt.auditable())
            .unwrap_or(false);

        if auditable {
            let scope = raisin_audit::AuditScope::new(
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &self.workspace_id,
            );
            let agent = self.auth_context.as_ref().and_then(|c| c.agent_id());
            audit.write(scope, node, action, details, agent).await?;
        }
        Ok(())
    }

    /// The actor string to stamp on a transaction's commit metadata.
    ///
    /// Prefers the caller's real identity over a hardcoded label so per-node
    /// revision history attributes a change to the user who made it. Falls back
    /// to `"system"` only when no auth context is present — which commit itself
    /// rejects, so in practice this is unreachable on a committing path.
    pub(crate) fn commit_actor(&self) -> String {
        self.auth_context
            .as_ref()
            .map(|c| c.actor_id())
            .unwrap_or_else(|| "system".to_string())
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

    /// Validate a node against its NodeType and stamp the materialized
    /// effective-mixin / supertype membership sets used by `has_mixin()` /
    /// `is_a()`.
    ///
    /// This is the single write-path choke point: it checks the NodeType exists,
    /// runs full schema validation, and — reusing the same resolution — writes the
    /// reserved `$mixins` / `$supertypes` properties onto the node. Any
    /// client-supplied reserved properties are stripped first so they can't be
    /// spoofed.
    pub(crate) async fn validate_and_stamp(
        &self,
        node: &mut raisin_models::nodes::Node,
    ) -> Result<()> {
        // Never trust client-supplied membership sets.
        node.strip_reserved_properties();

        self.validator
            .validate_node_type_exists(&node.node_type)
            .await?;
        let resolved = self
            .validator
            .validate_node_resolved(&self.workspace_id, node)
            .await?;

        let supertypes = resolved.effective_supertypes();
        node.set_effective_types(resolved.resolved_mixins.clone(), supertypes);
        Ok(())
    }
}
