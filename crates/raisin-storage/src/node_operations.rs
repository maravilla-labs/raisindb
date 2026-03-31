// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Node operation types and options
//!
//! Provides configuration types for NodeRepository operations,
//! enabling explicit control over validation, performance, and behavior.

use raisin_hlc::HLC;
use raisin_models as models;
use serde::{Deserialize, Serialize};

/// Options for creating a new node
///
/// Controls validation behavior when creating nodes. All validation
/// is enabled by default for safety.
///
/// # Examples
///
/// ```
/// use raisin_storage::CreateNodeOptions;
///
/// // Full validation (default, recommended for API endpoints)
/// let opts = CreateNodeOptions::default();
///
/// // Skip validation for trusted bulk imports
/// let opts = CreateNodeOptions {
///     validate_schema: false,
///     validate_parent_allows_child: false,
///     validate_workspace_allows_type: false,
///     operation_meta: None,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct CreateNodeOptions {
    /// Whether to validate properties against NodeType schema
    ///
    /// When true:
    /// - Required properties must be present
    /// - Property types must match schema
    /// - Strict mode: no extra properties if NodeType.strict=true
    ///
    /// Default: true (always validate in production)
    pub validate_schema: bool,

    /// Whether to check parent-child type compatibility
    ///
    /// When true, verifies parent's `allowed_children` includes this node type.
    ///
    /// Default: true
    pub validate_parent_allows_child: bool,

    /// Whether to check workspace allows this node type
    ///
    /// When true:
    /// - For root nodes: checks `workspace.allowed_root_node_types`
    /// - For all nodes: checks `workspace.allowed_node_types`
    ///
    /// Default: true
    pub validate_workspace_allows_type: bool,

    /// Custom operation metadata for audit trail
    ///
    /// Include actor, message, and other metadata for tracking changes.
    pub operation_meta: Option<models::operations::OperationMeta>,
}

impl Default for CreateNodeOptions {
    fn default() -> Self {
        Self {
            validate_schema: true,
            validate_parent_allows_child: true,
            validate_workspace_allows_type: true,
            operation_meta: None,
        }
    }
}

/// Options for updating an existing node
///
/// Controls validation and safety guards when updating nodes.
///
/// # Examples
///
/// ```
/// use raisin_storage::UpdateNodeOptions;
///
/// // Safe update (default)
/// let opts = UpdateNodeOptions::default();
///
/// // Allow risky type change
/// let opts = UpdateNodeOptions {
///     validate_schema: true,
///     allow_type_change: true, // Dangerous!
///     operation_meta: None,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct UpdateNodeOptions {
    /// Whether to validate properties against NodeType schema
    ///
    /// Default: true
    pub validate_schema: bool,

    /// Allow changing node_type (risky operation)
    ///
    /// When false (default), attempting to change node_type returns an error.
    /// Set to true only when you explicitly want to change the type of an
    /// existing node (e.g., converting a Folder to a Page).
    ///
    /// **Warning**: Changing node_type can break references and violate
    /// parent-child constraints. Use with caution.
    ///
    /// Default: false (prevents accidental type changes)
    pub allow_type_change: bool,

    /// Custom operation metadata for audit trail
    pub operation_meta: Option<models::operations::OperationMeta>,
}

impl Default for UpdateNodeOptions {
    fn default() -> Self {
        Self {
            validate_schema: true,
            allow_type_change: false,
            operation_meta: None,
        }
    }
}

/// Options for deleting a node
///
/// Controls cascade behavior and safety checks.
///
/// # Safety
///
/// By default, cascades to all descendants to prevent orphaned nodes.
/// Disable cascade only if you know the node has no children.
///
/// # Examples
///
/// ```
/// use raisin_storage::DeleteNodeOptions;
///
/// // Safe delete with cascade (default)
/// let opts = DeleteNodeOptions::default();
///
/// // Fail if node has children
/// let opts = DeleteNodeOptions {
///     cascade: false,
///     check_has_children: true,
///     operation_meta: None,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct DeleteNodeOptions {
    /// Whether to cascade delete to all descendants
    ///
    /// When true (default): Recursively delete all children and their children.
    /// When false: Fail if node has children (unless check_has_children=false).
    ///
    /// Default: true (prevent orphaned nodes)
    pub cascade: bool,

    /// Whether to check if node has children before delete
    ///
    /// When true (default) and cascade=false: Fail if node has children.
    /// When false: Allow deleting nodes with children (leaves orphans!).
    ///
    /// **Warning**: Setting this to false with cascade=false can create
    /// orphaned nodes that are unreachable via tree navigation.
    ///
    /// Default: true (fail-safe)
    pub check_has_children: bool,

    /// Custom operation metadata for audit trail
    pub operation_meta: Option<models::operations::OperationMeta>,
}

impl Default for DeleteNodeOptions {
    fn default() -> Self {
        Self {
            cascade: true,
            check_has_children: true,
            operation_meta: None,
        }
    }
}

/// Options for list operations that control performance
///
/// The `compute_has_children` flag has significant performance impact:
/// - true: Performs N additional queries for N nodes (expensive)
/// - false: Returns has_children=None for all nodes (fast)
///
/// # Performance
///
/// For 1000 nodes:
/// - `compute_has_children=false`: ~5ms (1 query)
/// - `compute_has_children=true`: ~300ms (1001 queries)
///
/// **Guideline**: Use `for_api()` for UI responses, `for_sql()` for queries.
///
/// # Examples
///
/// ```
/// use raisin_storage::ListOptions;
///
/// // API responses (users need expand arrows)
/// let opts = ListOptions::for_api();
///
/// // SQL queries (don't need UI metadata)
/// let opts = ListOptions::for_sql();
///
/// // Time-travel query at specific revision
/// let opts = ListOptions::at_revision(HLC::new(42, 0));
/// ```
#[derive(Debug, Clone, Default)]
pub struct ListOptions {
    /// Whether to compute has_children for each node
    ///
    /// - true: Populate has_children (for UI/API responses)
    /// - false: Leave has_children as None (for SQL queries, faster)
    ///
    /// Default: false (performance by default)
    pub compute_has_children: bool,

    /// Maximum revision to query (for time-travel)
    ///
    /// When Some(rev), returns nodes as they existed at that revision.
    /// When None, returns latest version.
    ///
    /// Default: None (latest)
    pub max_revision: Option<HLC>,
}

impl ListOptions {
    /// Create list options for UI/API usage (computes has_children)
    ///
    /// Use this when:
    /// - Serving REST API responses
    /// - Building UI tree views
    /// - Users need to see expand arrows
    ///
    /// # Performance
    /// For N nodes, performs N+1 queries (1 for nodes, N for has_children checks).
    #[inline]
    pub fn for_api() -> Self {
        Self {
            compute_has_children: true,
            max_revision: None,
        }
    }

    /// Create list options for SQL engine (skips has_children)
    ///
    /// Use this when:
    /// - Executing SQL queries
    /// - Building search results
    /// - UI metadata not needed
    ///
    /// # Performance
    /// For N nodes, performs 1 query (significantly faster than for_api).
    #[inline]
    pub fn for_sql() -> Self {
        Self {
            compute_has_children: false,
            max_revision: None,
        }
    }

    /// Create list options with specific revision (time-travel)
    ///
    /// Returns nodes as they existed at the given revision.
    /// Does not compute has_children by default.
    ///
    /// # Arguments
    /// * `revision` - The revision (HLC timestamp) to query
    #[inline]
    pub fn at_revision(revision: HLC) -> Self {
        Self {
            compute_has_children: false,
            max_revision: Some(revision),
        }
    }

    /// Create list options for API with specific revision
    ///
    /// Combines time-travel with has_children computation.
    #[inline]
    pub fn for_api_at_revision(revision: HLC) -> Self {
        Self {
            compute_has_children: true,
            max_revision: Some(revision),
        }
    }
}

/// A node returned from get_with_children that includes populated children
///
/// This type is specifically for the `get_with_children()` method and always
/// has children populated (even if empty Vec).
///
/// # Differences from NodeWithChildren
///
/// This is different from `models::nodes::NodeWithChildren` which has a
/// `ChildrenField` enum. This type always has populated children as `Vec<Node>`.
///
/// # Examples
///
/// ```
/// use raisin_storage::NodeWithPopulatedChildren;
///
/// let parent = storage.nodes()
///     .get_with_children(tenant, repo, branch, ws, "parent-id", None)
///     .await?
///     .unwrap();
///
/// println!("Parent: {}", parent.node.name);
/// println!("Children: {}", parent.children_nodes.len());
///
/// for child in parent.children_nodes {
///     println!("  - {}", child.name);
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeWithPopulatedChildren {
    /// The parent node (with has_children computed)
    ///
    /// The has_children field will be:
    /// - Some(true) if children_nodes is not empty
    /// - Some(false) if children_nodes is empty
    #[serde(flatten)]
    pub node: models::nodes::Node,

    /// Direct children as full Node objects
    ///
    /// This Vec is always populated (may be empty if no children).
    /// Children are returned in their stored order (fractional index order).
    pub children_nodes: Vec<models::nodes::Node>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_options_default() {
        let opts = CreateNodeOptions::default();
        assert!(opts.validate_schema);
        assert!(opts.validate_parent_allows_child);
        assert!(opts.validate_workspace_allows_type);
        assert!(opts.operation_meta.is_none());
    }

    #[test]
    fn test_update_options_default() {
        let opts = UpdateNodeOptions::default();
        assert!(opts.validate_schema);
        assert!(!opts.allow_type_change);
        assert!(opts.operation_meta.is_none());
    }

    #[test]
    fn test_delete_options_default() {
        let opts = DeleteNodeOptions::default();
        assert!(opts.cascade);
        assert!(opts.check_has_children);
        assert!(opts.operation_meta.is_none());
    }

    #[test]
    fn test_list_options_default() {
        let opts = ListOptions::default();
        assert!(!opts.compute_has_children);
        assert!(opts.max_revision.is_none());
    }

    #[test]
    fn test_list_options_for_api() {
        let opts = ListOptions::for_api();
        assert!(opts.compute_has_children);
        assert!(opts.max_revision.is_none());
    }

    #[test]
    fn test_list_options_for_sql() {
        let opts = ListOptions::for_sql();
        assert!(!opts.compute_has_children);
        assert!(opts.max_revision.is_none());
    }

    #[test]
    fn test_list_options_at_revision() {
        let rev = raisin_hlc::HLC::new(42, 0);
        let opts = ListOptions::at_revision(rev);
        assert!(!opts.compute_has_children);
        assert_eq!(opts.max_revision, Some(raisin_hlc::HLC::new(42, 0)));
    }

    #[test]
    fn test_list_options_for_api_at_revision() {
        let rev = raisin_hlc::HLC::new(100, 0);
        let opts = ListOptions::for_api_at_revision(rev);
        assert!(opts.compute_has_children);
        assert_eq!(opts.max_revision, Some(raisin_hlc::HLC::new(100, 0)));
    }
}
