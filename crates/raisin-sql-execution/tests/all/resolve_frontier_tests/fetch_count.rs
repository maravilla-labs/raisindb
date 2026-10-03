//! A target shared by every row of a statement is read once.

use super::*;
use raisin_core::services::reference_resolver::{ReferenceResolver, ResolveMemo};

/// Fifty rows sharing three targets read each target ONCE. Counted at the
/// resolver's storage seam; the memo is the one the SQL engine keeps per
/// statement and hands every row.
#[tokio::test]
async fn fifty_rows_sharing_three_targets_read_three_times() {
    let (storage, engine, _dir) = setup().await;
    for t in ["t0", "t1", "t2"] {
        insert(&engine, t, &format!("/{t}"), json!({ "title": t })).await;
    }
    let row = json!({
        "header": reference("t0"),
        "footer": [reference("t1"), reference("t2"), reference("t0")],
    });

    let head = storage
        .branches()
        .get_branch(TENANT, REPO, BRANCH)
        .await
        .expect("branch")
        .expect("exists")
        .head;
    let memo = Arc::new(ResolveMemo::default());
    for _ in 0..50 {
        let out = ReferenceResolver::new(storage.clone(), TENANT, REPO, BRANCH, head)
            .with_auth(Some(AuthContext::system()))
            .with_memo(memo.clone())
            .resolve_json(WS, &row, 2, None)
            .await
            .expect("resolve");
        assert_eq!(out["header"]["title"], "t0");
        assert_eq!(out["footer"][2]["title"], "t0");
    }
    let stats = memo.stats();
    assert_eq!(stats.reads, 3, "{stats:?}");
    assert_eq!(stats.occurrences, 50 * 4, "{stats:?}");

    // And through SQL, every one of fifty rows comes back fully resolved.
    insert(&engine, "list", "/list", json!({})).await;
    for i in 0..50 {
        insert(
            &engine,
            &format!("r{i}"),
            &format!("/list/r{i}"),
            row.clone(),
        )
        .await;
    }
    let found = rows(
        &engine,
        &format!("SELECT RESOLVE(properties) AS r FROM '{WS}' WHERE CHILD_OF('/list')"),
    )
    .await;
    assert_eq!(found.len(), 50);
    for r in &found {
        assert_eq!(r["r"]["footer"][1]["title"], "t2");
    }
}
