//! Plan Phase 13e: a bare folder listing served by the WORKSPACE's compound
//! index answers exactly what the scan answers — mixed node types in the
//! folder, deletes, moves out of and into it (a leaf and a subtree), updates,
//! LIMIT/OFFSET pages and a keyset page — while every write keeps the index
//! `Ready` (each comparison also asserts the listing is still index-served).
//! Plan Phase 13f: the same for the BUILT-IN `@__children_by_created_at`.

use super::compound_index_hierarchy::{
    explain, setup, strings, Engine, Owner, BRANCH, NODE_TYPE, NOTE_TYPE, TENANT, WS,
};
use chrono::{DateTime, Duration, TimeZone, Utc};
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope, UpdateNodeOptions};
use std::collections::HashMap;

fn scope(repo: &str) -> StorageScope<'_> {
    StorageScope::new(TENANT, repo, BRANCH, WS)
}

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap()
}

fn at(secs: i64) -> DateTime<Utc> {
    base() + Duration::seconds(secs)
}

fn child(id: &str, path: &str, node_type: &str, created: i64) -> Node {
    let parent_path = path.rsplitn(2, '/').nth(1).unwrap_or("");
    let parent = parent_path.rsplit('/').next().filter(|p| !p.is_empty());
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap().to_string(),
        parent: Some(parent.unwrap_or("/").to_string()),
        node_type: node_type.to_string(),
        properties: HashMap::new(),
        created_at: Some(at(created)),
        ..Default::default()
    }
}

async fn create(storage: &RocksDBStorage, repo: &str, node: Node) {
    storage
        .nodes()
        .create(
            scope(repo),
            node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("create");
}

/// The index-served listing of `parent` against the same listing written so
/// no compound index can serve it (a path-prefix scan + sort): equal, page by
/// page, and the first one really is index-served.
async fn assert_listing_matches_scan(engine: &Engine, index: &str, parent: &str, step: &str) {
    let indexed = |extra: &str| {
        format!("SELECT id FROM 'ws' WHERE CHILD_OF('{parent}') ORDER BY created_at DESC{extra}")
    };
    let scanned = |extra: &str| {
        format!(
            "SELECT id FROM 'ws' WHERE path LIKE '{parent}/%' AND path NOT LIKE '{parent}/%/%' \
             ORDER BY created_at DESC{extra}"
        )
    };
    let plan = explain(engine, &format!("EXPLAIN {}", indexed(" LIMIT 2"))).await;
    assert!(
        plan.contains("CompoundIndexScan") && plan.contains(index),
        "{step}: {parent} listing not index-served:\n{plan}"
    );
    let scan_plan = explain(engine, &format!("EXPLAIN {}", scanned(""))).await;
    assert!(
        !scan_plan.contains("CompoundIndexScan"),
        "{step}: the reference query must not use the index:\n{scan_plan}"
    );

    let all = strings(engine, &scanned(""), "id").await;
    assert_eq!(
        strings(engine, &indexed(""), "id").await,
        all,
        "{step}: {parent} full listing"
    );
    for offset in (0..=all.len()).step_by(2) {
        let page = format!(" LIMIT 2 OFFSET {offset}");
        assert_eq!(
            strings(engine, &indexed(&page), "id").await,
            strings(engine, &scanned(&page), "id").await,
            "{step}: {parent} page at offset {offset}"
        );
    }
}

#[tokio::test]
async fn the_workspace_index_answers_exactly_what_the_scan_answers() {
    let (engine, storage, _tmp) = setup("r_cwl", Owner::Workspace, true).await;
    scenario(&engine, &storage, "r_cwl", "@folder_time").await;
}

/// Plan Phase 13f: the built-in index, built by the automatic
/// `compound_builds` link, answers exactly what the scan answers too.
#[tokio::test]
async fn the_builtin_index_answers_exactly_what_the_scan_answers() {
    let repo = "r_cwl_builtin";
    let (engine, storage, _tmp) = setup(repo, Owner::Builtin, false).await;
    super::compound_builtin_listing::build_automatically(&storage, repo).await;
    scenario(&engine, &storage, repo, "@__children_by_created_at").await;
}

/// Mixed types, deletes, leaf and subtree moves, an update, every page and a
/// keyset page — each equal to the scan, each still served by `index`.
async fn scenario(engine: &Engine, storage: &RocksDBStorage, repo: &str, index: &str) {
    // `/a` holds m0..m2 (test:Message, created now); mix in notes, a folder
    // with a grandchild (never a child of `/a`), and a second folder.
    create(storage, repo, child("n0", "/a/n0", NOTE_TYPE, 10)).await;
    create(storage, repo, child("n1", "/a/n1", NOTE_TYPE, 20)).await;
    create(storage, repo, child("m3", "/a/m3", NODE_TYPE, 30)).await;
    create(storage, repo, child("b", "/b", NOTE_TYPE, 1)).await;
    create(storage, repo, child("x0", "/b/x0", NODE_TYPE, 40)).await;
    create(storage, repo, child("x1", "/b/x1", NOTE_TYPE, 50)).await;
    create(storage, repo, child("sub", "/a/sub", NOTE_TYPE, 60)).await;
    create(storage, repo, child("deep", "/a/sub/deep", NODE_TYPE, 70)).await;
    assert_listing_matches_scan(engine, index, "/a", "initial").await;
    assert_listing_matches_scan(engine, index, "/b", "initial").await;
    assert_eq!(
        strings(
            engine,
            "SELECT id FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC",
            "id"
        )
        .await
        .len(),
        7,
        "every type's children of /a, and only children"
    );

    for id in ["n0", "m1"] {
        storage
            .nodes()
            .delete(scope(repo), id, Default::default())
            .await
            .expect("delete");
    }
    assert_listing_matches_scan(engine, index, "/a", "after deletes").await;

    storage
        .nodes()
        .move_node(scope(repo), "m3", "/b/m3", None)
        .await
        .expect("move out");
    storage
        .nodes()
        .move_node(scope(repo), "x1", "/a/x1", None)
        .await
        .expect("move in");
    assert_listing_matches_scan(engine, index, "/a", "after leaf moves").await;
    assert_listing_matches_scan(engine, index, "/b", "after leaf moves").await;

    storage
        .nodes()
        .move_node_tree(scope(repo), "sub", "/b/sub", None)
        .await
        .expect("move subtree");
    assert_listing_matches_scan(engine, index, "/a", "after subtree move").await;
    assert_listing_matches_scan(engine, index, "/b", "after subtree move").await;
    assert_listing_matches_scan(engine, index, "/b/sub", "after subtree move").await;

    let mut n1 = storage
        .nodes()
        .get(scope(repo), "n1", None)
        .await
        .expect("get")
        .expect("n1");
    n1.properties.insert(
        "title".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("edited".to_string()),
    );
    storage
        .nodes()
        .update(scope(repo), n1, UpdateNodeOptions::default())
        .await
        .expect("update");
    assert_listing_matches_scan(engine, index, "/a", "after update").await;

    // A keyset page: everything older than a cursor.
    let cursor = at(35).to_rfc3339();
    let keyset = |base: &str| {
        format!("{base} AND created_at < '{cursor}'::TIMESTAMPTZ ORDER BY created_at DESC LIMIT 2")
    };
    let indexed = keyset("SELECT id FROM 'ws' WHERE CHILD_OF('/a')");
    let scanned = keyset("SELECT id FROM 'ws' WHERE path LIKE '/a/%' AND path NOT LIKE '/a/%/%'");
    assert!(explain(engine, &format!("EXPLAIN {indexed}"))
        .await
        .contains("CompoundIndexScan"));
    let page = strings(engine, &indexed, "id").await;
    assert_eq!(page, strings(engine, &scanned, "id").await, "keyset page");
    assert_eq!(
        page,
        ["n1"],
        "only n1 is older than the cursor and still in /a"
    );
}
