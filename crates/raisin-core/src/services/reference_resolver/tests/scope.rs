//! Scope and safety: workspace identity, row-level security, the budget.

use super::*;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};

/// A reader holding read on `workspace` only.
pub(super) fn reader_of(workspace: &str) -> AuthContext {
    AuthContext::for_user("reader").with_permissions(ResolvedPermissions {
        user_id: "reader".into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new("/**", vec![Operation::Read]).with_workspace(workspace)],
        is_system_admin: false,
        resolved_at: None,
    })
}

/// The same path in two workspaces is two targets. The old memo was keyed by
/// the locator alone, so the second workspace's reference got the first's node.
#[tokio::test]
async fn test_same_path_in_two_workspaces_does_not_collide() {
    let storage = Arc::new(InMemoryStorage::default());
    create_test_node(
        &storage,
        "assets",
        "a1",
        "/logo",
        str_props(&[("v", "assets")]),
    )
    .await;
    create_test_node(
        &storage,
        "media",
        "m1",
        "/logo",
        str_props(&[("v", "media")]),
    )
    .await;

    let doc = serde_json::json!({
        "a": {"raisin:ref": "/logo", "raisin:workspace": "assets"},
        "m": {"raisin:ref": "/logo", "raisin:workspace": "media"},
    });
    let out = resolver(&storage)
        .resolve_json("stories", &doc, 1, None)
        .await
        .unwrap();
    assert_eq!(out["a"]["v"], "assets");
    assert_eq!(out["m"]["v"], "media");
}

/// A target the caller may not read is left bare — byte-identical to a target
/// that does not exist — and the denied workspace is never read at all.
#[tokio::test]
async fn test_denied_target_is_indistinguishable_from_missing() {
    let storage = Arc::new(InMemoryStorage::default());
    create_test_node(&storage, "secret", "s1", "/s1", str_props(&[("pii", "x")])).await;
    create_test_node(&storage, "open", "o1", "/o1", str_props(&[("title", "O")])).await;

    let doc = serde_json::json!({
        "denied": json_ref("s1", "secret"),
        "missing": json_ref("nope", "secret"),
        "allowed": json_ref("o1", "open"),
    });
    let resolver = resolver(&storage).with_auth(Some(reader_of("open")));
    let out = resolver.resolve_json("open", &doc, 1, None).await.unwrap();

    assert_eq!(out["denied"], doc["denied"]);
    assert_eq!(out["missing"], doc["missing"]);
    assert_eq!(out["allowed"]["title"], "O");
    // Only the readable workspace was read.
    assert_eq!(resolver.memo.stats().reads, 1);
}

/// Exceeding the budget is an error naming RESOLVE, not a truncated document.
#[tokio::test]
async fn test_budget_errors_loudly() {
    let storage = Arc::new(InMemoryStorage::default());
    for i in 0..3 {
        create_test_node(
            &storage,
            "test",
            &format!("n{i}"),
            &format!("/n{i}"),
            HashMap::new(),
        )
        .await;
    }
    let doc = serde_json::json!([
        json_ref("n0", "test"),
        json_ref("n1", "test"),
        json_ref("n2", "test")
    ]);

    let tight = ResolveBudget {
        max_targets: 2,
        ..ResolveBudget::default()
    };
    let err = resolver(&storage)
        .with_memo(Arc::new(ResolveMemo::new(tight)))
        .resolve_json("test", &doc, 1, None)
        .await
        .expect_err("three targets over a budget of two");
    assert!(err.to_string().contains("RESOLVE"), "{err}");

    let tight = ResolveBudget {
        max_occurrences: 2,
        ..ResolveBudget::default()
    };
    let err = resolver(&storage)
        .with_memo(Arc::new(ResolveMemo::new(tight)))
        .resolve_json("test", &doc, 1, None)
        .await
        .expect_err("three inlined occurrences over a budget of two");
    assert!(err.to_string().contains("RESOLVE"), "{err}");
}
