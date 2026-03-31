//! Tests for permission checker.

use super::checker::{can_read_in_path, PermissionChecker};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use std::collections::HashMap;

fn make_node(path: &str, node_type: &str) -> Node {
    Node {
        id: "test".to_string(),
        name: "test".to_string(),
        path: path.to_string(),
        node_type: node_type.to_string(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

fn make_permission(path: &str, ops: Vec<Operation>) -> Permission {
    Permission::new(path, ops)
}

#[test]
fn test_basic_read_permission() {
    let permissions = vec![make_permission("content.**", vec![Operation::Read])];

    let auth = AuthContext::for_user("user1").with_permissions(ResolvedPermissions {
        user_id: "user1".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: permissions.clone(),
        is_system_admin: false,
        resolved_at: None,
    });

    let checker = PermissionChecker::with_permissions(&auth, &permissions);

    let node = make_node("/content/articles/news", "Article");
    assert!(checker.can_read(&node));

    let outside_node = make_node("/users/profile", "Profile");
    assert!(!checker.can_read(&outside_node));
}

#[test]
fn test_system_context_bypasses() {
    let auth = AuthContext::system();

    // System context returns None from new() - it bypasses all checks
    assert!(PermissionChecker::new(&auth).is_none());
}

#[test]
fn test_node_type_filter() {
    let mut permission = make_permission("content.**", vec![Operation::Read]);
    permission.node_types = Some(vec!["Article".to_string()]);

    let permissions = vec![permission];

    let auth = AuthContext::for_user("user1").with_permissions(ResolvedPermissions {
        user_id: "user1".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: permissions.clone(),
        is_system_admin: false,
        resolved_at: None,
    });

    let checker = PermissionChecker::with_permissions(&auth, &permissions);

    let article = make_node("/content/post1", "Article");
    assert!(checker.can_read(&article));

    let folder = make_node("/content/folder1", "Folder");
    assert!(!checker.can_read(&folder));
}

#[test]
fn test_operation_filter() {
    let permissions = vec![make_permission("content.**", vec![Operation::Read])];

    let auth = AuthContext::for_user("user1").with_permissions(ResolvedPermissions {
        user_id: "user1".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: permissions.clone(),
        is_system_admin: false,
        resolved_at: None,
    });

    let checker = PermissionChecker::with_permissions(&auth, &permissions);
    let node = make_node("/content/post1", "Article");

    assert!(checker.can_read(&node));
    assert!(!checker.can_update(&node));
    assert!(!checker.can_delete(&node));
}

#[test]
fn test_rel_condition_ownership() {
    let permission = Permission::new(
        "content.**",
        vec![Operation::Read, Operation::Update, Operation::Delete],
    )
    .with_condition("node.created_by == auth.user_id");

    let permissions = vec![permission];

    let auth = AuthContext::for_user("user1").with_permissions(ResolvedPermissions {
        user_id: "user1".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: permissions.clone(),
        is_system_admin: false,
        resolved_at: None,
    });

    let checker = PermissionChecker::with_permissions(&auth, &permissions);

    // Node created by user1 - should be allowed
    let mut owned_node = make_node("/content/post1", "Article");
    owned_node.created_by = Some("user1".to_string());
    assert!(checker.can_read(&owned_node));
    assert!(checker.can_update(&owned_node));
    assert!(checker.can_delete(&owned_node));

    // Node created by someone else - should be denied
    let mut other_node = make_node("/content/post2", "Article");
    other_node.created_by = Some("user2".to_string());
    assert!(!checker.can_read(&other_node));
    assert!(!checker.can_update(&other_node));
    assert!(!checker.can_delete(&other_node));
}

#[test]
fn test_rel_condition_published_status() {
    let permission = Permission::new("content.**", vec![Operation::Read])
        .with_condition("node.status == 'published' || node.created_by == auth.user_id");

    let permissions = vec![permission];

    let auth = AuthContext::for_user("user1").with_permissions(ResolvedPermissions {
        user_id: "user1".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: permissions.clone(),
        is_system_admin: false,
        resolved_at: None,
    });

    let checker = PermissionChecker::with_permissions(&auth, &permissions);

    // Published node by someone else - should be readable
    let mut published_node = make_node("/content/post1", "Article");
    published_node.created_by = Some("user2".to_string());
    published_node.properties.insert(
        "status".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("published".to_string()),
    );
    assert!(checker.can_read(&published_node));

    // Draft node by user1 (owner) - should be readable
    let mut own_draft_node = make_node("/content/post2", "Article");
    own_draft_node.created_by = Some("user1".to_string());
    own_draft_node.properties.insert(
        "status".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("draft".to_string()),
    );
    assert!(checker.can_read(&own_draft_node));

    // Draft node by someone else - should NOT be readable
    let mut other_draft_node = make_node("/content/post3", "Article");
    other_draft_node.created_by = Some("user2".to_string());
    other_draft_node.properties.insert(
        "status".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("draft".to_string()),
    );
    assert!(!checker.can_read(&other_draft_node));
}
