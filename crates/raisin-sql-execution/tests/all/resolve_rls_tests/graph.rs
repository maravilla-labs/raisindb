//! Graph RLS (`RELATES … VIA`) on RESOLVE targets.
//!
//! A grant whose condition walks the relation graph cannot be decided from a
//! path, so RESOLVE must load the target and evaluate the condition with a
//! graph resolver built at the statement's snapshot. (The path-only pre-check
//! is switched off for such callers in `fetch.rs`; it is an upper bound, so
//! no SQL-visible result depends on that — the graph verdict below does.)

use super::*;

/// Read on `pages`; read on `secret` only for nodes related to the caller's
/// own user node by `OWNED_BY`.
fn owner_reader() -> AuthContext {
    AuthContext::for_user("owner")
        .with_permissions(ResolvedPermissions {
            user_id: "owner".into(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions: vec![
                Permission::new("/**", vec![Operation::Read]).with_workspace(PAGES),
                Permission::new("/**", vec![Operation::Read])
                    .with_workspace(SECRET)
                    .with_condition("node.id RELATES auth.local_user_id VIA 'OWNED_BY'"),
            ],
            is_system_admin: false,
            resolved_at: Some(std::time::Instant::now()),
        })
        .with_local_user_id("u1")
}

#[tokio::test]
async fn resolve_graph_rls_target_denied() {
    let (storage, _dir) = setup().await;
    let sys = engine(&storage, AuthContext::system());
    insert(&sys, SECRET, "u1", "/u1", json!({ "kind": "user" })).await;
    insert(&sys, SECRET, "mine", "/mine", json!({ "note": "owned" })).await;
    insert(
        &sys,
        SECRET,
        "theirs",
        "/theirs",
        json!({ "note": "not owned" }),
    )
    .await;
    rows(
        &sys,
        &format!(
            "RELATE FROM id='mine' IN WORKSPACE '{SECRET}' \
             TO id='u1' IN WORKSPACE '{SECRET}' TYPE 'OWNED_BY'"
        ),
    )
    .await;
    insert(
        &sys,
        PAGES,
        "home",
        "/home",
        json!({
            "mine": reference("mine", SECRET),
            "theirs": reference("theirs", SECRET),
            "missing": reference("nope", SECRET),
        }),
    )
    .await;

    let reader = engine(&storage, owner_reader());
    assert!(owner_reader().uses_graph_rls());
    let r = resolved(&reader, "RESOLVE(properties)").await;
    let stored = resolved(&reader, "properties").await;

    // The related target passes the graph condition and is inlined.
    assert_eq!(r["mine"]["note"], "owned", "{r}");
    // The unrelated one is denied, and comes back exactly like a missing one.
    assert!(r["theirs"].get("note").is_none(), "graph RLS leaked: {r}");
    assert_eq!(r["theirs"], stored["theirs"]);
    assert_eq!(r["missing"], stored["missing"]);

    // Same verdict for a literal the caller typed.
    let literal = r#"RESOLVE('{"raisin:ref":"theirs","raisin:workspace":"secret"}'::jsonb)"#;
    let r = resolved(&reader, literal).await;
    assert_eq!(
        r,
        json!({ "raisin:ref": "theirs", "raisin:workspace": "secret" })
    );
}
