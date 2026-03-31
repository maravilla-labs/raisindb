//! RLS (Row-Level Security) helper methods for NodeService
//!
//! Contains methods for permission checking and filtering based on authentication context.

use raisin_models as models;
use raisin_models::permissions::{Operation, PermissionScope};
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

    /// Apply RLS filtering to a single node.
    ///
    /// Returns None if the user doesn't have permission to read the node.
    pub(crate) fn apply_rls_filter(
        &self,
        node: models::nodes::Node,
    ) -> Option<models::nodes::Node> {
        match &self.auth_context {
            Some(auth) => rls_filter::filter_node(node, auth, &self.permission_scope()),
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
    pub(crate) fn apply_rls_filter_many(
        &self,
        nodes: Vec<models::nodes::Node>,
    ) -> Vec<models::nodes::Node> {
        match &self.auth_context {
            Some(auth) => rls_filter::filter_nodes(nodes, auth, &self.permission_scope()),
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
    pub(crate) fn check_rls_permission(
        &self,
        node: &models::nodes::Node,
        operation: Operation,
    ) -> bool {
        match &self.auth_context {
            Some(auth) => rls_filter::can_perform(node, operation, auth, &self.permission_scope()),
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
    pub(crate) fn check_rls_create_permission(&self, path: &str, node_type: &str) -> bool {
        match &self.auth_context {
            Some(auth) => {
                rls_filter::can_create_at_path(path, node_type, auth, &self.permission_scope())
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
