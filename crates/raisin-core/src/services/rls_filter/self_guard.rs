//! Defence in depth: nobody but an administrator or the engine writes what
//! decides a user's own rights.
//!
//! PermissionService takes a user's roles and groups from their own
//! `raisin:User` node, the login maps to it by `user_id` / `email`, and
//! `can_login`, `status` and `birth_date` (minor status) gate what the account
//! may do. Whatever any role grants, an authenticated caller who is neither
//! the system nor a system administrator may not write those properties of
//! their OWN user node, nor move it (its path is their home). A role that
//! grants more is a misconfiguration; this keeps it from becoming a privilege
//! escalation (the role is also rewritten on start,
//! `system_updates::role_security`).

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;

use super::written_fields::{written_fields, Written};

/// Properties of a user's own node only an administrator or the engine writes.
pub const SELF_PROTECTED: [&str; 9] = [
    "roles",
    "groups",
    "can_login",
    "status",
    "email",
    "email_verified",
    "user_id",
    "identity_id",
    "birth_date",
];

const USER_TYPE: &str = "raisin:User";
const ACL_WORKSPACE: &str = "raisin:access_control";

fn is_own(node: &Node, auth: &AuthContext) -> bool {
    auth.local_user_id.as_deref() == Some(node.id.as_str())
        || auth.home.as_deref() == Some(node.path.as_str())
        || matches!(
            (node.properties.get("user_id"), auth.user_id.as_deref()),
            (Some(PropertyValue::String(u)), Some(me)) if u == me
        )
}

/// The protected property (or `path`) a write of `node` over `existing`
/// (`None`: a create) would change on the caller's own user node; `None` when
/// the write is allowed.
pub fn self_protected_write(
    existing: Option<&Node>,
    node: &Node,
    auth: &AuthContext,
    scope: &PermissionScope,
) -> Option<String> {
    if auth.is_system || auth.permissions().is_some_and(|p| p.is_system_admin) {
        return None;
    }
    if scope.workspace != ACL_WORKSPACE {
        return None;
    }
    let target = existing.unwrap_or(node);
    let user_type = |n: &Node| n.node_type == USER_TYPE;
    if !(user_type(target) || user_type(node)) {
        return None;
    }
    if !(is_own(target, auth) || is_own(node, auth)) {
        return None;
    }
    written_fields(existing, node)
        .into_iter()
        .find_map(|w| match w {
            Written::Property(p) if SELF_PROTECTED.contains(&p.as_str()) => Some(p),
            Written::NodeField("path") => Some("path".to_string()),
            Written::NodeField("node_type") => Some("node_type".to_string()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};

    fn me(admin: bool) -> AuthContext {
        let mut a = AuthContext::for_user("id-me").with_permissions(ResolvedPermissions {
            user_id: "id-me".into(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            // Even a role that grants everything on every user node.
            permissions: vec![Permission::new(
                "/**",
                vec![Operation::Read, Operation::Update],
            )],
            is_system_admin: admin,
            home: Some("/users/internal/u_me".into()),
            resolved_at: None,
        });
        a.local_user_id = Some("node-me".into());
        a.home = Some("/users/internal/u_me".into());
        a
    }

    fn user(id: &str, path: &str) -> Node {
        let mut n = Node {
            id: id.into(),
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            node_type: USER_TYPE.into(),
            ..Default::default()
        };
        n.properties
            .insert("display_name".into(), PropertyValue::String("Me".into()));
        n
    }

    fn acl() -> PermissionScope {
        PermissionScope::new(ACL_WORKSPACE, "main")
    }

    #[test]
    fn a_user_cannot_write_their_own_access_properties() {
        let stored = user("node-me", "/users/internal/u_me");
        for key in SELF_PROTECTED {
            let mut w = stored.clone();
            w.properties.insert(
                key.into(),
                PropertyValue::from_json(&serde_json::json!(["x"])),
            );
            assert_eq!(
                self_protected_write(Some(&stored), &w, &me(false), &acl()),
                Some(key.to_string()),
                "{key}"
            );
        }
        // Their home cannot move either.
        let mut moved = stored.clone();
        moved.path = "/users/internal/u_other".into();
        assert_eq!(
            self_protected_write(Some(&stored), &moved, &me(false), &acl()),
            Some("path".into())
        );
        // Descriptive fields stay theirs.
        let mut renamed = stored.clone();
        renamed
            .properties
            .insert("display_name".into(), PropertyValue::String("New".into()));
        assert_eq!(
            self_protected_write(Some(&stored), &renamed, &me(false), &acl()),
            None
        );
    }

    #[test]
    fn admins_the_system_and_other_users_nodes_are_not_this_guards_business() {
        let stored = user("node-me", "/users/internal/u_me");
        let mut w = stored.clone();
        w.properties.insert(
            "roles".into(),
            PropertyValue::from_json(&serde_json::json!(["admin"])),
        );
        assert_eq!(
            self_protected_write(Some(&stored), &w, &me(true), &acl()),
            None
        );
        assert_eq!(
            self_protected_write(Some(&stored), &w, &AuthContext::system(), &acl()),
            None
        );
        let other = user("node-other", "/users/internal/u_other");
        let mut w = other.clone();
        w.properties.insert(
            "roles".into(),
            PropertyValue::from_json(&serde_json::json!(["x"])),
        );
        assert_eq!(
            self_protected_write(Some(&other), &w, &me(false), &acl()),
            None
        );
    }
}
