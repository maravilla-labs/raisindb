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

//! Permission resolution service.
//!
//! This service resolves a user's effective permissions from:
//! - User -> direct roles
//! - User -> groups -> group roles
//! - Role -> inherited roles (recursive)
//!
//! The result is a flattened set of permissions that can be cached.

mod cached;
mod parsing;

pub use cached::CachedPermissionService;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Permission, ResolvedPermissions};
use raisin_storage::{Storage, StorageScope};

use parsing::{as_array, as_string, extract_string_array, parse_permission};

/// The workspace where access control entities are stored
pub const ACCESS_CONTROL_WORKSPACE: &str = "raisin:access_control";

/// Permission resolution service.
///
/// Resolves users' effective permissions from the access_control workspace.
pub struct PermissionService<S: Storage> {
    storage: Arc<S>,
}

impl<S: Storage> PermissionService<S> {
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }

    /// Resolve permissions for a user by their email.
    pub async fn resolve_for_email(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        email: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let user = self
            .find_user_by_email(tenant_id, repo_id, branch, email)
            .await?;

        match user {
            Some(user_node) => {
                let resolved = self
                    .resolve_for_user_node(tenant_id, repo_id, branch, &user_node)
                    .await?;
                Ok(Some(resolved))
            }
            None => Ok(None),
        }
    }

    /// Resolve permissions for a user by their node ID.
    pub async fn resolve_for_user_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        user_id: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let user = self
            .get_node_by_id(tenant_id, repo_id, branch, user_id)
            .await?;

        match user {
            Some(user_node) if user_node.node_type == "raisin:User" => {
                let resolved = self
                    .resolve_for_user_node(tenant_id, repo_id, branch, &user_node)
                    .await?;
                Ok(Some(resolved))
            }
            _ => Ok(None),
        }
    }

    /// Resolve permissions for a user by their identity ID (from JWT sub claim).
    ///
    /// This is different from `resolve_for_user_id` which looks up by node UUID.
    /// This function finds the raisin:User node by the `user_id` property,
    /// which stores the identity_id from the global authentication system.
    pub async fn resolve_for_identity_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        identity_id: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let user = self
            .find_user_by_identity_id(tenant_id, repo_id, branch, identity_id)
            .await?;

        match user {
            Some(user_node) => {
                let resolved = self
                    .resolve_for_user_node(tenant_id, repo_id, branch, &user_node)
                    .await?;
                Ok(Some(resolved))
            }
            None => Ok(None),
        }
    }

    /// Resolve permissions for the anonymous role (DEPRECATED - use resolve_anonymous_user).
    ///
    /// This method directly extracts permissions from the anonymous role.
    /// It's kept for backwards compatibility but `resolve_anonymous_user()` should be preferred
    /// as it uses the physical anonymous user node for proper role-based permission inheritance.
    pub async fn resolve_anonymous(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<ResolvedPermissions> {
        let anonymous_role = self
            .find_role_by_id(tenant_id, repo_id, branch, "anonymous")
            .await?;

        let permissions = match anonymous_role {
            Some(role_node) => self.extract_permissions_from_role(&role_node),
            None => vec![],
        };

        Ok(ResolvedPermissions::anonymous(permissions))
    }

    /// Resolve permissions for anonymous access using the physical anonymous user node.
    ///
    /// This is the preferred method for anonymous access. It:
    /// 1. Finds the physical `raisin:User` node with `user_id: "anonymous"`
    /// 2. Resolves permissions through the normal user resolution path (roles, groups, inheritance)
    pub async fn resolve_anonymous_user(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<ResolvedPermissions>> {
        let user_node = self
            .find_user_by_identity_id(tenant_id, repo_id, branch, "anonymous")
            .await?;

        match user_node {
            Some(node) => {
                let resolved = self
                    .resolve_for_user_node(tenant_id, repo_id, branch, &node)
                    .await?;

                tracing::info!(
                    user_id = %resolved.user_id,
                    permissions_count = resolved.permissions.len(),
                    effective_roles = ?resolved.effective_roles,
                    is_system_admin = resolved.is_system_admin,
                    "Resolved anonymous user permissions"
                );

                Ok(Some(resolved))
            }
            None => {
                tracing::warn!(
                    tenant_id = tenant_id,
                    repo_id = repo_id,
                    branch = branch,
                    "Physical anonymous user not found at /users/system/anonymous, falling back to role-only resolution"
                );
                let resolved = self.resolve_anonymous(tenant_id, repo_id, branch).await?;
                Ok(Some(resolved))
            }
        }
    }

    /// Resolve permissions for a user node.
    async fn resolve_for_user_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        user_node: &Node,
    ) -> Result<ResolvedPermissions> {
        let user_id = user_node.id.clone();
        let email = user_node
            .properties
            .get("email")
            .and_then(as_string)
            .map(|s| s.to_string());

        let direct_roles = extract_string_array(&user_node.properties, "roles");
        let groups = extract_string_array(&user_node.properties, "groups");

        // Resolve group roles
        let mut group_roles = Vec::new();
        for group_id in &groups {
            if let Some(group_node) = self
                .find_group_by_id(tenant_id, repo_id, branch, group_id)
                .await?
            {
                let roles = extract_string_array(&group_node.properties, "roles");
                group_roles.extend(roles);
            }
        }

        // Combine and deduplicate roles
        let mut all_role_ids: HashSet<String> = HashSet::new();
        all_role_ids.extend(direct_roles.clone());
        all_role_ids.extend(group_roles.clone());

        // Resolve role inheritance
        let effective_roles = self
            .resolve_role_inheritance(tenant_id, repo_id, branch, &all_role_ids)
            .await?;

        let is_system_admin = effective_roles.contains("system_admin");

        // Collect all permissions
        let mut permissions = Vec::new();
        for role_id in &effective_roles {
            if let Some(role_node) = self
                .find_role_by_id(tenant_id, repo_id, branch, role_id)
                .await?
            {
                let role_permissions = self.extract_permissions_from_role(&role_node);
                permissions.extend(role_permissions);
            }
        }

        Ok(ResolvedPermissions {
            user_id,
            email,
            direct_roles,
            group_roles,
            effective_roles: effective_roles.into_iter().collect(),
            groups,
            permissions,
            is_system_admin,
            resolved_at: Some(std::time::Instant::now()),
        })
    }

    /// Resolve role inheritance with cycle detection.
    async fn resolve_role_inheritance(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        initial_roles: &HashSet<String>,
    ) -> Result<HashSet<String>> {
        let mut all_roles = initial_roles.clone();
        let mut to_process: Vec<String> = initial_roles.iter().cloned().collect();
        let mut visited: HashSet<String> = HashSet::new();

        while let Some(role_id) = to_process.pop() {
            if visited.contains(&role_id) {
                continue;
            }
            visited.insert(role_id.clone());

            if let Some(role_node) = self
                .find_role_by_id(tenant_id, repo_id, branch, &role_id)
                .await?
            {
                let inherits = extract_string_array(&role_node.properties, "inherits");
                for inherited_role in inherits {
                    if all_roles.insert(inherited_role.clone()) {
                        to_process.push(inherited_role);
                    }
                }
            }
        }

        Ok(all_roles)
    }

    /// Extract permissions from a role node.
    fn extract_permissions_from_role(&self, role_node: &Node) -> Vec<Permission> {
        let Some(permissions_value) = role_node.properties.get("permissions") else {
            return vec![];
        };

        let Some(permissions_array) = as_array(permissions_value) else {
            return vec![];
        };

        permissions_array
            .iter()
            .filter_map(parse_permission)
            .collect()
    }

    // === Storage helpers ===

    async fn find_user_by_email(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        email: &str,
    ) -> Result<Option<Node>> {
        use raisin_storage::NodeRepository;

        let scope = StorageScope::new(tenant_id, repo_id, branch, ACCESS_CONTROL_WORKSPACE);
        let email_value = PropertyValue::String(email.to_string());
        let nodes = self
            .storage
            .nodes()
            .find_by_property(scope, "email", &email_value)
            .await?;

        Ok(nodes.into_iter().find(|n| n.node_type == "raisin:User"))
    }

    /// Find a user by their identity ID (from global auth system).
    ///
    /// This looks up a raisin:User node by the `user_id` property,
    /// which stores the identity_id from the global authentication system.
    /// This is different from `get_node_by_id` which looks up by node UUID.
    async fn find_user_by_identity_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        identity_id: &str,
    ) -> Result<Option<Node>> {
        use raisin_storage::NodeRepository;

        eprintln!(
            "[DEBUG find_user_by_identity_id] Searching for user with identity_id='{}' in tenant='{}', repo='{}', branch='{}', workspace='{}'",
            identity_id, tenant_id, repo_id, branch, ACCESS_CONTROL_WORKSPACE
        );

        let scope = StorageScope::new(tenant_id, repo_id, branch, ACCESS_CONTROL_WORKSPACE);
        let identity_id_value = PropertyValue::String(identity_id.to_string());
        let nodes = self
            .storage
            .nodes()
            .find_by_property(scope, "user_id", &identity_id_value)
            .await?;

        eprintln!(
            "[DEBUG find_user_by_identity_id] Found {} nodes with user_id property = '{}'",
            nodes.len(),
            identity_id
        );

        for (i, node) in nodes.iter().enumerate() {
            eprintln!(
                "[DEBUG find_user_by_identity_id] Node {}: id='{}', type='{}', path='{}', properties={:?}",
                i, node.id, node.node_type, node.path, node.properties.keys().collect::<Vec<_>>()
            );
        }

        let result = nodes.into_iter().find(|n| n.node_type == "raisin:User");

        if let Some(ref user) = result {
            eprintln!(
                "[DEBUG find_user_by_identity_id] Found raisin:User node: id='{}', path='{}'",
                user.id, user.path
            );
        } else {
            eprintln!(
                "[DEBUG find_user_by_identity_id] No raisin:User node found for identity_id='{}'",
                identity_id
            );
        }

        Ok(result)
    }

    async fn find_role_by_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        role_id: &str,
    ) -> Result<Option<Node>> {
        use raisin_storage::NodeRepository;

        let scope = StorageScope::new(tenant_id, repo_id, branch, ACCESS_CONTROL_WORKSPACE);
        let role_id_value = PropertyValue::String(role_id.to_string());
        let nodes = self
            .storage
            .nodes()
            .find_by_property(scope, "role_id", &role_id_value)
            .await?;

        Ok(nodes.into_iter().find(|n| n.node_type == "raisin:Role"))
    }

    async fn find_group_by_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        group_name: &str,
    ) -> Result<Option<Node>> {
        use raisin_storage::NodeRepository;

        let scope = StorageScope::new(tenant_id, repo_id, branch, ACCESS_CONTROL_WORKSPACE);
        let group_name_value = PropertyValue::String(group_name.to_string());
        let nodes = self
            .storage
            .nodes()
            .find_by_property(scope, "name", &group_name_value)
            .await?;

        Ok(nodes.into_iter().find(|n| n.node_type == "raisin:Group"))
    }

    async fn get_node_by_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
    ) -> Result<Option<Node>> {
        use raisin_storage::NodeRepository;

        let scope = StorageScope::new(tenant_id, repo_id, branch, ACCESS_CONTROL_WORKSPACE);
        self.storage.nodes().get(scope, node_id, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_string_array() {
        let mut props = HashMap::new();
        props.insert(
            "roles".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::String("admin".to_string()),
                PropertyValue::String("editor".to_string()),
            ]),
        );

        let result = extract_string_array(&props, "roles");
        assert_eq!(result, vec!["admin", "editor"]);

        let empty = extract_string_array(&props, "nonexistent");
        assert!(empty.is_empty());
    }
}
