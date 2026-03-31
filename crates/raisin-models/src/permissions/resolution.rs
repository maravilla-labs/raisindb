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

//! Resolved permissions for a user.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{Operation, Permission};

/// The resolved, flattened permissions for a user.
///
/// This is computed from the user's direct roles, group memberships,
/// and role inheritance, then cached for performance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPermissions {
    /// The user ID these permissions belong to
    pub user_id: String,

    /// User's email (for auth variable resolution)
    pub email: Option<String>,

    /// Direct role IDs assigned to the user
    pub direct_roles: Vec<String>,

    /// Role IDs inherited through groups
    pub group_roles: Vec<String>,

    /// All effective role IDs (direct + inherited, deduplicated)
    pub effective_roles: Vec<String>,

    /// Group IDs the user belongs to
    pub groups: Vec<String>,

    /// All flattened permissions from all roles
    pub permissions: Vec<Permission>,

    /// Whether this user has the system_admin role (bypasses all checks)
    pub is_system_admin: bool,

    /// When these permissions were resolved (for cache invalidation)
    #[serde(skip)]
    pub resolved_at: Option<Instant>,
}

impl ResolvedPermissions {
    /// Create empty resolved permissions for a user
    pub fn empty(user_id: impl Into<String>) -> Self {
        ResolvedPermissions {
            user_id: user_id.into(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions: vec![],
            is_system_admin: false,
            resolved_at: Some(Instant::now()),
        }
    }

    /// Create resolved permissions for the system admin
    pub fn system_admin() -> Self {
        ResolvedPermissions {
            user_id: "system".to_string(),
            email: None,
            direct_roles: vec!["system_admin".to_string()],
            group_roles: vec![],
            effective_roles: vec!["system_admin".to_string()],
            groups: vec![],
            permissions: vec![Permission::full_access("**")],
            is_system_admin: true,
            resolved_at: Some(Instant::now()),
        }
    }

    /// Create resolved permissions for anonymous access
    pub fn anonymous(permissions: Vec<Permission>) -> Self {
        ResolvedPermissions {
            user_id: "anonymous".to_string(),
            email: None,
            direct_roles: vec!["anonymous".to_string()],
            group_roles: vec![],
            effective_roles: vec!["anonymous".to_string()],
            groups: vec![],
            permissions,
            is_system_admin: false,
            resolved_at: Some(Instant::now()),
        }
    }

    /// Create resolved permissions that deny all access.
    ///
    /// Use for deny-by-default when:
    /// - Anonymous access is disabled and no auth token is provided
    /// - Access should be explicitly denied
    pub fn deny_all() -> Self {
        ResolvedPermissions {
            user_id: "$deny".to_string(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions: vec![], // Empty = deny all
            is_system_admin: false,
            resolved_at: Some(Instant::now()),
        }
    }

    /// Check if the permissions cache is still valid
    pub fn is_valid(&self, ttl: Duration) -> bool {
        match self.resolved_at {
            Some(resolved_at) => resolved_at.elapsed() < ttl,
            None => false,
        }
    }

    /// Check if this user has a specific role
    pub fn has_role(&self, role_id: &str) -> bool {
        self.effective_roles.iter().any(|r| r == role_id)
    }

    /// Check if this user is in a specific group
    pub fn in_group(&self, group_id: &str) -> bool {
        self.groups.iter().any(|g| g == group_id)
    }

    /// Find all permissions that match a given path pattern
    ///
    /// Note: This does NOT evaluate conditions, only returns potentially matching permissions.
    /// Conditions should be evaluated against the actual node being accessed.
    pub fn permissions_for_path(&self, path: &str) -> Vec<&Permission> {
        if self.is_system_admin {
            return self.permissions.iter().collect();
        }

        self.permissions
            .iter()
            .filter(|p| path_matches_pattern(path, &p.path))
            .collect()
    }

    /// Check if this user has any permission for an operation on a path
    ///
    /// Note: This is a quick check that doesn't evaluate conditions.
    /// For actual authorization, conditions must also be evaluated.
    pub fn may_have_access(&self, path: &str, operation: Operation) -> bool {
        if self.is_system_admin {
            return true;
        }

        self.permissions_for_path(path)
            .iter()
            .any(|p| p.allows_operation(operation))
    }
}

/// Check if a path matches a permission pattern.
///
/// Pattern syntax:
/// - `*` - matches exactly one path segment
/// - `**` - matches zero or more path segments
/// - Literal segments must match exactly
///
/// Examples:
/// - `content.articles.**` matches `/content/articles/`, `/content/articles/foo`, `/content/articles/foo/bar`
/// - `users.*.profile` matches `/users/john/profile`, `/users/jane/profile`
/// - `**` matches everything
pub fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    // Normalize paths: remove leading slash, convert to segments
    let path = path.trim_start_matches('/');
    let pattern = pattern.trim_start_matches('/');

    // Convert to segments (support both `/` and `.` as separators)
    let path_segments: Vec<&str> = path.split(['/', '.']).filter(|s| !s.is_empty()).collect();

    let pattern_segments: Vec<&str> = pattern
        .split(['/', '.'])
        .filter(|s| !s.is_empty())
        .collect();

    match_segments(&path_segments, &pattern_segments)
}

fn match_segments(path: &[&str], pattern: &[&str]) -> bool {
    // Handle empty cases
    if pattern.is_empty() {
        return path.is_empty();
    }

    let first_pattern = pattern[0];
    let rest_pattern = &pattern[1..];

    match first_pattern {
        "**" => {
            // `**` can match zero or more segments
            // Try matching with 0, 1, 2, ... segments consumed
            if rest_pattern.is_empty() {
                // `**` at end matches everything remaining
                return true;
            }

            // Try consuming 0 to all remaining path segments
            for i in 0..=path.len() {
                if match_segments(&path[i..], rest_pattern) {
                    return true;
                }
            }
            false
        }
        "*" => {
            // `*` matches exactly one segment
            if path.is_empty() {
                false
            } else {
                match_segments(&path[1..], rest_pattern)
            }
        }
        literal => {
            // Literal segment must match exactly
            if path.is_empty() || path[0] != literal {
                false
            } else {
                match_segments(&path[1..], rest_pattern)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_matching_literal() {
        assert!(path_matches_pattern(
            "/content/articles",
            "content.articles"
        ));
        assert!(path_matches_pattern("content/articles", "content.articles"));
        assert!(!path_matches_pattern("/content/posts", "content.articles"));
    }

    #[test]
    fn test_path_matching_single_wildcard() {
        assert!(path_matches_pattern(
            "/users/john/profile",
            "users.*.profile"
        ));
        assert!(path_matches_pattern(
            "/users/jane/profile",
            "users.*.profile"
        ));
        assert!(!path_matches_pattern(
            "/users/john/settings",
            "users.*.profile"
        ));
        assert!(!path_matches_pattern("/users/profile", "users.*.profile")); // missing segment
    }

    #[test]
    fn test_path_matching_double_wildcard() {
        // `**` at end
        assert!(path_matches_pattern(
            "/content/articles",
            "content.articles.**"
        ));
        assert!(path_matches_pattern(
            "/content/articles/foo",
            "content.articles.**"
        ));
        assert!(path_matches_pattern(
            "/content/articles/foo/bar/baz",
            "content.articles.**"
        ));

        // `**` in middle
        assert!(path_matches_pattern("/a/b/c/d", "a.**.d"));
        assert!(path_matches_pattern("/a/d", "a.**.d"));

        // `**` at start
        assert!(path_matches_pattern("/foo/bar/config", "**.config"));
        assert!(path_matches_pattern("/config", "**.config"));
    }

    #[test]
    fn test_path_matching_full_wildcard() {
        assert!(path_matches_pattern("/anything", "**"));
        assert!(path_matches_pattern("/foo/bar/baz", "**"));
        assert!(path_matches_pattern("", "**"));
    }

    #[test]
    fn test_resolved_permissions_system_admin() {
        let perms = ResolvedPermissions::system_admin();
        assert!(perms.is_system_admin);
        assert!(perms.may_have_access("/anything/at/all", Operation::Delete));
    }

    #[test]
    fn test_resolved_permissions_empty() {
        let perms = ResolvedPermissions::empty("user123");
        assert!(!perms.is_system_admin);
        assert!(!perms.may_have_access("/content/articles", Operation::Read));
    }
}
