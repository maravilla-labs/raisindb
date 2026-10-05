//! Plan Phase 13f: the BUILT-IN `(__parent_path, __created_at)` workspace
//! index. Every workspace carries it unless it opts out, so a bare
//! `CHILD_OF($p) ORDER BY created_at [ASC|DESC] [LIMIT n]` is index-served out
//! of the box once the automatic `compound_builds` link has built it — and
//! until then (or once switched off) the listing scans exactly as before.

use super::compound_index_hierarchy::{
    build_compound, explain, folder_time, setup, setup_with, strings, Owner, BARE, TENANT, TYPED,
    WS,
};
use futures::StreamExt;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use raisin_rocksdb::RocksDBStorage;

const BUILTIN: &str = "@__children_by_created_at";

/// The `compound_builds` link on `main` — what the automatic chain's job
/// runs after start (no admin call).
pub(super) async fn build_automatically(storage: &RocksDBStorage, repo: &str) {
    let reports = run_repair(
        storage,
        TENANT,
        repo,
        Some("main"),
        RepairKind::CompoundBuilds,
        RepairOptions {
            check_headroom: false,
            max_bytes_per_sec: 0,
            ..RepairOptions::default()
        },
    )
    .await
    .expect("compound_builds link");
    assert!(reports.iter().all(|r| r.completed), "{reports:?}");
}

async fn run(engine: &super::compound_index_hierarchy::Engine, sql: &str) {
    let mut stream = engine.execute(sql).await.expect(sql);
    while let Some(row) = stream.next().await {
        row.expect("row");
    }
}

/// On by default: not used before it is built, used (sort elided, owner
/// shown) once the automatic link built it — newest-first and oldest-first,
/// typed or not, with or without another predicate. DESCENDANT_OF is not.
#[tokio::test]
async fn the_builtin_index_serves_folder_listings_once_built() {
    let repo = "r_cbl_on";
    let (engine, storage, _tmp) = setup(repo, Owner::Builtin, false).await;
    assert!(
        !explain(&engine, BARE).await.contains("CompoundIndexScan"),
        "not built yet: the listing scans as before"
    );
    build_automatically(&storage, repo).await;

    for sql in [
        BARE,
        TYPED,
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at ASC LIMIT 3",
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC",
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') AND name <> 'zz' \
         ORDER BY created_at DESC LIMIT 2",
    ] {
        let plan = explain(&engine, sql).await;
        assert!(
            plan.contains("CompoundIndexScan")
                && plan.contains(BUILTIN)
                && plan.contains("owner: workspace ws"),
            "{sql}:\n{plan}"
        );
        assert!(
            !plan.contains("Sort") && !plan.contains("TopN"),
            "the order column elides the sort; {sql}:\n{plan}"
        );
    }
    // Rows: newest first, oldest first, the LIMIT respected.
    let newest = strings(
        &engine,
        "SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC",
        "name",
    )
    .await;
    let mut oldest = strings(
        &engine,
        "SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at ASC",
        "name",
    )
    .await;
    assert_eq!(newest.len(), 3);
    oldest.reverse();
    assert_eq!(newest, oldest);

    // Not a subtree index, and editorial listings keep the child scan.
    for sql in [
        "EXPLAIN SELECT name FROM 'ws' WHERE DESCENDANT_OF('/a') ORDER BY created_at DESC LIMIT 3",
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY __order",
        "EXPLAIN SELECT name FROM 'ws' WHERE CHILD_OF('/a')",
    ] {
        assert!(
            !explain(&engine, sql).await.contains(BUILTIN),
            "{sql} must not use the built-in index"
        );
    }
}

/// `(__parent_path, __node_type, __created_at)`: matches a typed listing
/// BETTER than the built-in (two equality columns against one).
fn folder_type_time() -> CompoundIndexDefinition {
    let mut index = folder_time();
    index.name = "folder_type_time".to_string();
    index.columns.insert(
        1,
        CompoundIndexColumn {
            property: "__node_type".to_string(),
            column_type: CompoundColumnType::String,
            ascending: None,
        },
    );
    index
}

/// An explicit index that matches as well wins (the tie goes to the user's
/// declaration), one that matches better wins, and an UNBUILT user index
/// never shadows the built-in one: the planner takes the first usable.
#[tokio::test]
async fn an_explicit_index_still_wins_and_an_unbuilt_one_does_not_shadow() {
    let repo = "r_cbl_win";
    let (engine, storage, _tmp) = setup_with(repo, Owner::Workspace, true, true).await;
    build_automatically(&storage, repo).await;
    assert!(
        explain(&engine, BARE).await.contains("@folder_time"),
        "same match: the declared index wins the tie"
    );

    // Declare a better typed index but do not build it yet.
    let declaration = serde_json::to_string(&vec![folder_time(), folder_type_time()]).unwrap();
    run(
        &engine,
        &format!(
            "UPDATE Workspaces SET compound_indexes = '{declaration}'::jsonb WHERE name = '{WS}'"
        ),
    )
    .await;
    raisin_sql_execution::invalidate_compound_index_cache();
    let typed = explain(&engine, TYPED).await;
    assert!(
        typed.contains("CompoundIndexScan") && !typed.contains("@folder_type_time"),
        "an unbuilt better index must not take the listing off the index:\n{typed}"
    );
    build_compound(&storage, repo).await;
    assert!(explain(&engine, TYPED).await.contains("@folder_type_time"));
    assert!(explain(&engine, BARE).await.contains("@folder_time"));
}

/// Opting out over SQL (`UPDATE Workspaces SET builtin_indexes = …`): the
/// planner stops using it at once (the record fails closed), the link drops
/// its entries, the column shows the switch; opting back in scans until the
/// next build, then serves again.
#[tokio::test]
async fn opting_out_and_back_in_over_sql() {
    let repo = "r_cbl_toggle";
    let (engine, storage, _tmp) = setup(repo, Owner::Builtin, false).await;
    build_automatically(&storage, repo).await;
    assert!(explain(&engine, BARE).await.contains(BUILTIN));

    run(
        &engine,
        &format!(
            "UPDATE Workspaces SET builtin_indexes = '{{\"children_by_created_at\": false}}'::jsonb \
             WHERE name = '{WS}'"
        ),
    )
    .await;
    assert!(
        !explain(&engine, BARE).await.contains("CompoundIndexScan"),
        "switched off: scans at once, even with the planner's declarations cached"
    );
    raisin_sql_execution::invalidate_compound_index_cache();
    let mut rows = engine
        .execute(&format!(
            "SELECT builtin_indexes FROM Workspaces WHERE name = '{WS}'"
        ))
        .await
        .expect("select");
    let row = rows.next().await.expect("a row").expect("decode");
    let shown = format!("{:?}", row.columns);
    assert!(
        shown.contains("children_by_created_at") && shown.contains("false"),
        "the column shows the switch in force: {shown}"
    );
    build_automatically(&storage, repo).await; // drops the entries
    let entries = raisin_rocksdb::indexing::compound::keyspace::count(
        storage.db(),
        TENANT,
        repo,
        "main",
        WS,
        BUILTIN,
    )
    .unwrap();
    assert_eq!(entries, 0, "the link dropped the switched-off index");
    assert!(!explain(&engine, BARE).await.contains("CompoundIndexScan"));

    run(
        &engine,
        &format!("UPDATE Workspaces SET builtin_indexes = NULL WHERE name = '{WS}'"),
    )
    .await;
    raisin_sql_execution::invalidate_compound_index_cache();
    assert!(
        !explain(&engine, BARE).await.contains("CompoundIndexScan"),
        "back on, not built yet"
    );
    build_automatically(&storage, repo).await;
    assert!(explain(&engine, BARE).await.contains(BUILTIN));
    assert_eq!(
        strings(
            &engine,
            "SELECT name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 2",
            "name"
        )
        .await
        .len(),
        2
    );
}
