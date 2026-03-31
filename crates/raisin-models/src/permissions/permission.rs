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

//! Permission and Operation types.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::path_matcher::PathMatcher;
use super::scope_matcher::{PermissionScope, ScopeMatcher};

/// Operations that can be performed on nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    /// Create new nodes
    Create,
    /// Read/view nodes
    Read,
    /// Update existing nodes
    Update,
    /// Delete nodes
    Delete,
    /// Translate nodes (modify translations)
    Translate,
    /// Relate nodes (create relationships)
    Relate,
    /// Unrelate nodes (remove relationships)
    Unrelate,
}

impl Operation {
    /// All available operations
    pub fn all() -> Vec<Operation> {
        vec![
            Operation::Create,
            Operation::Read,
            Operation::Update,
            Operation::Delete,
            Operation::Translate,
            Operation::Relate,
            Operation::Unrelate,
        ]
    }

    /// Parse from string representation
    pub fn parse(s: &str) -> Option<Operation> {
        match s.to_lowercase().as_str() {
            "create" => Some(Operation::Create),
            "read" => Some(Operation::Read),
            "update" => Some(Operation::Update),
            "delete" => Some(Operation::Delete),
            "translate" => Some(Operation::Translate),
            "relate" => Some(Operation::Relate),
            "unrelate" => Some(Operation::Unrelate),
            _ => None,
        }
    }
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operation::Create => write!(f, "create"),
            Operation::Read => write!(f, "read"),
            Operation::Update => write!(f, "update"),
            Operation::Delete => write!(f, "delete"),
            Operation::Translate => write!(f, "translate"),
            Operation::Relate => write!(f, "relate"),
            Operation::Unrelate => write!(f, "unrelate"),
        }
    }
}

/// A permission grant that specifies what operations are allowed on which paths.
///
/// Permissions are defined in roles and can include:
/// - Workspace and branch patterns (scope restriction)
/// - Path patterns (glob-style matching)
/// - Node type restrictions
/// - Allowed operations
/// - Field-level access control
/// - Runtime conditions
#[derive(Clone, Serialize, Deserialize)]
pub struct Permission {
    /// Workspace pattern to match (glob-style, e.g., "content-*", "media")
    /// If None or empty, matches all workspaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,

    /// Branch pattern to match (glob-style, e.g., "main", "features/*", "release-*")
    /// If None or empty, matches all branches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_pattern: Option<String>,

    /// Path pattern to match (e.g., "content.articles.**", "users.*.profile")
    ///
    /// Supports:
    /// - `*` - matches any single path segment
    /// - `**` - matches any number of path segments (recursive)
    /// - Literal segments
    pub path: String,

    /// Restrict to specific node types (optional)
    /// If empty, applies to all node types
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_types: Option<Vec<String>>,

    /// Operations allowed by this permission
    pub operations: Vec<Operation>,

    /// Whitelist: only these fields are accessible
    /// If set, only these fields can be read/written
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,

    /// Blacklist: these fields are NOT accessible
    /// If set, all fields except these can be read/written
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub except_fields: Option<Vec<String>>,

    /// REL expression condition for this permission.
    /// Must evaluate to truthy for the permission to apply.
    /// Example: "node.created_by == auth.user_id"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,

    /// Cached scope matcher (not serialized)
    #[serde(skip)]
    cached_scope_matcher: OnceLock<ScopeMatcher>,

    /// Cached path matcher (not serialized)
    #[serde(skip)]
    cached_path_matcher: OnceLock<PathMatcher>,
}

impl Permission {
    /// Create a new permission with the given path and operations
    pub fn new(path: impl Into<String>, operations: Vec<Operation>) -> Self {
        Permission {
            workspace: None,
            branch_pattern: None,
            path: path.into(),
            node_types: None,
            operations,
            fields: None,
            except_fields: None,
            condition: None,
            cached_scope_matcher: OnceLock::new(),
            cached_path_matcher: OnceLock::new(),
        }
    }

    /// Create a full access permission (all operations) for a path
    pub fn full_access(path: impl Into<String>) -> Self {
        Permission::new(path, Operation::all())
    }

    /// Create a read-only permission for a path
    pub fn read_only(path: impl Into<String>) -> Self {
        Permission::new(path, vec![Operation::Read])
    }

    /// Set workspace pattern (glob-style)
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = Some(workspace.into());
        self.cached_scope_matcher = OnceLock::new(); // Reset cache
        self
    }

    /// Set branch pattern (glob-style)
    pub fn with_branch_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.branch_pattern = Some(pattern.into());
        self.cached_scope_matcher = OnceLock::new(); // Reset cache
        self
    }

    /// Add node type restrictions
    pub fn with_node_types(mut self, node_types: Vec<String>) -> Self {
        self.node_types = Some(node_types);
        self
    }

    /// Add field whitelist
    pub fn with_fields(mut self, fields: Vec<String>) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Add field blacklist
    pub fn with_except_fields(mut self, except_fields: Vec<String>) -> Self {
        self.except_fields = Some(except_fields);
        self
    }

    /// Set a REL expression condition
    /// Example: "node.created_by == auth.user_id"
    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.condition = Some(condition.into());
        self
    }

    /// Get or create the cached scope matcher.
    ///
    /// The matcher is compiled once and reused for efficiency.
    pub fn scope_matcher(&self) -> &ScopeMatcher {
        self.cached_scope_matcher.get_or_init(|| {
            ScopeMatcher::new(self.workspace.as_deref(), self.branch_pattern.as_deref())
        })
    }

    /// Get or create the cached path matcher.
    ///
    /// The matcher is compiled once and reused for efficiency.
    pub fn path_matcher(&self) -> &PathMatcher {
        self.cached_path_matcher
            .get_or_init(|| PathMatcher::new(&self.path))
    }

    /// Check if this permission applies to the given scope.
    ///
    /// Returns true if both workspace and branch patterns match.
    pub fn applies_to_scope(&self, scope: &PermissionScope) -> bool {
        self.scope_matcher().matches(scope)
    }

    /// Check if a path matches this permission's path pattern.
    ///
    /// Uses the cached PathMatcher for efficiency.
    pub fn matches_path(&self, path: &str) -> bool {
        self.path_matcher().matches(path)
    }

    /// Get the path pattern specificity score.
    ///
    /// Higher scores indicate more specific patterns.
    pub fn path_specificity(&self) -> usize {
        self.path_matcher().specificity()
    }

    /// Check if this permission grants a specific operation
    pub fn allows_operation(&self, op: Operation) -> bool {
        self.operations.contains(&op)
    }

    /// Check if this permission applies to a specific node type
    pub fn applies_to_node_type(&self, node_type: &str) -> bool {
        match &self.node_types {
            Some(types) => types.iter().any(|t| t == node_type),
            None => true, // No restriction = applies to all
        }
    }

    /// Get the list of allowed fields (if whitelist is set)
    pub fn allowed_fields(&self) -> Option<&[String]> {
        self.fields.as_deref()
    }

    /// Get the list of denied fields (if blacklist is set)
    pub fn denied_fields(&self) -> Option<&[String]> {
        self.except_fields.as_deref()
    }

    /// Check if a field is accessible under this permission
    pub fn is_field_accessible(&self, field: &str) -> bool {
        // Check whitelist first
        if let Some(allowed) = &self.fields {
            return allowed.iter().any(|f| f == field);
        }

        // Check blacklist
        if let Some(denied) = &self.except_fields {
            return !denied.iter().any(|f| f == field);
        }

        // No restrictions
        true
    }
}

// Manual Debug implementation (OnceCell doesn't derive Debug well)
impl std::fmt::Debug for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Permission")
            .field("workspace", &self.workspace)
            .field("branch_pattern", &self.branch_pattern)
            .field("path", &self.path)
            .field("node_types", &self.node_types)
            .field("operations", &self.operations)
            .field("fields", &self.fields)
            .field("except_fields", &self.except_fields)
            .field("condition", &self.condition)
            .finish()
    }
}

// Manual PartialEq implementation (OnceCell doesn't derive PartialEq)
impl PartialEq for Permission {
    fn eq(&self, other: &Self) -> bool {
        self.workspace == other.workspace
            && self.branch_pattern == other.branch_pattern
            && self.path == other.path
            && self.node_types == other.node_types
            && self.operations == other.operations
            && self.fields == other.fields
            && self.except_fields == other.except_fields
            && self.condition == other.condition
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_builder() {
        let permission = Permission::full_access("content.articles.**")
            .with_node_types(vec!["blog:Article".to_string()])
            .with_except_fields(vec!["internal_notes".to_string()])
            .with_condition("node.created_by == auth.user_id");

        assert_eq!(permission.path, "content.articles.**");
        assert_eq!(
            permission.condition,
            Some("node.created_by == auth.user_id".to_string())
        );
        assert!(permission.allows_operation(Operation::Create));
        assert!(permission.allows_operation(Operation::Read));
        assert!(permission.allows_operation(Operation::Update));
        assert!(permission.allows_operation(Operation::Delete));
        assert!(permission.allows_operation(Operation::Translate));
        assert!(permission.allows_operation(Operation::Relate));
        assert!(permission.allows_operation(Operation::Unrelate));
        assert!(permission.applies_to_node_type("blog:Article"));
        assert!(!permission.applies_to_node_type("blog:Comment"));
        assert!(permission.is_field_accessible("title"));
        assert!(!permission.is_field_accessible("internal_notes"));
    }

    #[test]
    fn test_read_only_permission() {
        let permission = Permission::read_only("public/**");

        assert!(!permission.allows_operation(Operation::Create));
        assert!(permission.allows_operation(Operation::Read));
        assert!(!permission.allows_operation(Operation::Update));
        assert!(!permission.allows_operation(Operation::Delete));
        assert!(!permission.allows_operation(Operation::Translate));
        assert!(!permission.allows_operation(Operation::Relate));
        assert!(!permission.allows_operation(Operation::Unrelate));
    }

    #[test]
    fn test_field_whitelist() {
        let permission = Permission::read_only("users/**")
            .with_fields(vec!["name".to_string(), "email".to_string()]);

        assert!(permission.is_field_accessible("name"));
        assert!(permission.is_field_accessible("email"));
        assert!(!permission.is_field_accessible("password_hash"));
    }
}
