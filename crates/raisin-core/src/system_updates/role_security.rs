// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Security system update: a signed-in user may not write what decides
//! their own rights.
//!
//! Repositories created before this fix hold an `authenticated_user` role
//! (`raisin:access_control:/roles/authenticated_user`) whose self grant —
//! `update` on the caller's own `raisin:User` node, `node.id ==
//! auth.local_user_id` — has no field list. PermissionService reads a user's
//! roles and groups from that node, so any signed-in user could grant
//! themselves a role. The shipped definition now limits the self update to
//! the descriptive fields; this rewrites existing repositories to match, on
//! every start, idempotently:
//!
//! - only a permission of that exact shape is changed (update on
//!   `raisin:User` under that condition, no `fields` / `except_fields`); every
//!   other permission of a customised role is kept as it is;
//! - such a permission is split into the same grant without `update` (when it
//!   granted more) and an `update` grant limited to [`SELF_EDITABLE`];
//! - a role already fixed (or never vulnerable) is not written.
//!
//! It runs whatever the definitions' `auto_apply` policy says: it closes a
//! privilege escalation, it does not change a schema.

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::node_patch::{PatchTarget, PropertyPatch};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde_json::{json, Value};

/// Where the role lives.
pub const ROLE_WORKSPACE: &str = "raisin:access_control";
/// The role node.
pub const ROLE_PATH: &str = "/roles/authenticated_user";
/// Roles live on `main` whatever the repository's default branch is.
pub const ROLE_BRANCH: &str = "main";
/// What a user may edit on their own user node.
pub const SELF_EDITABLE: [&str; 3] = ["display_name", "title", "metadata"];

const SELF_CONDITION: &str = "node.id == auth.local_user_id";

fn has(list: Option<&Value>, item: &str) -> bool {
    list.and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(item)))
}

/// The vulnerable shape: an unrestricted self update of the user node.
fn is_open_self_update(p: &Value) -> bool {
    has(p.get("operations"), "update")
        && has(p.get("node_types"), "raisin:User")
        && p.get("condition").and_then(Value::as_str).map(str::trim) == Some(SELF_CONDITION)
        && p.get("fields").is_none_or(Value::is_null)
        && p.get("except_fields").is_none_or(Value::is_null)
}

/// The fixed permission list, or `None` when nothing needs changing.
pub fn secure_permissions(permissions: &[Value]) -> Option<Vec<Value>> {
    if !permissions.iter().any(is_open_self_update) {
        return None;
    }
    let mut out = Vec::with_capacity(permissions.len() + 1);
    for p in permissions {
        if !is_open_self_update(p) {
            out.push(p.clone());
            continue;
        }
        let others: Vec<Value> = p["operations"]
            .as_array()
            .map(|ops| {
                ops.iter()
                    .filter(|o| o.as_str() != Some("update"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !others.is_empty() {
            let mut rest = p.clone();
            rest["operations"] = Value::Array(others);
            out.push(rest);
        }
        let mut update = p.clone();
        update["operations"] = json!(["update"]);
        update["fields"] = json!(SELF_EDITABLE);
        out.push(update);
    }
    Some(out)
}

/// Rewrite the `authenticated_user` role of one repository if it still has
/// the open self update. Returns whether it was written.
pub async fn secure_authenticated_user_role<S>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
) -> Result<bool>
where
    S: Storage + TransactionalStorage,
{
    let scope = StorageScope::new(tenant_id, repo_id, ROLE_BRANCH, ROLE_WORKSPACE);
    let Some(role) = storage.nodes().get_by_path(scope, ROLE_PATH, None).await? else {
        return Ok(false);
    };
    let Some(current) = role.properties.get("permissions") else {
        return Ok(false);
    };
    let Ok(Value::Array(permissions)) = serde_json::to_value(current) else {
        return Ok(false);
    };
    let Some(fixed) = secure_permissions(&permissions) else {
        return Ok(false);
    };

    let txn = storage.begin_context().await?;
    txn.set_tenant_repo(tenant_id, repo_id)?;
    txn.set_branch(ROLE_BRANCH)?;
    txn.set_actor("system-update")?;
    txn.set_message("security: authenticated_user may not write its own roles or groups")?;
    txn.set_auth_context(raisin_models::auth::AuthContext::system())?;
    txn.patch_node_properties(
        ROLE_WORKSPACE,
        &PatchTarget::Id(role.id.clone()),
        PropertyPatch::setting(vec![(
            "permissions".to_string(),
            PropertyValue::from_json(&Value::Array(fixed)),
        )]),
    )
    .await?;
    txn.commit().await?;
    tracing::warn!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        "security update: authenticated_user's self update of raisin:User limited to {:?}",
        SELF_EDITABLE
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::Node;
    use raisin_storage::{BranchRepository, CreateNodeOptions, RepoScope, WorkspaceRepository};
    use raisin_storage_memory::InMemoryStorage;

    fn old_self_grant() -> Value {
        json!({
            "path": "users/**",
            "operations": ["read", "update"],
            "node_types": ["raisin:User"],
            "workspace": "raisin:access_control",
            "condition": "node.id == auth.local_user_id"
        })
    }

    #[test]
    fn the_open_self_update_is_split_and_the_rest_is_kept() {
        let custom = json!({ "path": "projects/**", "operations": ["read"] });
        let fixed = secure_permissions(&[old_self_grant(), custom.clone()]).expect("changed");
        assert_eq!(fixed.len(), 3);
        assert_eq!(fixed[0]["operations"], json!(["read"]));
        assert!(fixed[0].get("fields").is_none());
        assert_eq!(fixed[1]["operations"], json!(["update"]));
        assert_eq!(fixed[1]["fields"], json!(SELF_EDITABLE));
        assert_eq!(fixed[1]["condition"], json!(SELF_CONDITION));
        assert_eq!(fixed[2], custom);
        // A second run changes nothing.
        assert!(secure_permissions(&fixed).is_none());
    }

    #[test]
    fn a_restricted_or_different_grant_is_left_alone() {
        let mut restricted = old_self_grant();
        restricted["fields"] = json!(["display_name"]);
        assert!(secure_permissions(&[restricted]).is_none());
        let mut other = old_self_grant();
        other["condition"] = json!("auth.roles.contains('hr')");
        assert!(secure_permissions(&[other]).is_none());
    }

    /// Old role in, fixed role out; the second run writes nothing.
    #[tokio::test]
    async fn an_existing_repository_is_secured_once() {
        let storage = InMemoryStorage::default();
        let (t, r) = ("default", "legacy");
        let _ = storage
            .branches()
            .create_branch(t, r, ROLE_BRANCH, "test", None, None, false, false)
            .await;
        let _ = storage
            .workspaces()
            .put(
                RepoScope::new(t, r),
                raisin_models::workspace::Workspace::new(ROLE_WORKSPACE.to_string()),
            )
            .await;
        let mut role = Node {
            id: "role-au".into(),
            name: "authenticated_user".into(),
            path: ROLE_PATH.into(),
            node_type: "raisin:Role".into(),
            ..Default::default()
        };
        let custom = json!({ "path": "projects/**", "operations": ["read"] });
        role.properties.insert(
            "permissions".into(),
            PropertyValue::from_json(&json!([old_self_grant(), custom])),
        );
        role.properties.insert(
            "role_id".into(),
            PropertyValue::String("authenticated_user".into()),
        );
        storage
            .nodes()
            .create(
                StorageScope::new(t, r, ROLE_BRANCH, ROLE_WORKSPACE),
                role,
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("seed role");

        assert!(secure_authenticated_user_role(&storage, t, r)
            .await
            .expect("first run"));
        let stored = storage
            .nodes()
            .get_by_path(
                StorageScope::new(t, r, ROLE_BRANCH, ROLE_WORKSPACE),
                ROLE_PATH,
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let perms = serde_json::to_value(&stored.properties["permissions"]).unwrap();
        assert_eq!(perms.as_array().unwrap().len(), 3);
        assert_eq!(perms[1]["fields"], json!(SELF_EDITABLE));
        assert_eq!(
            perms[2]["path"],
            json!("projects/**"),
            "custom permission kept"
        );
        assert_eq!(
            stored.properties.get("role_id"),
            Some(&PropertyValue::String("authenticated_user".into()))
        );

        assert!(!secure_authenticated_user_role(&storage, t, r)
            .await
            .expect("second run"));
        // A repository without the role is skipped.
        assert!(!secure_authenticated_user_role(&storage, t, "other")
            .await
            .unwrap());
    }
}
