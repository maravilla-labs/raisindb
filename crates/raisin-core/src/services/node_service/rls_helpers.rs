//! RLS (Row-Level Security) helper methods for NodeService
//!
//! Contains methods for permission checking and filtering based on authentication context.

use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::permissions::{Operation, PermissionScope};
use raisin_rel::eval::RelationResolver;
use raisin_storage::scope::BranchScope;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use super::NodeService;
use crate::services::rls_filter;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Get the permission scope for this service context.
    ///
    /// The scope includes the current workspace and branch for scope-aware permission checks.
    pub(crate) fn permission_scope(&self) -> PermissionScope {
        PermissionScope::new(&self.workspace_id, &self.branch)
    }

    /// The revision at which RLS graph-relationship conditions are evaluated.
    ///
    /// Uses the service's pinned revision (time-travel reads) when set, otherwise
    /// the current HLC (latest committed state).
    pub(crate) fn rls_revision(&self) -> HLC {
        self.revision.unwrap_or_else(HLC::now)
    }

    /// Build a graph relationship resolver for `RELATES … VIA` conditions,
    /// scoped to this service's branch and the given `revision`. Returns `None`
    /// for storage backends without graph support (RELATES then fails closed).
    pub(crate) fn rls_resolver<'a>(
        &'a self,
        revision: &'a HLC,
    ) -> Option<Box<dyn RelationResolver + 'a>> {
        let scope = BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch);
        self.storage.graph_resolver(scope, revision)
    }

    /// Apply RLS filtering to a single node.
    ///
    /// Returns None if the user doesn't have permission to read the node.
    pub(crate) async fn apply_rls_filter(
        &self,
        node: models::nodes::Node,
    ) -> Option<models::nodes::Node> {
        match &self.auth_context {
            // Fast path: no graph (`RELATES`) conditions → synchronous, no resolver.
            Some(auth) if !auth.uses_graph_rls() => {
                rls_filter::filter_node(node, auth, &self.permission_scope())
            }
            Some(auth) => {
                let rev = self.rls_revision();
                let resolver = self.rls_resolver(&rev);
                rls_filter::filter_node_async(
                    node,
                    auth,
                    &self.permission_scope(),
                    resolver.as_deref(),
                )
                .await
            }
            None => {
                // SECURITY: Deny access when no auth context is set.
                // Use AuthContext::system() explicitly for admin/system operations.
                tracing::warn!(
                    node_id = %node.id,
                    path = ?node.path,
                    "RLS: No auth context set - denying access"
                );
                None
            }
        }
    }

    /// Apply RLS filtering to multiple nodes.
    ///
    /// Filters out nodes the user doesn't have permission to read.
    pub(crate) async fn apply_rls_filter_many(
        &self,
        nodes: Vec<models::nodes::Node>,
    ) -> Vec<models::nodes::Node> {
        match &self.auth_context {
            // Fast path: no graph (`RELATES`) conditions → synchronous, no resolver.
            Some(auth) if !auth.uses_graph_rls() => {
                rls_filter::filter_nodes(nodes, auth, &self.permission_scope())
            }
            Some(auth) => {
                let rev = self.rls_revision();
                let resolver = self.rls_resolver(&rev);
                rls_filter::filter_nodes_async(
                    nodes,
                    auth,
                    &self.permission_scope(),
                    resolver.as_deref(),
                )
                .await
            }
            None => {
                // SECURITY: Deny access when no auth context is set.
                // Use AuthContext::system() explicitly for admin/system operations.
                if !nodes.is_empty() {
                    tracing::warn!(
                        count = nodes.len(),
                        "RLS: No auth context set - denying access to all nodes"
                    );
                }
                vec![]
            }
        }
    }

    /// Check if user can perform an operation on a node.
    pub(crate) async fn check_rls_permission(
        &self,
        node: &models::nodes::Node,
        operation: Operation,
    ) -> bool {
        match &self.auth_context {
            // Fast path: no graph (`RELATES`) conditions → synchronous, no resolver.
            Some(auth) if !auth.uses_graph_rls() => {
                rls_filter::can_perform(node, operation, auth, &self.permission_scope())
            }
            Some(auth) => {
                let rev = self.rls_revision();
                let resolver = self.rls_resolver(&rev);
                rls_filter::can_perform_async(
                    node,
                    operation,
                    auth,
                    &self.permission_scope(),
                    resolver.as_deref(),
                )
                .await
            }
            None => {
                // SECURITY: Deny operations when no auth context is set.
                // Use AuthContext::system() explicitly for admin/system operations.
                tracing::warn!(
                    node_id = %node.id,
                    operation = ?operation,
                    "RLS: No auth context set - denying operation"
                );
                false
            }
        }
    }

    /// Check if user can create a node at a path.
    pub(crate) async fn check_rls_create_permission(&self, path: &str, node_type: &str) -> bool {
        match &self.auth_context {
            // Fast path: no graph (`RELATES`) conditions → synchronous, no resolver.
            Some(auth) if !auth.uses_graph_rls() => {
                rls_filter::can_create_at_path(path, node_type, auth, &self.permission_scope())
            }
            Some(auth) => {
                let rev = self.rls_revision();
                let resolver = self.rls_resolver(&rev);
                rls_filter::can_create_at_path_async(
                    path,
                    node_type,
                    auth,
                    &self.permission_scope(),
                    resolver.as_deref(),
                )
                .await
            }
            None => {
                // SECURITY: Deny creation when no auth context is set.
                // Use AuthContext::system() explicitly for admin/system operations.
                tracing::warn!(
                    path = path,
                    node_type = node_type,
                    "RLS: No auth context set - denying create"
                );
                false
            }
        }
    }
}
