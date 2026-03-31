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

//! Scope matching for workspace and branch patterns.
//!
//! This module provides types for matching permission scopes based on
//! workspace and branch patterns. Patterns support glob-style wildcards:
//!
//! - `*` matches any sequence of characters (except path separators)
//! - `?` matches any single character
//!
//! Examples:
//! - `content-*` matches `content-us`, `content-eu`
//! - `features/*` matches `features/auth`, `features/login`
//! - Empty/None pattern matches all values

use glob::Pattern;

/// Execution context for permission evaluation.
///
/// Contains the current workspace and branch for scope-aware permission checks.
/// This is passed alongside `AuthContext` during permission evaluation.
#[derive(Debug, Clone, Default)]
pub struct PermissionScope {
    /// Current workspace ID (e.g., "content", "media", "raisin:access_control")
    pub workspace: String,
    /// Current branch name (e.g., "main", "feature/login", "release-v2")
    pub branch: String,
}

impl PermissionScope {
    /// Create a new permission scope.
    pub fn new(workspace: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            branch: branch.into(),
        }
    }

    /// Create a scope that matches everything (for system context or tests).
    ///
    /// Note: This creates a scope with empty strings, which will match
    /// against any ScopeMatcher (since empty patterns match all).
    pub fn any() -> Self {
        Self::default()
    }
}

/// Pre-compiled scope patterns for efficient matching.
///
/// Compiles workspace and branch patterns once for reuse across many matches.
/// This is more efficient than re-compiling patterns on every permission check.
#[derive(Debug, Clone)]
pub struct ScopeMatcher {
    /// Compiled workspace pattern (None = matches all)
    workspace_pattern: Option<Pattern>,
    /// Original workspace pattern string (for display/debugging)
    workspace_raw: Option<String>,
    /// Compiled branch pattern (None = matches all)
    branch_pattern: Option<Pattern>,
    /// Original branch pattern string (for display/debugging)
    branch_raw: Option<String>,
}

impl ScopeMatcher {
    /// Create a new scope matcher from permission patterns.
    ///
    /// Empty, None, or "*" patterns match everything.
    /// Invalid patterns are treated as "match nothing" (returns None on compile error).
    pub fn new(workspace: Option<&str>, branch: Option<&str>) -> Self {
        Self {
            workspace_pattern: Self::compile_pattern(workspace),
            workspace_raw: workspace.filter(|s| !s.is_empty()).map(String::from),
            branch_pattern: Self::compile_pattern(branch),
            branch_raw: branch.filter(|s| !s.is_empty()).map(String::from),
        }
    }

    /// Compile a pattern string into a glob Pattern.
    ///
    /// Returns None for:
    /// - Empty/whitespace strings (matches all)
    /// - "*" (matches all)
    /// - Invalid patterns (treated as match nothing)
    fn compile_pattern(pattern: Option<&str>) -> Option<Pattern> {
        let pattern = pattern?.trim();

        // Empty or "*" means match all
        if pattern.is_empty() || pattern == "*" {
            return None;
        }

        // Compile the pattern
        Pattern::new(pattern).ok()
    }

    /// Check if the given scope matches this matcher's patterns.
    ///
    /// Returns true if both workspace and branch match (or pattern is None/empty).
    pub fn matches(&self, scope: &PermissionScope) -> bool {
        self.matches_workspace(&scope.workspace) && self.matches_branch(&scope.branch)
    }

    /// Check if a workspace matches this matcher's workspace pattern.
    ///
    /// Returns true if:
    /// - No workspace pattern is set (matches all)
    /// - The workspace matches the pattern
    pub fn matches_workspace(&self, workspace: &str) -> bool {
        match &self.workspace_pattern {
            Some(pattern) => pattern.matches(workspace),
            None => true, // No pattern = matches all
        }
    }

    /// Check if a branch matches this matcher's branch pattern.
    ///
    /// Returns true if:
    /// - No branch pattern is set (matches all)
    /// - The branch matches the pattern
    pub fn matches_branch(&self, branch: &str) -> bool {
        match &self.branch_pattern {
            Some(pattern) => pattern.matches(branch),
            None => true, // No pattern = matches all
        }
    }

    /// Returns true if this matcher has no patterns (matches everything).
    pub fn is_unrestricted(&self) -> bool {
        self.workspace_pattern.is_none() && self.branch_pattern.is_none()
    }

    /// Get the raw workspace pattern string (if any).
    pub fn workspace_pattern_str(&self) -> Option<&str> {
        self.workspace_raw.as_deref()
    }

    /// Get the raw branch pattern string (if any).
    pub fn branch_pattern_str(&self) -> Option<&str> {
        self.branch_raw.as_deref()
    }
}

impl Default for ScopeMatcher {
    /// Default scope matcher that matches everything.
    fn default() -> Self {
        Self {
            workspace_pattern: None,
            workspace_raw: None,
            branch_pattern: None,
            branch_raw: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== PermissionScope Tests ==========

    #[test]
    fn test_permission_scope_new() {
        let scope = PermissionScope::new("content", "main");
        assert_eq!(scope.workspace, "content");
        assert_eq!(scope.branch, "main");
    }

    #[test]
    fn test_permission_scope_any() {
        let scope = PermissionScope::any();
        assert_eq!(scope.workspace, "");
        assert_eq!(scope.branch, "");
    }

    // ========== ScopeMatcher - Workspace Pattern Tests ==========

    #[test]
    fn test_exact_workspace_match() {
        let matcher = ScopeMatcher::new(Some("content"), None);
        assert!(matcher.matches_workspace("content"));
        assert!(!matcher.matches_workspace("media"));
        assert!(!matcher.matches_workspace("content-us"));
    }

    #[test]
    fn test_workspace_wildcard_suffix() {
        let matcher = ScopeMatcher::new(Some("content-*"), None);
        assert!(matcher.matches_workspace("content-us"));
        assert!(matcher.matches_workspace("content-eu"));
        assert!(matcher.matches_workspace("content-asia"));
        assert!(!matcher.matches_workspace("content"));
        assert!(!matcher.matches_workspace("media"));
    }

    #[test]
    fn test_workspace_wildcard_prefix() {
        let matcher = ScopeMatcher::new(Some("*-content"), None);
        assert!(matcher.matches_workspace("draft-content"));
        assert!(matcher.matches_workspace("published-content"));
        assert!(!matcher.matches_workspace("content"));
        assert!(!matcher.matches_workspace("content-draft"));
    }

    #[test]
    fn test_workspace_empty_matches_all() {
        let matcher = ScopeMatcher::new(Some(""), None);
        assert!(matcher.matches_workspace("content"));
        assert!(matcher.matches_workspace("media"));
        assert!(matcher.matches_workspace("anything"));
    }

    #[test]
    fn test_workspace_none_matches_all() {
        let matcher = ScopeMatcher::new(None, None);
        assert!(matcher.matches_workspace("content"));
        assert!(matcher.matches_workspace("media"));
        assert!(matcher.matches_workspace("anything"));
    }

    #[test]
    fn test_workspace_star_matches_all() {
        let matcher = ScopeMatcher::new(Some("*"), None);
        assert!(matcher.matches_workspace("content"));
        assert!(matcher.matches_workspace("media"));
        assert!(matcher.matches_workspace("anything"));
    }

    // ========== ScopeMatcher - Branch Pattern Tests ==========

    #[test]
    fn test_exact_branch_match() {
        let matcher = ScopeMatcher::new(None, Some("main"));
        assert!(matcher.matches_branch("main"));
        assert!(!matcher.matches_branch("develop"));
        assert!(!matcher.matches_branch("main-backup"));
    }

    #[test]
    fn test_branch_wildcard_path_style() {
        // Note: glob's '*' matches any characters, including '/'
        let matcher = ScopeMatcher::new(None, Some("features/*"));
        assert!(matcher.matches_branch("features/auth"));
        assert!(matcher.matches_branch("features/login"));
        assert!(!matcher.matches_branch("features"));
        assert!(!matcher.matches_branch("hotfix/bug"));
    }

    #[test]
    fn test_branch_release_pattern() {
        let matcher = ScopeMatcher::new(None, Some("release-*"));
        assert!(matcher.matches_branch("release-1.0"));
        assert!(matcher.matches_branch("release-2.0.1"));
        assert!(matcher.matches_branch("release-v3"));
        assert!(!matcher.matches_branch("release"));
        assert!(!matcher.matches_branch("hotfix-1.0"));
    }

    #[test]
    fn test_branch_empty_matches_all() {
        let matcher = ScopeMatcher::new(None, Some(""));
        assert!(matcher.matches_branch("main"));
        assert!(matcher.matches_branch("develop"));
        assert!(matcher.matches_branch("features/auth"));
    }

    #[test]
    fn test_branch_none_matches_all() {
        let matcher = ScopeMatcher::new(None, None);
        assert!(matcher.matches_branch("main"));
        assert!(matcher.matches_branch("develop"));
        assert!(matcher.matches_branch("features/auth"));
    }

    #[test]
    fn test_branch_star_matches_all() {
        let matcher = ScopeMatcher::new(None, Some("*"));
        assert!(matcher.matches_branch("main"));
        assert!(matcher.matches_branch("develop"));
        // Note: '*' in glob matches path separators too
        assert!(matcher.matches_branch("features/auth"));
    }

    // ========== ScopeMatcher - Combined Tests ==========

    #[test]
    fn test_scope_both_patterns() {
        let matcher = ScopeMatcher::new(Some("content"), Some("main"));
        let scope_match = PermissionScope::new("content", "main");
        let scope_wrong_ws = PermissionScope::new("media", "main");
        let scope_wrong_branch = PermissionScope::new("content", "develop");
        let scope_both_wrong = PermissionScope::new("media", "develop");

        assert!(matcher.matches(&scope_match));
        assert!(!matcher.matches(&scope_wrong_ws));
        assert!(!matcher.matches(&scope_wrong_branch));
        assert!(!matcher.matches(&scope_both_wrong));
    }

    #[test]
    fn test_scope_workspace_only() {
        let matcher = ScopeMatcher::new(Some("content-*"), None);
        let scope1 = PermissionScope::new("content-us", "main");
        let scope2 = PermissionScope::new("content-eu", "develop");
        let scope3 = PermissionScope::new("media", "main");

        assert!(matcher.matches(&scope1));
        assert!(matcher.matches(&scope2));
        assert!(!matcher.matches(&scope3));
    }

    #[test]
    fn test_scope_branch_only() {
        let matcher = ScopeMatcher::new(None, Some("features/*"));
        let scope1 = PermissionScope::new("content", "features/auth");
        let scope2 = PermissionScope::new("media", "features/login");
        let scope3 = PermissionScope::new("content", "main");

        assert!(matcher.matches(&scope1));
        assert!(matcher.matches(&scope2));
        assert!(!matcher.matches(&scope3));
    }

    #[test]
    fn test_scope_unrestricted() {
        let matcher = ScopeMatcher::new(None, None);
        assert!(matcher.is_unrestricted());

        let restricted = ScopeMatcher::new(Some("content"), None);
        assert!(!restricted.is_unrestricted());
    }

    // ========== ScopeMatcher - Default Tests ==========

    #[test]
    fn test_default_matches_everything() {
        let matcher = ScopeMatcher::default();
        let scope = PermissionScope::new("any-workspace", "any-branch");
        assert!(matcher.matches(&scope));
        assert!(matcher.is_unrestricted());
    }

    // ========== ScopeMatcher - Edge Cases ==========

    #[test]
    fn test_special_characters_in_workspace() {
        let matcher = ScopeMatcher::new(Some("raisin:access_control"), None);
        assert!(matcher.matches_workspace("raisin:access_control"));
        assert!(!matcher.matches_workspace("raisin:other"));
    }

    #[test]
    fn test_question_mark_wildcard() {
        let matcher = ScopeMatcher::new(Some("content-?"), None);
        assert!(matcher.matches_workspace("content-a"));
        assert!(matcher.matches_workspace("content-1"));
        assert!(!matcher.matches_workspace("content-us")); // '?' matches single char
        assert!(!matcher.matches_workspace("content-"));
    }
}
