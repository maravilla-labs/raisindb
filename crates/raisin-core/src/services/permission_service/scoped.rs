// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Scoped API keys: a service principal that holds exactly a set of roles.
//!
//! A scoped key is not a person and has no `raisin:User` node. It is resolved
//! with the SAME role inheritance and permission extraction a user's roles
//! get ([`PermissionService::resolve_role_inheritance`],
//! [`PermissionService::extract_permissions_from_role`]), so a key holding
//! `studio_viewer` can do exactly what a user holding only `studio_viewer`
//! can, minus anything that depends on a home (it has none: `auth.home`
//! conditions never match).

use std::collections::HashSet;

use raisin_error::Result;
use raisin_models::api_key::ApiKey;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_storage::Storage;

use super::{PermissionService, ACCESS_CONTROL_WORKSPACE};

/// Role ids that ARE administration, whatever they grant.
const ADMIN_ROLE_IDS: &[&str] = &["system_admin", "admin"];

/// The principal id a scoped key acts as. Never a user id, never an admin
/// user id: audit logs and `created_by` show which key wrote.
pub fn scoped_key_principal(key_id: &str) -> String {
    format!("apikey:{key_id}")
}

/// Why a role set may not be given to a scoped key, or `None`.
///
/// Administration is refused in its three spellings: an admin role id
/// (anywhere in the inheritance chain), an UNCONDITIONAL write to the access
/// control workspace (whoever can write roles and users can make themselves
/// admin), and an unconditional write to every workspace (`workspace` unset
/// matches `raisin:access_control` too). Conditional ACL grants, such as
/// `authenticated_user`'s "update your own profile", stay allowed: their
/// conditions name `auth.home` / `auth.local_user_id`, which a key does not
/// have, so they never match.
pub fn admin_grant_in(resolved: &ResolvedPermissions) -> Option<String> {
    if resolved.is_system_admin {
        return Some("it inherits system_admin".to_string());
    }
    if let Some(r) = resolved
        .effective_roles
        .iter()
        .find(|r| ADMIN_ROLE_IDS.contains(&r.as_str()))
    {
        return Some(format!("it inherits the admin role '{r}'"));
    }
    let writes = |p: &Permission| {
        p.operations
            .iter()
            .any(|o| matches!(o, Operation::Create | Operation::Update | Operation::Delete))
    };
    for p in &resolved.permissions {
        if p.condition.as_deref().is_some_and(|c| !c.trim().is_empty()) || !writes(p) {
            continue;
        }
        match p.workspace.as_deref().map(str::trim) {
            None | Some("") | Some("*") | Some("**") => {
                return Some(format!(
                    "it may write '{}' in EVERY workspace, raisin:access_control included",
                    p.path
                ))
            }
            Some(ws) if ws == ACCESS_CONTROL_WORKSPACE => {
                return Some(format!(
                    "it may write '{}' in {ACCESS_CONTROL_WORKSPACE} (roles and users) without a condition",
                    p.path
                ))
            }
            _ => {}
        }
    }
    None
}

impl<S: Storage> PermissionService<S> {
    /// Whether `role_id` exists in the repository's access control workspace.
    pub async fn role_exists(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        role_id: &str,
    ) -> Result<bool> {
        Ok(self
            .find_role_by_id(tenant_id, repo_id, branch, role_id)
            .await?
            .is_some())
    }

    /// Resolve a principal that holds exactly `roles` (plus what they
    /// inherit), the way a user's roles are resolved. No groups, no home.
    pub async fn resolve_for_roles(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        principal_id: &str,
        roles: &[String],
    ) -> Result<ResolvedPermissions> {
        let direct: Vec<String> = roles
            .iter()
            .map(|r| r.strip_prefix("/roles/").unwrap_or(r).to_string())
            .collect();
        let initial: HashSet<String> = direct.iter().cloned().collect();
        let effective = self
            .resolve_role_inheritance(tenant_id, repo_id, branch, &initial)
            .await?;
        let mut permissions = Vec::new();
        for role_id in &effective {
            if let Some(role_node) = self
                .find_role_by_id(tenant_id, repo_id, branch, role_id)
                .await?
            {
                permissions.extend(self.extract_permissions_from_role(&role_node));
            }
        }
        let mut effective_roles: Vec<String> = effective.into_iter().collect();
        effective_roles.sort();
        Ok(ResolvedPermissions {
            user_id: principal_id.to_string(),
            email: None,
            direct_roles: direct,
            group_roles: Vec::new(),
            is_system_admin: effective_roles.iter().any(|r| r == "system_admin"),
            effective_roles,
            groups: Vec::new(),
            permissions,
            home: None,
            resolved_at: Some(std::time::Instant::now()),
        })
    }

    /// THE resolver for a scoped key, shared by every wire (HTTP, pgwire), so
    /// a key cannot mean different things depending on how it arrived.
    ///
    /// Returns `None` for an unscoped key (its owner's authority applies, as
    /// before). For a scoped key: an RLS context for its service principal
    /// with exactly its roles, never `is_system`, never anonymous. A role set
    /// that has become administrative since the key was minted (a role edited
    /// to inherit `system_admin`) resolves to NO permissions rather than to
    /// admin: the check made at creation is made again on every use.
    pub async fn resolve_scoped_key(
        &self,
        key: &ApiKey,
        branch: &str,
    ) -> Result<Option<AuthContext>> {
        let Some(scope) = &key.scope else {
            return Ok(None);
        };
        let principal = scoped_key_principal(&key.key_id);
        let mut resolved = self
            .resolve_for_roles(
                &key.tenant_id,
                &scope.repo,
                branch,
                &principal,
                &scope.roles,
            )
            .await?;
        if let Some(reason) = admin_grant_in(&resolved) {
            tracing::error!(
                key = %key.key_prefix,
                repo = %scope.repo,
                reason = %reason,
                "scoped API key's roles now grant administration; the key resolves to no permissions"
            );
            resolved.permissions.clear();
            resolved.effective_roles.clear();
            resolved.direct_roles.clear();
            resolved.is_system_admin = false;
        }
        Ok(Some(
            AuthContext::for_user(principal).with_permissions(resolved),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perm(workspace: Option<&str>, ops: &[Operation], condition: Option<&str>) -> Permission {
        let mut p: Permission = serde_json::from_value(serde_json::json!({
            "path": "/**",
            "operations": [],
        }))
        .unwrap();
        p.workspace = workspace.map(str::to_string);
        p.operations = ops.to_vec();
        p.condition = condition.map(str::to_string);
        p
    }

    fn resolved(roles: &[&str], permissions: Vec<Permission>) -> ResolvedPermissions {
        ResolvedPermissions {
            user_id: "apikey:k".into(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: roles.iter().map(|r| r.to_string()).collect(),
            groups: vec![],
            permissions,
            is_system_admin: roles.contains(&"system_admin"),
            home: None,
            resolved_at: None,
        }
    }

    #[test]
    fn a_read_only_role_is_not_admin() {
        let r = resolved(
            &["studio_viewer"],
            vec![
                perm(Some("stories"), &[Operation::Read], None),
                perm(None, &[Operation::Read], None),
                // authenticated_user: own profile, conditioned on the home.
                perm(
                    Some(ACCESS_CONTROL_WORKSPACE),
                    &[Operation::Read, Operation::Update],
                    Some("node.path == auth.home + '/profile'"),
                ),
            ],
        );
        assert_eq!(admin_grant_in(&r), None);
    }

    #[test]
    fn admin_in_any_spelling_is_refused() {
        assert!(admin_grant_in(&resolved(&["system_admin"], vec![])).is_some());
        assert!(admin_grant_in(&resolved(&["editor", "admin"], vec![])).is_some());
        // The engine-seeded `editor`: write on every workspace.
        assert!(admin_grant_in(&resolved(
            &["editor"],
            vec![perm(None, &[Operation::Create, Operation::Update], None)]
        ))
        .is_some());
        assert!(admin_grant_in(&resolved(
            &["acl_writer"],
            vec![perm(
                Some(ACCESS_CONTROL_WORKSPACE),
                &[Operation::Update],
                None
            )]
        ))
        .is_some());
        assert!(admin_grant_in(&resolved(
            &["all"],
            vec![perm(Some("*"), &[Operation::Delete], None)]
        ))
        .is_some());
    }

    #[test]
    fn the_principal_names_the_key_not_a_user() {
        assert_eq!(scoped_key_principal("1234"), "apikey:1234");
    }
}
