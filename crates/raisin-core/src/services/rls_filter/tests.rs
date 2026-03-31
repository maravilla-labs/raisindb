use raisin_models::permissions::ResolvedPermissions;

use super::*;
use raisin_models::permissions::{Operation, Permission, PermissionScope};
use std::collections::HashMap;

fn make_auth(user_id: &str, permissions: Vec<Permission>) -> AuthContext {
    AuthContext::for_user(user_id).with_permissions(ResolvedPermissions {
        user_id: user_id.to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions,
        is_system_admin: false,
        resolved_at: None,
    })
}

fn make_node(path: &str, node_type: &str) -> Node {
    Node {
        id: "test-id".to_string(),
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

fn make_scope() -> PermissionScope {
    PermissionScope::new("content", "main")
}

#[test]
fn test_system_context_bypasses() {
    let auth = AuthContext::system();
    let node = make_node("/content/secret", "Secret");
    let scope = make_scope();

    let result = filter_node(node.clone(), &auth, &scope);
    assert!(result.is_some());
}

#[test]
fn test_no_permissions_denies() {
    let auth = AuthContext::for_user("user1");
    let node = make_node("/content/article", "Article");
    let scope = make_scope();

    let result = filter_node(node, &auth, &scope);
    assert!(result.is_none());
}

#[test]
fn test_matching_permission_allows() {
    let auth = make_auth(
        "user1",
        vec![make_permission("content/**", vec![Operation::Read])],
    );
    let node = make_node("/content/article", "Article");
    let scope = make_scope();

    let result = filter_node(node, &auth, &scope);
    assert!(result.is_some());
}

#[test]
fn test_no_matching_permission_denies() {
    let auth = make_auth(
        "user1",
        vec![make_permission("users/**", vec![Operation::Read])],
    );
    let node = make_node("/content/article", "Article");
    let scope = make_scope();

    let result = filter_node(node, &auth, &scope);
    assert!(result.is_none());
}

#[test]
fn test_wrong_operation_denies() {
    let auth = make_auth(
        "user1",
        vec![make_permission("content/**", vec![Operation::Update])],
    );
    let node = make_node("/content/article", "Article");
    let scope = make_scope();

    let result = filter_node(node, &auth, &scope);
    assert!(result.is_none());
}

// === Scope-based tests ===

#[test]
fn test_workspace_scope_restriction() {
    let auth = make_auth(
        "user1",
        vec![Permission::new("**", vec![Operation::Read]).with_workspace("marketing")],
    );
    let node = make_node("/articles/news", "Article");

    let scope_content = PermissionScope::new("content", "main");
    assert!(filter_node(node.clone(), &auth, &scope_content).is_none());

    let scope_marketing = PermissionScope::new("marketing", "main");
    assert!(filter_node(node.clone(), &auth, &scope_marketing).is_some());
}

#[test]
fn test_branch_pattern_restriction() {
    let auth = make_auth(
        "user1",
        vec![Permission::new("**", vec![Operation::Read]).with_branch_pattern("features/*")],
    );
    let node = make_node("/articles/news", "Article");

    let scope_main = PermissionScope::new("content", "main");
    assert!(filter_node(node.clone(), &auth, &scope_main).is_none());

    let scope_feature = PermissionScope::new("content", "features/auth");
    assert!(filter_node(node.clone(), &auth, &scope_feature).is_some());
}

#[test]
fn test_combined_scope_restriction() {
    let auth = make_auth(
        "user1",
        vec![Permission::new("**", vec![Operation::Read])
            .with_workspace("content")
            .with_branch_pattern("main")],
    );
    let node = make_node("/articles/news", "Article");

    let scope_match = PermissionScope::new("content", "main");
    assert!(filter_node(node.clone(), &auth, &scope_match).is_some());

    let scope_wrong_ws = PermissionScope::new("media", "main");
    assert!(filter_node(node.clone(), &auth, &scope_wrong_ws).is_none());

    let scope_wrong_branch = PermissionScope::new("content", "develop");
    assert!(filter_node(node.clone(), &auth, &scope_wrong_branch).is_none());
}

#[test]
fn test_no_scope_restriction_matches_all() {
    let auth = make_auth("user1", vec![Permission::new("**", vec![Operation::Read])]);
    let node = make_node("/articles/news", "Article");

    let scope1 = PermissionScope::new("content", "main");
    let scope2 = PermissionScope::new("marketing", "features/test");
    let scope3 = PermissionScope::new("any", "any");

    assert!(filter_node(node.clone(), &auth, &scope1).is_some());
    assert!(filter_node(node.clone(), &auth, &scope2).is_some());
    assert!(filter_node(node.clone(), &auth, &scope3).is_some());
}

// === Stewardship context tests ===

#[test]
fn test_stewardship_context_in_rel_condition() {
    let mut auth = make_auth(
        "steward1",
        vec![Permission::new("content/**", vec![Operation::Read])
            .with_condition("node.owner_id == auth.acting_as_ward")],
    );

    auth.acting_as_ward = Some("ward123".to_string());
    auth.active_stewardship_source = Some("guardian_of".to_string());

    let scope = make_scope();

    let mut node_owned_by_ward = make_node("/content/article", "Article");
    node_owned_by_ward.owner_id = Some("ward123".to_string());
    assert!(filter_node(node_owned_by_ward, &auth, &scope).is_some());

    let mut node_owned_by_other = make_node("/content/article", "Article");
    node_owned_by_other.owner_id = Some("other_user".to_string());
    assert!(filter_node(node_owned_by_other, &auth, &scope).is_none());
}

#[test]
fn test_stewardship_or_owner_condition() {
    let mut auth = make_auth(
        "user1",
        vec![
            Permission::new("content/**", vec![Operation::Read]).with_condition(
                "node.owner_id == auth.local_user_id || node.owner_id == auth.acting_as_ward",
            ),
        ],
    );

    auth.local_user_id = Some("user1".to_string());
    auth.acting_as_ward = Some("ward456".to_string());

    let scope = make_scope();

    let mut own_node = make_node("/content/article1", "Article");
    own_node.owner_id = Some("user1".to_string());
    assert!(filter_node(own_node, &auth, &scope).is_some());

    let mut ward_node = make_node("/content/article2", "Article");
    ward_node.owner_id = Some("ward456".to_string());
    assert!(filter_node(ward_node, &auth, &scope).is_some());

    let mut other_node = make_node("/content/article3", "Article");
    other_node.owner_id = Some("other_user".to_string());
    assert!(filter_node(other_node, &auth, &scope).is_none());
}

#[test]
fn test_no_stewardship_context_is_null() {
    let auth = make_auth(
        "user1",
        vec![Permission::new("content/**", vec![Operation::Read])
            .with_condition("auth.acting_as_ward == null")],
    );

    let scope = make_scope();
    let node = make_node("/content/article", "Article");

    assert!(filter_node(node, &auth, &scope).is_some());
}

#[test]
fn test_active_stewardship_source_in_condition() {
    let mut auth = make_auth(
        "steward1",
        vec![Permission::new("content/**", vec![Operation::Read])
            .with_condition("auth.active_stewardship_source == 'guardian_of'")],
    );

    auth.acting_as_ward = Some("ward789".to_string());
    auth.active_stewardship_source = Some("guardian_of".to_string());

    let scope = make_scope();
    let node = make_node("/content/article", "Article");

    assert!(filter_node(node, &auth, &scope).is_some());
}
