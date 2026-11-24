//! Write grants: a create condition over the node being created, and
//! field-level `fields` / `except_fields` on create and update grants.
//!
//! - **Create condition.** [`can_create_node`] evaluates a create grant's
//!   `condition` against the node AS IT WILL BE STORED (path, type,
//!   properties, `created_by`), with the same REL subset read grants use —
//!   e.g. `node.path.startsWith(auth.home)` for "only under my own path".
//!   [`super::can_create_at_path`], which knows only a path and a type, keeps
//!   failing closed on conditions over anything else.
//! - **Field-level writes.** [`write_fields_denied`] names the first property
//!   a write changes that no matching grant lets it write. A property is
//!   writable when ANY matching grant (scope, path, type, operation and
//!   condition) allows it: no field list, or listed in `fields` and not in
//!   `except_fields`. An update is judged by the properties it CHANGES, so a
//!   client that echoes a protected value back unchanged is not refused. A
//!   create is judged by every property it sets.
//!
//! Engine-owned keys (`$…`, `__…`) are never subject to field grants.
//! Enforced at the write layer (`put_node` / `add_node`), which every path
//! (REST, SQL, WebSocket, functions) reaches.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, PermissionScope};
use raisin_rel::eval::RelationResolver;

use super::context::evaluate_rel_condition_async;
use super::matching::matching_permissions;
use super::visitor_zone;

/// Whether `auth` may create `node` (its path, type and create condition
/// evaluated over the node itself).
pub async fn can_create_node(
    node: &Node,
    auth: &AuthContext,
    scope: &PermissionScope,
    resolver: Option<&dyn RelationResolver>,
) -> bool {
    if auth.is_system {
        return true;
    }
    if let Some(allowed) = visitor_zone(auth, scope, &node.path, Operation::Create) {
        return allowed;
    }
    let Some(permissions) = auth.permissions() else {
        return false;
    };
    if permissions.is_system_admin {
        return true;
    }
    !granting(
        node,
        node,
        &permissions.permissions,
        auth,
        scope,
        Operation::Create,
        resolver,
    )
    .await
    .is_empty()
}

/// The first property `node` writes that no matching grant allows `auth` to
/// write; `None` when every one is allowed (or no grant restricts fields).
/// `existing`: the stored node for an update, `None` for a create.
pub async fn write_fields_denied(
    existing: Option<&Node>,
    node: &Node,
    auth: &AuthContext,
    scope: &PermissionScope,
    resolver: Option<&dyn RelationResolver>,
) -> Option<String> {
    if auth.is_system {
        return None;
    }
    let (operation, target) = match existing {
        Some(stored) => (Operation::Update, stored),
        None => (Operation::Create, node),
    };
    if visitor_zone(auth, scope, &target.path, operation).is_some() {
        return None;
    }
    let permissions = auth.permissions()?;
    if permissions.is_system_admin {
        return None;
    }
    let grants = granting(
        target,
        node,
        &permissions.permissions,
        auth,
        scope,
        operation,
        resolver,
    )
    .await;
    // No grant at all is the operation check's refusal, not ours; an
    // unrestricted grant allows every field.
    if grants.is_empty() || grants.iter().any(|g| restricts_nothing(g)) {
        return None;
    }
    written_properties(existing, node)
        .into_iter()
        .find(|p| !grants.iter().any(|g| allows_field(g, p)))
}

/// Grants matching `target` for `operation` whose condition holds over
/// `subject` (the stored node for an update, the new node for a create).
async fn granting<'a>(
    target: &Node,
    subject: &Node,
    permissions: &'a [Permission],
    auth: &AuthContext,
    scope: &PermissionScope,
    operation: Operation,
    resolver: Option<&dyn RelationResolver>,
) -> Vec<&'a Permission> {
    let mut out = Vec::new();
    for permission in matching_permissions(target, permissions, scope, operation) {
        if let Some(condition) = &permission.condition {
            if !evaluate_rel_condition_async(condition, subject, auth, resolver).await {
                continue;
            }
        }
        out.push(permission);
    }
    out
}

fn restricts_nothing(grant: &Permission) -> bool {
    grant.fields.is_none() && grant.except_fields.is_none()
}

fn allows_field(grant: &Permission, property: &str) -> bool {
    grant
        .fields
        .as_ref()
        .is_none_or(|f| f.iter().any(|x| x == property))
        && grant
            .except_fields
            .as_ref()
            .is_none_or(|e| !e.iter().any(|x| x == property))
}

fn is_engine_owned(property: &str) -> bool {
    property.starts_with('$') || property.starts_with("__")
}

/// The properties a write sets: those it changes (update) or carries
/// (create), engine-owned keys left out, sorted.
fn written_properties(existing: Option<&Node>, node: &Node) -> Vec<String> {
    let mut names: Vec<String> = match existing {
        None => node.properties.keys().cloned().collect(),
        Some(stored) => {
            let differs =
                |a: &raisin_models::nodes::properties::PropertyValue,
                 b: &raisin_models::nodes::properties::PropertyValue| {
                    a != b && serde_json::to_value(a).ok() != serde_json::to_value(b).ok()
                };
            node.properties
                .iter()
                .filter(|(k, v)| stored.properties.get(*k).is_none_or(|o| differs(o, v)))
                .map(|(k, _)| k.clone())
                .chain(
                    stored
                        .properties
                        .keys()
                        .filter(|k| !node.properties.contains_key(*k))
                        .cloned(),
                )
                .collect()
        }
    };
    names.retain(|k| !is_engine_owned(k));
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::permissions::ResolvedPermissions;

    fn auth(permissions: Vec<Permission>, home: Option<&str>) -> AuthContext {
        let mut a = AuthContext::for_user("u1").with_permissions(ResolvedPermissions {
            user_id: "u1".to_string(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions,
            is_system_admin: false,
            home: home.map(str::to_string),
            resolved_at: None,
        });
        a.home = home.map(str::to_string);
        a
    }

    fn node(path: &str, props: &[(&str, &str)]) -> Node {
        Node {
            id: "n".into(),
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            node_type: "app:Order".into(),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
                .collect(),
            ..Default::default()
        }
    }

    fn scope() -> PermissionScope {
        PermissionScope::new("app", "main")
    }

    fn grant(ops: Vec<Operation>) -> Permission {
        Permission::new("/**", ops)
    }

    /// A technician may update only `status` and `notes` of a work order.
    #[tokio::test]
    async fn an_update_grant_with_fields_allows_only_those_fields() {
        let mut g = grant(vec![Operation::Read, Operation::Update]);
        g.fields = Some(vec!["status".into(), "notes".into()]);
        let a = auth(vec![g], None);
        let stored = node("/wo/1", &[("status", "open"), ("price", "100")]);

        let ok = node(
            "/wo/1",
            &[("status", "done"), ("price", "100"), ("$mixins", "x")],
        );
        assert_eq!(
            write_fields_denied(Some(&stored), &ok, &a, &scope(), None).await,
            None
        );

        let bad = node("/wo/1", &[("status", "done"), ("price", "1")]);
        assert_eq!(
            write_fields_denied(Some(&stored), &bad, &a, &scope(), None).await,
            Some("price".to_string())
        );
        // Removing a property is writing it.
        let removed = node("/wo/1", &[("status", "open")]);
        assert_eq!(
            write_fields_denied(Some(&stored), &removed, &a, &scope(), None).await,
            Some("price".to_string())
        );
    }

    #[tokio::test]
    async fn except_fields_protects_a_field_and_another_grant_can_open_it() {
        let mut g = grant(vec![Operation::Update]);
        g.except_fields = Some(vec!["cost".into()]);
        let stored = node("/wo/1", &[("cost", "5"), ("title", "a")]);
        let write = node("/wo/1", &[("cost", "6"), ("title", "a")]);
        let a = auth(vec![g.clone()], None);
        assert_eq!(
            write_fields_denied(Some(&stored), &write, &a, &scope(), None).await,
            Some("cost".to_string())
        );
        // An unrestricted second grant (another role) allows it: grants add up.
        let a = auth(vec![g, grant(vec![Operation::Update])], None);
        assert_eq!(
            write_fields_denied(Some(&stored), &write, &a, &scope(), None).await,
            None
        );
    }

    #[tokio::test]
    async fn a_create_is_judged_by_every_property_it_sets() {
        let mut g = grant(vec![Operation::Create]);
        g.fields = Some(vec!["title".into()]);
        let a = auth(vec![g], None);
        let new = node("/wo/2", &[("title", "x"), ("price", "1")]);
        assert_eq!(
            write_fields_denied(None, &new, &a, &scope(), None).await,
            Some("price".to_string())
        );
    }

    /// A role that may create only under its own home.
    #[tokio::test]
    async fn a_create_condition_sees_the_node_being_created() {
        let mut g = Permission::new("/**", vec![Operation::Create]);
        g.condition = Some("node.path.startsWith(auth.home)".into());
        let a = auth(vec![g], Some("/customers/acme"));
        assert!(can_create_node(&node("/customers/acme/req-1", &[]), &a, &scope(), None).await);
        assert!(!can_create_node(&node("/customers/other/req-1", &[]), &a, &scope(), None).await);

        // A condition over a property of the new node.
        let mut g = Permission::new("/**", vec![Operation::Create]);
        g.condition = Some("node.status == 'draft'".into());
        let a = auth(vec![g], None);
        assert!(can_create_node(&node("/x", &[("status", "draft")]), &a, &scope(), None).await);
        assert!(!can_create_node(&node("/x", &[("status", "sent")]), &a, &scope(), None).await);
    }
}
