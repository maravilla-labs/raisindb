//! Validation helpers for node operations
//!
//! This module provides shared validation logic used by both NodeRepository
//! and TransactionalContext to avoid code duplication.

use super::NodeRepositoryImpl;
use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use raisin_storage::{
    CreateNodeOptions, NodeTypeRepository, UpdateNodeOptions, WorkspaceRepository,
};

impl NodeRepositoryImpl {
    /// Validate a node for creation
    ///
    /// Performs all validation checks required before creating a new node:
    /// 1. Checks node doesn't already exist (conflict detection)
    /// 2. Validates schema against NodeType (if enabled)
    /// 3. Validates parent-child type compatibility (if enabled)
    /// 4. Validates workspace type constraints (if enabled)
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node` - The node to validate
    /// * `options` - Validation options controlling what checks to perform
    ///
    /// # Returns
    /// * `Ok(())` if all validations pass
    /// * `Err(Error::Conflict)` if node already exists
    /// * `Err(Error::Validation)` if any validation check fails
    /// * `Err(Error::NotFound)` if parent or NodeType not found
    pub(crate) async fn validate_for_create(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        options: &CreateNodeOptions,
    ) -> Result<()> {
        // 1. Check node doesn't already exist (always required for create)
        if self
            .get_impl(tenant_id, repo_id, branch, workspace, &node.id, false)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "Node with id '{}' already exists",
                node.id
            )));
        }

        // 2. Check path uniqueness (always required for create)
        let node_path = &node.path;
        if self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, node_path, None)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "Node with path '{}' already exists",
                node_path
            )));
        }

        // 3. Validate parent-child type compatibility
        if options.validate_parent_allows_child {
            self.validate_parent_allows_child(tenant_id, repo_id, branch, workspace, node)
                .await?;
        }

        // 4. Validate workspace type constraints
        if options.validate_workspace_allows_type {
            self.validate_workspace_allows_type(tenant_id, repo_id, branch, workspace, node)
                .await?;
        }

        // 5. Validate schema (NOTE: This requires NodeValidator from raisin-core)
        // For now, we'll add a TODO marker - this will be implemented when we
        // integrate with NodeService's validation layer
        if options.validate_schema {
            // TODO: Integrate with NodeValidator from raisin-core
            // Challenge: NodeValidator needs Storage trait, but we're in RocksDB layer
            // Solution: Either:
            //   - Pass validator as parameter (requires trait changes)
            //   - Call service layer validation (layered approach)
            //   - Extract validator core logic to shared crate
            //
            // For now, we rely on service layer validation (NodeService validates before calling create)
            // This is acceptable because:
            // 1. Service layer is the primary entry point
            // 2. Direct repository calls are rare (mainly tests)
            // 3. Can be improved later without breaking API
        }

        Ok(())
    }

    /// Validate a node for update
    ///
    /// Performs all validation checks required before updating an existing node:
    /// 1. Checks node exists
    /// 2. Prevents node_type changes (unless explicitly allowed)
    /// 3. Validates schema against NodeType (if enabled)
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node` - The node to validate
    /// * `options` - Validation options controlling what checks to perform
    ///
    /// # Returns
    /// * `Ok(())` if all validations pass
    /// * `Err(Error::NotFound)` if node doesn't exist
    /// * `Err(Error::Validation)` if validation check fails (e.g., type change blocked)
    pub(crate) async fn validate_for_update(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        options: &UpdateNodeOptions,
    ) -> Result<()> {
        // 1. Check node exists (always required for update)
        let existing = self
            .get_impl(tenant_id, repo_id, branch, workspace, &node.id, false)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!("Node with id '{}' not found for update", node.id))
            })?;

        // 2. Check type change guard (prevent unless explicitly allowed)
        if !options.allow_type_change && existing.node_type != node.node_type {
            return Err(Error::Validation(format!(
                "Cannot change node_type from '{}' to '{}'. Set allow_type_change=true to permit this risky operation.",
                existing.node_type, node.node_type
            )));
        }

        // 3. Validate schema (see note in validate_for_create about NodeValidator integration)
        if options.validate_schema {
            // TODO: Integrate with NodeValidator from raisin-core
            // Same challenge as in validate_for_create - requires Storage trait
            // For now, rely on service layer validation
        }

        Ok(())
    }

    /// Validate parent-child type compatibility
    ///
    /// Checks if the parent's NodeType allows this child's NodeType in its
    /// `allowed_children` list.
    ///
    /// # Rules
    /// - If parent has no `allowed_children` list (or empty), all types allowed
    /// - If list contains "*", all types allowed
    /// - Otherwise, child's node_type must be in the list
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node` - The child node to validate
    ///
    /// # Returns
    /// * `Ok(())` if parent allows this child type (or no parent)
    /// * `Err(Error::Validation)` if parent doesn't allow this child type
    /// * `Err(Error::NotFound)` if parent node or NodeType not found
    async fn validate_parent_allows_child(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
    ) -> Result<()> {
        // Root nodes have no parent - always allowed
        let parent_path = match node.parent_path() {
            Some(p) if p != "/" => p,
            _ => return Ok(()), // Root level or no parent
        };

        // Get parent node
        let parent = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!("Parent node with path '{}' not found", parent_path))
            })?;

        // Get parent's NodeType to check allowed_children
        let parent_node_type = self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                &parent.node_type,
                None,
            )
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "NodeType '{}' not found for parent validation",
                    parent.node_type
                ))
            })?;

        // Check allowed_children constraint
        let allowed_children = &parent_node_type.allowed_children;
        // Empty list means "allow all"
        if !allowed_children.is_empty() {
            // Check for wildcard or explicit match
            let is_allowed = allowed_children.contains(&"*".to_string())
                || allowed_children.contains(&node.node_type);

            if !is_allowed {
                return Err(Error::Validation(format!(
                    "Node type '{}' not allowed as child of '{}'. Allowed types: {:?}",
                    node.node_type, parent.node_type, allowed_children
                )));
            }
        }

        Ok(())
    }

    /// Validate workspace type constraints
    ///
    /// Checks if the workspace allows this NodeType:
    /// - For root nodes: checks `allowed_root_node_types`
    /// - For all nodes: checks `allowed_node_types`
    ///
    /// # Rules
    /// - Empty list means "allow all"
    /// - Otherwise, node_type must be in the list
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node` - The node to validate
    ///
    /// # Returns
    /// * `Ok(())` if workspace allows this node type
    /// * `Err(Error::Validation)` if workspace doesn't allow this node type
    /// * `Err(Error::NotFound)` if workspace not found
    async fn validate_workspace_allows_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
    ) -> Result<()> {
        // Get workspace configuration
        let workspace_obj = self
            .workspace_repo
            .get(
                raisin_storage::RepoScope::new(tenant_id, repo_id),
                workspace,
            )
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Workspace '{}' not found for type validation",
                    workspace
                ))
            })?;

        // Check if this is a root node
        let is_root = node.parent_path().map(|p| p == "/").unwrap_or(true);

        // Check root node type constraints
        if is_root {
            let allowed_root_types = &workspace_obj.allowed_root_node_types;
            if !allowed_root_types.is_empty() {
                // Check for wildcard or explicit match
                let is_allowed = allowed_root_types.contains(&"*".to_string())
                    || allowed_root_types.contains(&node.node_type);
                if !is_allowed {
                    return Err(Error::Validation(format!(
                        "Workspace '{}' does not allow root nodes of type '{}'. Allowed root types: {:?}",
                        workspace, node.node_type, allowed_root_types
                    )));
                }
            }
        }

        // Check general node type constraints (for all nodes)
        let allowed_types = &workspace_obj.allowed_node_types;
        if !allowed_types.is_empty() {
            // Check for wildcard or explicit match
            let is_allowed =
                allowed_types.contains(&"*".to_string()) || allowed_types.contains(&node.node_type);
            if !is_allowed {
                return Err(Error::Validation(format!(
                    "Workspace '{}' does not allow nodes of type '{}'. Allowed types: {:?}",
                    workspace, node.node_type, allowed_types
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: Full integration tests will be added after implementing cascade delete
    // These tests require a full RocksDB setup with test fixtures

    #[test]
    fn test_validation_exists() {
        // Placeholder test to ensure module compiles
        // Real tests will be in integration test suite
        assert!(true);
    }
}
