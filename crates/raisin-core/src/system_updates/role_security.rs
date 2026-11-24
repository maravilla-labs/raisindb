// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Security system update: no role lets a user write their own user node
//! beyond its descriptive fields.
//!
//! PermissionService reads a user's roles and groups from their own
//! `raisin:User` node, so a role that grants a user unrestricted `update` on
//! that node lets the user grant themselves anything. The shipped
//! `authenticated_user` role used to do exactly that, and any role an
//! operator wrote may too. On every start, for every repository and EVERY
//! branch (roles are found by `role_id` on whatever branch a request runs
//! on), every `raisin:Role` in `raisin:access_control` is checked:
//!
//! - a permission is a SELF GRANT when, parsed the way the resolver parses it
//!   (`permission_service::parsing::parse_permission`, so `conditions`,
//!   whitespace or key order make no difference), it grants `update`, applies
//!   to `raisin:User` (node_types empty or naming it), to the user's home
//!   path and to this workspace, has no `fields` / `except_fields`, and its
//!   condition holds for the caller's OWN user node but not for someone
//!   else's (a condition-less grant manages every user and is an
//!   administrator's grant, not a self grant);
//! - a self grant is split into the same permission without `update` (when
//!   it granted more) and an `update` limited to [`SELF_EDITABLE`]; every
//!   other permission of the role is kept as it is;
//! - a role with no self grant is not written, so the update is idempotent.
//!
//! Defence in depth beside it: the write layer refuses a non-admin's write of
//! the access-deciding properties of their own user node whatever any role
//! says (`rls_filter::self_guard`).
//!
//! Runs whatever the definitions' `auto_apply` policy says: it closes a
//! privilege escalation, it does not change a schema.

use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};
use raisin_storage::node_patch::{PatchTarget, PropertyPatch};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};
use serde_json::{json, Value};

use crate::services::permission_service::parsing::parse_permission;
use crate::services::rls_filter::context::evaluate_rel_condition;

/// Where roles live.
pub const ROLE_WORKSPACE: &str = "raisin:access_control";
/// What a user may edit on their own user node.
pub const SELF_EDITABLE: [&str; 3] = ["display_name", "title", "metadata"];

/// A probe caller and their own / a foreign user node, to decide whether a
/// condition is a self condition.
fn probes() -> (AuthContext, Node, Node) {
    let home = "/users/internal/u_selfprobe";
    let mut auth = AuthContext::for_user("selfprobe-identity");
    auth.local_user_id = Some("selfprobe-node".into());
    auth.home = Some(home.into());
    let mut own = Node {
        id: "selfprobe-node".into(),
        name: "u_selfprobe".into(),
        path: home.into(),
        node_type: "raisin:User".into(),
        created_by: Some("selfprobe-identity".into()),
        owner_id: Some("selfprobe-identity".into()),
        ..Default::default()
    };
    own.properties.insert(
        "user_id".into(),
        PropertyValue::String("selfprobe-identity".into()),
    );
    let mut other = own.clone();
    other.id = "otherprobe-node".into();
    other.name = "u_otherprobe".into();
    other.path = "/users/internal/u_otherprobe".into();
    other.created_by = Some("otherprobe-identity".into());
    other.owner_id = Some("otherprobe-identity".into());
    other.properties.insert(
        "user_id".into(),
        PropertyValue::String("otherprobe-identity".into()),
    );
    (auth, own, other)
}

/// Whether a stored permission entry is an unrestricted self update.
pub fn is_self_grant(entry: &PropertyValue) -> bool {
    let Some(p) = parse_permission(entry) else {
        return false;
    };
    if !p.operations.contains(&Operation::Update) || p.fields.is_some() || p.except_fields.is_some()
    {
        return false;
    }
    if let Some(types) = &p.node_types {
        if !types.is_empty() && !types.iter().any(|t| t == "raisin:User") {
            return false;
        }
    }
    let (auth, own, other) = probes();
    let scope = PermissionScope::new(ROLE_WORKSPACE, "main");
    if !p.applies_to_scope(&scope) || !p.matches_path(&own.path) {
        return false;
    }
    let Some(condition) = &p.condition else {
        return false; // manages every user: an administrator's grant
    };
    evaluate_rel_condition(condition, &own, &auth)
        && !evaluate_rel_condition(condition, &other, &auth)
}

/// The fixed permission list, or `None` when nothing needs changing.
pub fn secure_permissions(entries: &[PropertyValue]) -> Option<Vec<Value>> {
    if !entries.iter().any(is_self_grant) {
        return None;
    }
    let mut out = Vec::with_capacity(entries.len() + 1);
    for entry in entries {
        let raw = serde_json::to_value(entry).unwrap_or(Value::Null);
        if !is_self_grant(entry) {
            out.push(raw);
            continue;
        }
        let others: Vec<Value> = raw["operations"]
            .as_array()
            .map(|ops| {
                ops.iter()
                    .filter(|o| o.as_str().and_then(Operation::parse) != Some(Operation::Update))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !others.is_empty() {
            let mut rest = raw.clone();
            rest["operations"] = Value::Array(others);
            out.push(rest);
        }
        let mut update = raw;
        update["operations"] = json!(["update"]);
        update["fields"] = json!(SELF_EDITABLE);
        if let Some(obj) = update.as_object_mut() {
            obj.remove("except_fields");
        }
        out.push(update);
    }
    Some(out)
}

/// Rewrite every role of one repository branch that holds a self grant.
/// Returns how many roles were written.
pub async fn secure_roles<S>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<usize>
where
    S: Storage + TransactionalStorage,
{
    let scope = StorageScope::new(tenant_id, repo_id, branch, ROLE_WORKSPACE);
    let roles = storage
        .nodes()
        .list_by_type(scope, "raisin:Role", ListOptions::default())
        .await?;
    let mut written = 0;
    for role in roles {
        let Some(PropertyValue::Array(entries)) = role.properties.get("permissions") else {
            continue;
        };
        let Some(fixed) = secure_permissions(entries) else {
            continue;
        };
        let txn = storage.begin_context().await?;
        txn.set_tenant_repo(tenant_id, repo_id)?;
        txn.set_branch(branch)?;
        txn.set_actor("system-update")?;
        txn.set_message("security: a role may not let users write their own roles or groups")?;
        txn.set_auth_context(AuthContext::system())?;
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
            branch = %branch,
            role = %role.path,
            "security update: the role's self update of raisin:User limited to {:?}",
            SELF_EDITABLE
        );
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage::{BranchRepository, CreateNodeOptions, RepoScope, WorkspaceRepository};
    use raisin_storage_memory::InMemoryStorage;

    fn pv(v: Value) -> PropertyValue {
        PropertyValue::from_json(&v)
    }

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
    fn the_self_grant_is_split_and_the_rest_is_kept() {
        let custom = json!({ "path": "projects/**", "operations": ["read"] });
        let fixed =
            secure_permissions(&[pv(old_self_grant()), pv(custom.clone())]).expect("changed");
        assert_eq!(fixed.len(), 3);
        assert_eq!(fixed[0]["operations"], json!(["read"]));
        assert_eq!(fixed[1]["operations"], json!(["update"]));
        assert_eq!(fixed[1]["fields"], json!(SELF_EDITABLE));
        assert_eq!(fixed[2], custom);
        let again: Vec<PropertyValue> = fixed.iter().map(|v| pv(v.clone())).collect();
        assert!(
            secure_permissions(&again).is_none(),
            "second pass changes nothing"
        );
    }

    /// Spelled differently, same meaning: still a self grant.
    #[test]
    fn equivalent_spellings_are_recognised() {
        for v in [
            // whitespace, other self conditions, no node_types (= every type)
            json!({ "path": "users/**", "operations": ["UPDATE"], "condition": "  node.id==auth.local_user_id " }),
            json!({ "path": "**", "operations": ["update"], "node_types": [], "condition": "node.path == auth.home" }),
            json!({ "path": "users/**", "operations": ["update"], "condition": "node.user_id == auth.user_id" }),
            json!({ "path": "users/**", "operations": ["update"], "conditions": { "owner": "$user.id" } }),
        ] {
            assert!(is_self_grant(&pv(v.clone())), "{v}");
        }
    }

    #[test]
    fn restricted_foreign_and_admin_grants_are_left_alone() {
        for v in [
            json!({ "path": "users/**", "operations": ["update"], "node_types": ["raisin:User"],
                    "condition": "node.id == auth.local_user_id", "fields": ["display_name"] }),
            json!({ "path": "users/**", "operations": ["update"], "node_types": ["raisin:Profile"],
                    "condition": "node.id == auth.local_user_id" }),
            json!({ "path": "users/**", "operations": ["read"], "condition": "node.id == auth.local_user_id" }),
            // manages every user: an administrator's grant
            json!({ "path": "users/**", "operations": ["update"], "node_types": ["raisin:User"] }),
            json!({ "path": "content/**", "operations": ["update"], "condition": "node.id == auth.local_user_id" }),
        ] {
            assert!(!is_self_grant(&pv(v.clone())), "{v}");
        }
    }

    async fn seed(
        storage: &InMemoryStorage,
        t: &str,
        r: &str,
        branch: &str,
        id: &str,
        perms: Value,
    ) {
        let _ = storage
            .branches()
            .create_branch(t, r, branch, "test", None, None, false, false)
            .await;
        let _ = storage
            .workspaces()
            .put(
                RepoScope::new(t, r),
                raisin_models::workspace::Workspace::new(ROLE_WORKSPACE.into()),
            )
            .await;
        let mut role = Node {
            id: id.into(),
            name: id.into(),
            path: format!("/roles/{id}"),
            node_type: "raisin:Role".into(),
            ..Default::default()
        };
        role.properties
            .insert("role_id".into(), PropertyValue::String(id.into()));
        role.properties.insert("permissions".into(), pv(perms));
        storage
            .nodes()
            .create(
                StorageScope::new(t, r, branch, ROLE_WORKSPACE),
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
    }

    /// Every role on the branch, not only authenticated_user; the second run
    /// writes nothing; another branch is its own run.
    #[tokio::test]
    async fn every_role_of_a_branch_is_secured_once() {
        let storage = InMemoryStorage::default();
        let (t, r) = ("default", "legacy");
        let custom = json!({ "path": "projects/**", "operations": ["read"] });
        seed(
            &storage,
            t,
            r,
            "main",
            "authenticated_user",
            json!([old_self_grant(), custom]),
        )
        .await;
        seed(
            &storage,
            t,
            r,
            "main",
            "member",
            json!([{ "path": "users/**", "operations": ["update"],
            "condition": "node.path == auth.home" }]),
        )
        .await;
        seed(
            &storage,
            t,
            r,
            "main",
            "editor",
            json!([{ "path": "content/**", "operations": ["update"] }]),
        )
        .await;
        seed(
            &storage,
            t,
            r,
            "publish",
            "authenticated_user",
            json!([old_self_grant()]),
        )
        .await;

        assert_eq!(secure_roles(&storage, t, r, "main").await.unwrap(), 2);
        assert_eq!(
            secure_roles(&storage, t, r, "main").await.unwrap(),
            0,
            "idempotent"
        );
        assert_eq!(secure_roles(&storage, t, r, "publish").await.unwrap(), 1);

        let au = storage
            .nodes()
            .get_by_path(
                StorageScope::new(t, r, "main", ROLE_WORKSPACE),
                "/roles/authenticated_user",
                None,
            )
            .await
            .unwrap()
            .unwrap();
        let perms = serde_json::to_value(&au.properties["permissions"]).unwrap();
        assert_eq!(perms[1]["fields"], json!(SELF_EDITABLE));
        assert_eq!(
            perms[2]["path"],
            json!("projects/**"),
            "custom permission kept"
        );
    }
}
