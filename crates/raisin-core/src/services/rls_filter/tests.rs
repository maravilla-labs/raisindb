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

// === Graph (RELATES) RLS tests — async path ===

/// Mock resolver that records the path query it received and returns a fixed
/// answer. Lets us assert RELATES conditions are routed to the resolver and that
/// allow/deny follows its verdict.
struct MockResolver {
    answer: bool,
}

#[async_trait::async_trait]
impl raisin_rel::eval::RelationResolver for MockResolver {
    async fn has_path(
        &self,
        _source_id: &str,
        _target_id: &str,
        _relation_types: &[String],
        _min_depth: u32,
        _max_depth: u32,
        _direction: raisin_rel::RelDirection,
    ) -> Result<bool, raisin_rel::EvalError> {
        Ok(self.answer)
    }
}

fn relates_auth() -> AuthContext {
    make_auth(
        "guardian1",
        vec![Permission::new("content/**", vec![Operation::Read])
            .with_condition("node.created_by RELATES auth.local_user_id VIA 'PARENT_OF'")],
    )
    .with_local_user_id("guardian1")
}

fn child_node() -> Node {
    let mut n = make_node("/content/child", "Article");
    n.created_by = Some("child-user".to_string());
    n
}

#[test]
fn test_uses_graph_rls_detection() {
    // A RELATES condition is detected (hot-path guard picks the async lane).
    assert!(relates_auth().uses_graph_rls());

    // A plain condition is not.
    let plain = make_auth(
        "user1",
        vec![Permission::new("content/**", vec![Operation::Read])
            .with_condition("node.owner_id == auth.local_user_id")],
    );
    assert!(!plain.uses_graph_rls());

    // No conditions at all → not graph RLS.
    let none = make_auth(
        "user1",
        vec![make_permission("content/**", vec![Operation::Read])],
    );
    assert!(!none.uses_graph_rls());
}

#[tokio::test]
async fn test_relates_allows_when_path_exists() {
    let auth = relates_auth();
    let resolver = MockResolver { answer: true };
    let result = filter_node_async(child_node(), &auth, &make_scope(), Some(&resolver)).await;
    assert!(result.is_some(), "guardian with PARENT_OF path should read");
}

#[tokio::test]
async fn test_relates_denies_when_no_path() {
    let auth = relates_auth();
    let resolver = MockResolver { answer: false };
    let result = filter_node_async(child_node(), &auth, &make_scope(), Some(&resolver)).await;
    assert!(result.is_none(), "no PARENT_OF path should deny");
}

#[tokio::test]
async fn test_relates_fails_closed_without_resolver() {
    // Without a resolver the async path falls back to sync evaluation, which
    // cannot evaluate RELATES and therefore denies (fail-closed).
    let auth = relates_auth();
    let result = filter_node_async(child_node(), &auth, &make_scope(), None).await;
    assert!(
        result.is_none(),
        "RELATES must fail closed with no resolver"
    );
}

#[tokio::test]
async fn test_can_perform_async_relates_allow_deny() {
    let auth = relates_auth();
    let node = child_node();
    let scope = make_scope();

    let allow = MockResolver { answer: true };
    assert!(can_perform_async(&node, Operation::Read, &auth, &scope, Some(&allow)).await);

    let deny = MockResolver { answer: false };
    assert!(!can_perform_async(&node, Operation::Read, &auth, &scope, Some(&deny)).await);
}

#[tokio::test]
async fn test_async_non_relates_condition_still_works() {
    // A plain (non-graph) condition must still evaluate correctly on the async
    // path even when a resolver is present.
    let mut auth = make_auth(
        "user1",
        vec![Permission::new("content/**", vec![Operation::Read])
            .with_condition("node.owner_id == auth.local_user_id")],
    );
    auth.local_user_id = Some("user1".to_string());
    let resolver = MockResolver { answer: false };

    let mut own = make_node("/content/a", "Article");
    own.owner_id = Some("user1".to_string());
    assert!(
        filter_node_async(own, &auth, &make_scope(), Some(&resolver))
            .await
            .is_some()
    );

    let mut other = make_node("/content/b", "Article");
    other.owner_id = Some("someone-else".to_string());
    assert!(
        filter_node_async(other, &auth, &make_scope(), Some(&resolver))
            .await
            .is_none()
    );
}

// ── Grants are ADDITIVE ─────────────────────────────────────────────────────
//
// The bug these pin: reads resolved the single most SPECIFIC matching
// permission and denied when that one's condition failed, while creates
// already unioned across every match. So a narrow conditional grant HID rows a
// broad unconditional grant allowed — measured live, an administrator with
// `/users/** read` saw exactly one user (themselves), because
// `authenticated_user` grants `raisin:User` with `node.id == auth.local_user_id`
// and that rule was more specific. Every listing that offers people was empty
// but one, while a direct read of the same node succeeded.

/// A conditional grant that does NOT match must not veto a broad one that does.
#[test]
fn a_failing_condition_does_not_hide_what_another_grant_allows() {
    let mut narrow = make_permission("/users/**", vec![Operation::Read]);
    narrow.node_types = Some(vec!["raisin:User".to_string()]);
    narrow.condition = Some("node.id == 'somebody-else'".to_string());
    let broad = make_permission("/**", vec![Operation::Read]);

    let auth = make_auth("admin", vec![narrow, broad]);
    let node = make_node("/users/internal/alice", "raisin:User");

    assert!(
        filter_node(node, &auth, &make_scope()).is_some(),
        "the unconditional grant still allows the read"
    );
}

/// …and with no other grant, the condition still decides.
#[test]
fn a_failing_condition_alone_still_denies() {
    let mut narrow = make_permission("/users/**", vec![Operation::Read]);
    narrow.condition = Some("node.id == 'somebody-else'".to_string());

    let auth = make_auth("admin", vec![narrow]);
    let node = make_node("/users/internal/alice", "raisin:User");

    assert!(filter_node(node, &auth, &make_scope()).is_none());
}

/// The same union applies to a write check, not only to reads.
#[test]
fn can_perform_unions_across_grants_too() {
    let mut narrow = make_permission("/users/**", vec![Operation::Update]);
    narrow.condition = Some("node.id == 'somebody-else'".to_string());
    let broad = make_permission("/**", vec![Operation::Update]);

    let auth = make_auth("admin", vec![narrow, broad]);
    let node = make_node("/users/internal/alice", "raisin:User");

    assert!(can_perform(&node, Operation::Update, &auth, &make_scope()));
}

/// A SATISFIED condition on the most specific grant still wins, so field
/// filtering keeps applying the narrowest rule that actually granted access.
#[test]
fn the_most_specific_satisfied_grant_is_the_one_that_applies() {
    let mut narrow = make_permission("/users/**", vec![Operation::Read]);
    narrow.condition = Some("node.node_type == 'raisin:User'".to_string());
    narrow.except_fields = Some(vec!["email".to_string()]);
    let broad = make_permission("/**", vec![Operation::Read]);

    let auth = make_auth("admin", vec![narrow, broad]);
    let mut node = make_node("/users/internal/alice", "raisin:User");
    node.properties.insert(
        "email".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("a@b.c".to_string()),
    );

    let filtered = filter_node(node, &auth, &make_scope()).expect("allowed");
    assert!(
        !filtered.properties.contains_key("email"),
        "the narrow grant's field filter must be the one applied"
    );
}
