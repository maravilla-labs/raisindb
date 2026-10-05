//! Plan Phase 13e, measurement: a bare `CHILD_OF ORDER BY created_at DESC
//! LIMIT 20` over a folder of 10k children, served by the workspace index
//! versus the scan it replaces (PrefixScan + sort), on the same data.
//!
//! `cargo test -p raisin-sql-execution --test all compound_workspace_measure
//! -- --ignored --nocapture` (debug build is fine; numbers in the plan).

use super::compound_index_hierarchy::{explain, setup, Owner, BRANCH, NOTE_TYPE, TENANT, WS};
use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::time::Instant;

const REPO: &str = "r_cwm";
const CHILDREN: usize = 10_000;
const RUNS: usize = 30;
const SQL: &str =
    "SELECT id, name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 20";

async fn p50(engine: &super::compound_index_hierarchy::Engine) -> (f64, usize) {
    let mut times = Vec::with_capacity(RUNS);
    let mut rows = 0;
    for _ in 0..RUNS {
        let started = Instant::now();
        let mut stream = engine.execute(SQL).await.expect("query");
        rows = 0;
        while let Some(row) = stream.next().await {
            row.expect("row");
            rows += 1;
        }
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (times[RUNS / 2], rows)
}

#[tokio::test]
#[ignore]
async fn compound_workspace_measure() {
    let (engine, storage, _tmp) = setup(REPO, Owner::Workspace, true).await;
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let base = chrono::Utc::now() - chrono::Duration::days(1);
    let started = Instant::now();
    for i in 0..CHILDREN {
        let node = Node {
            id: format!("c{i:05}"),
            path: format!("/a/c{i:05}"),
            name: format!("c{i:05}"),
            parent: Some("a".to_string()),
            node_type: NOTE_TYPE.to_string(),
            created_at: Some(base + chrono::Duration::milliseconds(i as i64)),
            ..Default::default()
        };
        storage
            .nodes()
            .create(
                scope,
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
    eprintln!("created {CHILDREN} children in {:?}", started.elapsed());

    let plan = explain(&engine, &format!("EXPLAIN {SQL}")).await;
    assert!(plan.contains("CompoundIndexScan"), "{plan}");
    let (after, rows) = p50(&engine).await;
    assert_eq!(rows, 20);

    // The same data with the index failed closed: the planner scans.
    raisin_rocksdb::compound_state::CompoundStateStore::new(storage.db().clone())
        .mark_workspace_stale(TENANT, REPO, BRANCH, WS)
        .expect("mark");
    let plan = explain(&engine, &format!("EXPLAIN {SQL}")).await;
    assert!(!plan.contains("CompoundIndexScan"), "{plan}");
    let (before, rows) = p50(&engine).await;
    assert_eq!(rows, 20);
    eprintln!(
        "bare CHILD_OF ORDER BY created_at DESC LIMIT 20, {CHILDREN} children, p50 of {RUNS}: \
         scan {before:.2} ms, workspace index {after:.2} ms ({:.0}x)",
        before / after
    );
}
