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
use super::written_fields::{written_fields, Written};

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
    // An UPDATE grant's condition is judged on the node AS STORED: judged on
    // the new node, a caller could satisfy it by writing the very values it
    // checks (an owner field, a status). A create has only the new node.
    let grants = granting(
        target,
        target,
        &permissions.permissions,
        auth,
        scope,
        operation,
        resolver,
    )
    .await;
    // No grant at all is the operation check's refusal, not ours; an
    // unrestricted grant allows every field.
    if grants.is_empty() {
        return None;
    }
    let written = written_fields(existing, node);
    // A move or a type change must land where the caller may write too:
    // the NEW path and type have to match an update grant of theirs (its
    // condition judged on the stored node), whatever its field list.
    if let Some(stored) = existing {
        for field in ["path", "node_type"] {
            if written.contains(&Written::NodeField(field))
                && granting(
                    node,
                    stored,
                    &permissions.permissions,
                    auth,
                    scope,
                    operation,
                    resolver,
                )
                .await
                .is_empty()
            {
                return Some(field.to_string());
            }
        }
    }
    if grants.iter().any(|g| restricts_nothing(g)) {
        return None;
    }
    // Grants here all restrict fields (an unrestricted one returned above).
    written.into_iter().find_map(|w| match w {
        Written::Property(p) => (!grants.iter().any(|g| allows_field(g, &p))).then_some(p),
        // A node field (archetype, owner, relations, ...) is not a
        // property a field list can name: only an unrestricted grant
        // may write it.
        Written::NodeField(f) => Some(f.to_string()),
    })
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

    /// An update condition is judged on the STORED node: writing the value a
    /// condition checks does not satisfy it.
    #[tokio::test]
    async fn an_update_condition_cannot_be_satisfied_by_the_write_itself() {
        let mut owner_grant = grant(vec![Operation::Update]);
        owner_grant.condition = Some("node.owner == 'u1'".into());
        let mut notes_only = grant(vec![Operation::Update]);
        notes_only.fields = Some(vec!["notes".into()]);
        let a = auth(vec![owner_grant, notes_only], None);

        let stored = node("/wo/1", &[("owner", "other"), ("price", "100")]);
        let escalate = node("/wo/1", &[("owner", "u1"), ("price", "1")]);
        assert!(
            write_fields_denied(Some(&stored), &escalate, &a, &scope(), None)
                .await
                .is_some(),
            "claiming ownership in the same write must not unlock the grant"
        );

        // The real owner passes through the unrestricted grant.
        let mine = node("/wo/1", &[("owner", "u1"), ("price", "100")]);
        let edit = node("/wo/1", &[("owner", "u1"), ("price", "1")]);
        assert_eq!(
            write_fields_denied(Some(&mine), &edit, &a, &scope(), None).await,
            None
        );
    }

    /// Translations are writes of the property they translate; archetype,
    /// owner and relations are node fields only an unrestricted grant writes.
    #[tokio::test]
    async fn translations_and_node_fields_do_not_bypass_field_grants() {
        let mut g = grant(vec![Operation::Update]);
        g.fields = Some(vec!["status".into()]);
        let a = auth(vec![g], None);
        let stored = node("/wo/1", &[("status", "open"), ("title", "a")]);
        let translated = |locale_props: serde_json::Value| {
            let mut n = stored.clone();
            let mut tr = std::collections::HashMap::new();
            tr.insert("de".to_string(), PropertyValue::from_json(&locale_props));
            n.translations = Some(tr);
            n
        };

        let title_de = translated(serde_json::json!({ "title": "b" }));
        assert_eq!(
            write_fields_denied(Some(&stored), &title_de, &a, &scope(), None).await,
            Some("title".to_string())
        );
        let status_de = translated(serde_json::json!({ "status": "offen" }));
        assert_eq!(
            write_fields_denied(Some(&stored), &status_de, &a, &scope(), None).await,
            None
        );

        let mut archetype = stored.clone();
        archetype.archetype = Some("x:Other".into());
        let mut owner = stored.clone();
        owner.owner_id = Some("u1".into());
        let mut related = stored.clone();
        related.relations = vec![serde_json::from_value(serde_json::json!({
            "target": "n2", "workspace": "app", "target_node_type": "app:Team", "relation_type": "member"
        }))
        .expect("relation")];
        for (write, field) in [
            (&archetype, "archetype"),
            (&owner, "owner_id"),
            (&related, "relations"),
        ] {
            assert_eq!(
                write_fields_denied(Some(&stored), write, &a, &scope(), None).await,
                Some(field.to_string())
            );
        }

        // An unrestricted grant may write all of them.
        let a = auth(vec![grant(vec![Operation::Update])], None);
        for write in [&title_de, &archetype, &owner, &related] {
            assert_eq!(
                write_fields_denied(Some(&stored), write, &a, &scope(), None).await,
                None
            );
        }
    }

    /// A type change or move must land inside the caller's update grants,
    /// even with an unrestricted grant; a field-restricted grant may do
    /// neither.
    #[tokio::test]
    async fn a_type_change_or_move_must_stay_inside_the_grants() {
        let mut g = Permission::new("/wo/**", vec![Operation::Update]);
        g.node_types = Some(vec!["app:Order".into()]);
        let a = auth(vec![g], None);
        let stored = node("/wo/1", &[("status", "open")]);

        let mut retyped = stored.clone();
        retyped.node_type = "raisin:User".into();
        assert_eq!(
            write_fields_denied(Some(&stored), &retyped, &a, &scope(), None).await,
            Some("node_type".to_string())
        );
        let mut moved = stored.clone();
        moved.path = "/admin/1".into();
        assert_eq!(
            write_fields_denied(Some(&stored), &moved, &a, &scope(), None).await,
            Some("path".to_string())
        );
        let mut inside = stored.clone();
        inside.path = "/wo/2".into();
        inside.name = "2".into();
        assert_eq!(
            write_fields_denied(Some(&stored), &inside, &a, &scope(), None).await,
            None
        );

        // Field-restricted: neither a type change nor a move.
        let mut r = Permission::new("/**", vec![Operation::Update]);
        r.fields = Some(vec!["status".into()]);
        let a = auth(vec![r], None);
        let mut retyped = stored.clone();
        retyped.node_type = "app:Other".into();
        assert_eq!(
            write_fields_denied(Some(&stored), &retyped, &a, &scope(), None).await,
            Some("node_type".to_string())
        );
    }

    /// The built-in `authenticated_user` role's permissions, from the shipped
    /// `raisin:access_control` workspace definition.
    fn shipped_authenticated_user() -> Vec<Permission> {
        fn find(v: &serde_json::Value) -> Option<serde_json::Value> {
            match v {
                serde_json::Value::Object(m) => {
                    if m.get("properties")
                        .and_then(|p| p.get("role_id"))
                        .and_then(|r| r.as_str())
                        == Some("authenticated_user")
                    {
                        return m["properties"].get("permissions").cloned();
                    }
                    m.values().find_map(find)
                }
                serde_json::Value::Array(a) => a.iter().find_map(find),
                _ => None,
            }
        }
        let ws = crate::workspace_init::load_global_workspaces()
            .into_iter()
            .find(|w| w.name == "raisin:access_control")
            .expect("access_control workspace");
        let perms = find(&serde_json::to_value(&ws).unwrap()).expect("authenticated_user role");
        serde_json::from_value(perms).expect("permissions parse")
    }

    /// A signed-in user may edit the descriptive fields of their own user
    /// node, never what decides their rights (roles, groups, status, ...).
    #[tokio::test]
    async fn authenticated_user_cannot_grant_itself_roles() {
        let mut a = auth(shipped_authenticated_user(), None);
        a.local_user_id = Some("me".into());
        let acl = PermissionScope::new("raisin:access_control", "main");
        let mut stored = node("/users/internal/u_me", &[("display_name", "Me")]);
        stored.id = "me".into();
        stored.node_type = "raisin:User".into();

        let mut renamed = stored.clone();
        renamed
            .properties
            .insert("display_name".into(), PropertyValue::String("New".into()));
        assert_eq!(
            write_fields_denied(Some(&stored), &renamed, &a, &acl, None).await,
            None
        );

        for (key, value) in [
            ("roles", serde_json::json!(["admin"])),
            ("groups", serde_json::json!(["staff"])),
            ("can_login", serde_json::json!(true)),
            ("email", serde_json::json!("x@example.com")),
        ] {
            let mut write = stored.clone();
            write
                .properties
                .insert(key.into(), PropertyValue::from_json(&value));
            assert_eq!(
                write_fields_denied(Some(&stored), &write, &a, &acl, None).await,
                Some(key.to_string()),
                "{key}"
            );
        }
        // Reading the own node is still allowed.
        assert!(crate::services::rls_filter::can_perform(
            &stored,
            Operation::Read,
            &a,
            &acl
        ));
        assert!(crate::services::rls_filter::can_perform(
            &stored,
            Operation::Update,
            &a,
            &acl
        ));
    }
}
