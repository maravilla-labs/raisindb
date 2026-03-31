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

//! Authentication context for request handling.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::permissions::ResolvedPermissions;

/// Authentication and authorization context for a request.
///
/// This struct carries identity and permission information through the system,
/// from HTTP handlers down to the storage layer.
///
/// # User Types
///
/// RaisinDB distinguishes between two types of users:
///
/// 1. **Admin Users** - Operators with backend authentication (JWT)
///    - Access admin console, CLI, API, pgwire
///    - Stored in `admin_user_store.rs`
///    - Can impersonate regular users for testing
///
/// 2. **Regular Users** - Application end-users (`raisin:User` nodes)
///    - Stored in `raisin:access_control` workspace
///    - Have roles, groups, permissions
///    - Identity provided by external auth or API caller
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthContext {
    /// The user ID - global identity_id from JWT sub claim
    /// Used for ownership checks like `created_by == $auth.user_id`
    pub user_id: Option<String>,

    /// The local raisin:User node ID (workspace-specific UUID)
    /// Available as `$auth.local_user_id` in permission conditions
    pub local_user_id: Option<String>,

    /// User's email (for condition variable resolution)
    pub email: Option<String>,

    /// Effective role IDs
    pub roles: Vec<String>,

    /// Group IDs the user belongs to
    pub groups: Vec<String>,

    /// Whether this is a system operation (bypasses RLS)
    /// Only used for migrations and bootstrap operations
    pub is_system: bool,

    /// Whether this is an anonymous (unauthenticated) request
    pub is_anonymous: bool,

    /// Resolved permissions (lazily loaded and cached)
    #[serde(skip)]
    pub resolved_permissions: Option<Arc<ResolvedPermissions>>,

    /// If this context is impersonated, the admin user ID who is impersonating
    pub impersonated_by: Option<String>,

    /// If acting as a steward, the ward's user ID
    pub acting_as_ward: Option<String>,

    /// Active stewardship source (relation type or override ID)
    pub active_stewardship_source: Option<String>,

    /// User's home path in the repository (the raisin:User node path)
    /// Available as `$auth.home` in REL permission conditions
    /// Used for path-based access control (e.g., `node.path.descendantOf($auth.home)`)
    pub home: Option<String>,
}

impl Default for AuthContext {
    fn default() -> Self {
        Self::anonymous()
    }
}

impl AuthContext {
    /// Create an anonymous (unauthenticated) context.
    ///
    /// Anonymous users have no identity and only get permissions
    /// from the `anonymous` role (if configured).
    pub fn anonymous() -> Self {
        AuthContext {
            user_id: None,
            local_user_id: None,
            email: None,
            roles: vec![],
            groups: vec![],
            is_system: false,
            is_anonymous: true,
            resolved_permissions: None,
            impersonated_by: None,
            acting_as_ward: None,
            active_stewardship_source: None,
            home: None,
        }
    }

    /// Create a system context that bypasses all permission checks.
    ///
    /// Only use for:
    /// - Database migrations
    /// - Initial bootstrap operations
    /// - Internal system operations
    ///
    /// **WARNING**: This context has full access to everything.
    pub fn system() -> Self {
        AuthContext {
            user_id: Some("system".to_string()),
            local_user_id: None,
            email: None,
            roles: vec!["system_admin".to_string()],
            groups: vec![],
            is_system: true,
            is_anonymous: false,
            resolved_permissions: Some(Arc::new(ResolvedPermissions::system_admin())),
            impersonated_by: None,
            acting_as_ward: None,
            active_stewardship_source: None,
            home: None,
        }
    }

    /// Create a context that denies all access.
    ///
    /// Use when:
    /// - Anonymous access is disabled and no auth token is provided
    /// - Default deny-by-default behavior is needed
    ///
    /// This context has no permissions and RLS filter will deny all operations.
    pub fn deny_all() -> Self {
        AuthContext {
            user_id: None,
            local_user_id: None,
            email: None,
            roles: vec![],
            groups: vec![],
            is_system: false,
            is_anonymous: false, // Not anonymous - explicitly denied
            resolved_permissions: Some(Arc::new(ResolvedPermissions::deny_all())),
            impersonated_by: None,
            acting_as_ward: None,
            active_stewardship_source: None,
            home: None,
        }
    }

    /// Create a context for a regular user.
    ///
    /// The user's permissions should be resolved separately and attached
    /// using `with_permissions()`.
    pub fn for_user(user_id: impl Into<String>) -> Self {
        AuthContext {
            user_id: Some(user_id.into()),
            local_user_id: None,
            email: None,
            roles: vec![],
            groups: vec![],
            is_system: false,
            is_anonymous: false,
            resolved_permissions: None,
            impersonated_by: None,
            acting_as_ward: None,
            active_stewardship_source: None,
            home: None,
        }
    }

    /// Create an impersonated context.
    ///
    /// Used when an admin user wants to test permissions as another user.
    pub fn impersonated(user_id: impl Into<String>, admin_user_id: impl Into<String>) -> Self {
        AuthContext {
            user_id: Some(user_id.into()),
            local_user_id: None,
            email: None,
            roles: vec![],
            groups: vec![],
            is_system: false,
            is_anonymous: false,
            resolved_permissions: None,
            impersonated_by: Some(admin_user_id.into()),
            acting_as_ward: None,
            active_stewardship_source: None,
            home: None,
        }
    }

    /// Set the user's email
    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// Set the user's roles
    pub fn with_roles(mut self, roles: Vec<String>) -> Self {
        self.roles = roles;
        self
    }

    /// Set the user's groups
    pub fn with_groups(mut self, groups: Vec<String>) -> Self {
        self.groups = groups;
        self
    }

    /// Set the local user node ID (workspace-specific)
    pub fn with_local_user_id(mut self, local_user_id: impl Into<String>) -> Self {
        self.local_user_id = Some(local_user_id.into());
        self
    }

    /// Set the user's home path (the raisin:User node path)
    /// Used for path-based access control in REL conditions
    pub fn with_home(mut self, home: impl Into<String>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// Attach resolved permissions
    ///
    /// This also captures the local raisin:User node ID from the resolved permissions.
    pub fn with_permissions(mut self, permissions: ResolvedPermissions) -> Self {
        // Also sync roles and groups from resolved permissions
        self.roles = permissions.effective_roles.clone();
        self.groups = permissions.groups.clone();
        self.email = permissions.email.clone();
        // Capture the local user node ID (workspace-specific)
        self.local_user_id = Some(permissions.user_id.clone());
        self.resolved_permissions = Some(Arc::new(permissions));
        self
    }

    /// Check if this context has been fully resolved (permissions loaded)
    pub fn is_resolved(&self) -> bool {
        self.is_system || self.resolved_permissions.is_some()
    }

    /// Check if this context bypasses permission checks
    pub fn bypasses_rls(&self) -> bool {
        self.is_system
    }

    /// Check if this context is impersonated
    pub fn is_impersonated(&self) -> bool {
        self.impersonated_by.is_some()
    }

    /// Get the resolved permissions (if available)
    pub fn permissions(&self) -> Option<&ResolvedPermissions> {
        self.resolved_permissions.as_deref()
    }

    /// Check if the user has a specific role
    pub fn has_role(&self, role_id: &str) -> bool {
        if self.is_system {
            return true;
        }
        self.roles.iter().any(|r| r == role_id)
    }

    /// Check if the user is in a specific group
    pub fn in_group(&self, group_id: &str) -> bool {
        if self.is_system {
            return true;
        }
        self.groups.iter().any(|g| g == group_id)
    }

    /// Get the user ID for audit logging
    pub fn actor_id(&self) -> String {
        if let Some(ward_id) = &self.acting_as_ward {
            format!(
                "{}:acting_as:{}",
                self.user_id.as_deref().unwrap_or("unknown"),
                ward_id
            )
        } else if let Some(impersonator) = &self.impersonated_by {
            format!(
                "{}:impersonating:{}",
                impersonator,
                self.user_id.as_deref().unwrap_or("unknown")
            )
        } else if let Some(user_id) = &self.user_id {
            user_id.clone()
        } else if self.is_anonymous {
            "anonymous".to_string()
        } else {
            "unknown".to_string()
        }
    }

    /// Set the ward this user is acting on behalf of
    pub fn acting_as(
        mut self,
        ward_id: impl Into<String>,
        stewardship_source: impl Into<String>,
    ) -> Self {
        self.acting_as_ward = Some(ward_id.into());
        self.active_stewardship_source = Some(stewardship_source.into());
        self
    }

    /// Check if this context is acting as a steward for a ward
    pub fn is_acting_as_steward(&self) -> bool {
        self.acting_as_ward.is_some()
    }

    /// Get the ward ID if acting as steward
    pub fn ward_id(&self) -> Option<&str> {
        self.acting_as_ward.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anonymous_context() {
        let ctx = AuthContext::anonymous();
        assert!(ctx.is_anonymous);
        assert!(!ctx.is_system);
        assert!(!ctx.bypasses_rls());
        assert_eq!(ctx.actor_id(), "anonymous");
    }

    #[test]
    fn test_system_context() {
        let ctx = AuthContext::system();
        assert!(ctx.is_system);
        assert!(ctx.bypasses_rls());
        assert!(ctx.has_role("anything"));
        assert!(ctx.in_group("anything"));
    }

    #[test]
    fn test_user_context() {
        let ctx = AuthContext::for_user("user123")
            .with_email("user@example.com")
            .with_roles(vec!["editor".to_string()])
            .with_groups(vec!["team-a".to_string()]);

        assert!(!ctx.is_anonymous);
        assert!(!ctx.is_system);
        assert!(ctx.has_role("editor"));
        assert!(!ctx.has_role("admin"));
        assert!(ctx.in_group("team-a"));
        assert_eq!(ctx.actor_id(), "user123");
    }

    #[test]
    fn test_impersonated_context() {
        let ctx = AuthContext::impersonated("target_user", "admin_user");

        assert!(ctx.is_impersonated());
        assert_eq!(ctx.actor_id(), "admin_user:impersonating:target_user");
    }
}
