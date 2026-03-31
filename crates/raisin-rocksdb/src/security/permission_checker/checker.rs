//! Core permission checker implementation.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission};

use super::super::condition_evaluator::ConditionEvaluator;
use super::super::field_filter::filter_node_fields;
use super::super::path_matcher::matches_path_pattern;

/// Permission checker for RLS enforcement.
pub struct PermissionChecker<'a> {
    auth: &'a AuthContext,
    permissions: &'a [Permission],
}

impl<'a> PermissionChecker<'a> {
    /// Create a new permission checker from an auth context.
    pub fn new(auth: &'a AuthContext) -> Option<Self> {
        // If system context, no permission checking needed
        if auth.is_system {
            return None;
        }

        // Get resolved permissions
        let resolved = auth.permissions()?;

        Some(Self {
            auth,
            permissions: &resolved.permissions,
        })
    }

    /// Create a permission checker with explicit permissions.
    pub fn with_permissions(auth: &'a AuthContext, permissions: &'a [Permission]) -> Self {
        Self { auth, permissions }
    }

    /// Check if the user can perform an operation on a node.
    pub fn can_perform(&self, node: &Node, operation: Operation) -> bool {
        if self.auth.is_system {
            return true;
        }

        let matching = self.find_matching_permissions(node);

        if matching.is_empty() {
            return false;
        }

        for permission in matching {
            if self.permission_allows(permission, node, operation) {
                return true;
            }
        }

        false
    }

    /// Check if the user can read a node.
    pub fn can_read(&self, node: &Node) -> bool {
        self.can_perform(node, Operation::Read)
    }

    /// Check if the user can create a node at a path.
    pub fn can_create(&self, node: &Node) -> bool {
        self.can_perform(node, Operation::Create)
    }

    /// Check if the user can update a node.
    pub fn can_update(&self, node: &Node) -> bool {
        self.can_perform(node, Operation::Update)
    }

    /// Check if the user can delete a node.
    pub fn can_delete(&self, node: &Node) -> bool {
        self.can_perform(node, Operation::Delete)
    }

    /// Filter a list of nodes to only those the user can read.
    ///
    /// This also applies field-level filtering to each node.
    pub fn filter_readable(&self, nodes: Vec<Node>) -> Vec<Node> {
        nodes
            .into_iter()
            .filter_map(|mut node| {
                if self.can_read(&node) {
                    if let Some(permission) = self.find_best_matching_permission(&node) {
                        filter_node_fields(&mut node, permission);
                    }
                    Some(node)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Filter a single node if readable, applying field filtering.
    pub fn filter_if_readable(&self, mut node: Node) -> Option<Node> {
        if !self.can_read(&node) {
            return None;
        }

        if let Some(permission) = self.find_best_matching_permission(&node) {
            filter_node_fields(&mut node, permission);
        }

        Some(node)
    }

    /// Check if the user can perform an operation on a node (async version).
    ///
    /// This version supports async conditions like RELATES.
    pub async fn can_perform_async(
        &self,
        node: &Node,
        operation: Operation,
        graph_resolver: Option<&dyn raisin_rel::eval::RelationResolver>,
    ) -> bool {
        if self.auth.is_system {
            return true;
        }

        let matching = self.find_matching_permissions(node);

        if matching.is_empty() {
            return false;
        }

        for permission in matching {
            if self
                .permission_allows_async(permission, node, operation, graph_resolver)
                .await
            {
                return true;
            }
        }

        false
    }

    /// Check if the user can read a node (async version).
    pub async fn can_read_async(
        &self,
        node: &Node,
        graph_resolver: Option<&dyn raisin_rel::eval::RelationResolver>,
    ) -> bool {
        self.can_perform_async(node, Operation::Read, graph_resolver)
            .await
    }

    /// Check if the user can update a node (async version).
    pub async fn can_update_async(
        &self,
        node: &Node,
        graph_resolver: Option<&dyn raisin_rel::eval::RelationResolver>,
    ) -> bool {
        self.can_perform_async(node, Operation::Update, graph_resolver)
            .await
    }

    /// Check if the user can delete a node (async version).
    pub async fn can_delete_async(
        &self,
        node: &Node,
        graph_resolver: Option<&dyn raisin_rel::eval::RelationResolver>,
    ) -> bool {
        self.can_perform_async(node, Operation::Delete, graph_resolver)
            .await
    }

    /// Find all permissions that match a node's path and type.
    fn find_matching_permissions(&self, node: &Node) -> Vec<&Permission> {
        self.permissions
            .iter()
            .filter(|p| {
                if !matches_path_pattern(&p.path, &node.path) {
                    return false;
                }

                if let Some(types) = &p.node_types {
                    if !types.contains(&node.node_type) {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Find the best (most specific) matching permission.
    fn find_best_matching_permission(&self, node: &Node) -> Option<&Permission> {
        self.find_matching_permissions(node)
            .into_iter()
            .max_by_key(|p| calculate_specificity(&p.path))
    }

    /// Check if a specific permission allows an operation on a node.
    fn permission_allows(
        &self,
        permission: &Permission,
        node: &Node,
        operation: Operation,
    ) -> bool {
        if !permission.operations.contains(&operation) {
            return false;
        }

        if let Some(condition) = &permission.condition {
            let evaluator = ConditionEvaluator::new(self.auth);

            if ConditionEvaluator::requires_async(condition) {
                tracing::warn!(
                    condition = %condition,
                    "Async condition evaluation required but called from sync context. Use can_perform_async instead."
                );
                return false;
            }

            if !evaluator.evaluate_rel_expression(condition, node) {
                return false;
            }
        }

        true
    }

    /// Check if a specific permission allows an operation on a node (async version).
    async fn permission_allows_async(
        &self,
        permission: &Permission,
        node: &Node,
        operation: Operation,
        graph_resolver: Option<&dyn raisin_rel::eval::RelationResolver>,
    ) -> bool {
        if !permission.operations.contains(&operation) {
            return false;
        }

        if let Some(condition) = &permission.condition {
            let evaluator = ConditionEvaluator::new(self.auth);

            if !evaluator
                .evaluate_rel_expression_async(condition, node, graph_resolver)
                .await
            {
                return false;
            }
        }

        true
    }
}

/// Calculate pattern specificity for tie-breaking.
fn calculate_specificity(pattern: &str) -> usize {
    let segments: Vec<&str> = pattern.split('.').collect();
    let mut score = 0;

    for segment in &segments {
        match *segment {
            "**" => score += 1,
            "*" => score += 10,
            _ => score += 100,
        }
    }

    score += segments.len() * 5;
    score
}

/// Check if a user can read nodes in a path without loading the nodes.
///
/// This is a quick check for query optimization - if the user has no
/// read permissions in a path prefix, we can skip scanning entirely.
pub fn can_read_in_path(auth: &AuthContext, permissions: &[Permission], path_prefix: &str) -> bool {
    if auth.is_system {
        return true;
    }

    for permission in permissions {
        if permission_path_overlaps(&permission.path, path_prefix)
            && permission.operations.contains(&Operation::Read)
        {
            return true;
        }
    }

    false
}

/// Check if a permission path pattern overlaps with a query path prefix.
fn permission_path_overlaps(pattern: &str, path_prefix: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('.').filter(|s| !s.is_empty()).collect();
    let path_segments: Vec<&str> = path_prefix.split('/').filter(|s| !s.is_empty()).collect();

    for (i, (p, s)) in pattern_segments
        .iter()
        .zip(path_segments.iter())
        .enumerate()
    {
        match *p {
            "**" => return true,
            "*" => continue,
            _ if p != s => return false,
            _ => continue,
        }
    }

    if pattern_segments.len() > path_segments.len() {
        pattern_segments[path_segments.len()..].contains(&"**")
    } else {
        true
    }
}
