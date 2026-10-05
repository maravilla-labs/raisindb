//! Plan Phase 13f, measurement: what the BUILT-IN `(__parent_path,
//! __created_at)` index costs writes, and what it wins a newest-first folder
//! listing. Two fresh repositories, identical but for
//! `config.builtin_indexes.children_by_created_at`, run the same writes:
//! 10k creates under one folder, 10k updates (a property, so the index tuple
//! is unchanged), 2k moves to another folder, 2k deletes. Then the read: a
//! bare `CHILD_OF ORDER BY created_at DESC LIMIT 20` over the 10k-child folder,
//! served by the built-in index versus the scan it replaces (same data, the
//! index failed closed).
//!
//! `cargo test -p raisin-sql-execution --test all compound_builtin_measure
//! -- --ignored --nocapture` (debug build; numbers in the plan).

use super::compound_builtin_listing::build_automatically;
use super::compound_index_hierarchy::{
    explain, setup_with, Engine, Owner, BRANCH, NOTE_TYPE, TENANT, WS,
};
use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope, UpdateNodeOptions};
use std::time::Instant;

const CHILDREN: usize = 10_000;
const MOVES: usize = 2_000;
const DELETES: usize = 2_000;
const RUNS: usize = 30;
const SQL: &str =
    "SELECT id, name FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 20";

#[derive(Debug, Default, Clone, Copy)]
struct Costs {
    create: f64,
    update: f64,
    mv: f64,
    delete: f64,
}

fn per_sec(n: usize, secs: f64) -> f64 {
    n as f64 / secs
}

async fn writes(storage: &RocksDBStorage, repo: &str) -> Costs {
    let scope = StorageScope::new(TENANT, repo, BRANCH, WS);
    let opts = || CreateNodeOptions {
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        ..Default::default()
    };
    storage
        .nodes()
        .create(
            scope,
            Node {
                id: "b".to_string(),
                path: "/b".to_string(),
                name: "b".to_string(),
                parent: Some("/".to_string()),
                node_type: NOTE_TYPE.to_string(),
                ..Default::default()
            },
            opts(),
        )
        .await
        .expect("create /b");
    let base = chrono::Utc::now() - chrono::Duration::days(1);
    let mut costs = Costs::default();

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
            .create(scope, node, opts())
            .await
            .expect("create");
    }
    costs.create = per_sec(CHILDREN, started.elapsed().as_secs_f64());

    let started = Instant::now();
    for i in 0..CHILDREN {
        let mut node = storage
            .nodes()
            .get(scope, &format!("c{i:05}"), None)
            .await
            .expect("get")
            .expect("node");
        node.properties
            .insert("title".to_string(), PropertyValue::String(format!("t{i}")));
        storage
            .nodes()
            .update(scope, node, UpdateNodeOptions::default())
            .await
            .expect("update");
    }
    costs.update = per_sec(CHILDREN, started.elapsed().as_secs_f64());

    let started = Instant::now();
    for i in 0..MOVES {
        storage
            .nodes()
            .move_node(scope, &format!("c{i:05}"), &format!("/b/c{i:05}"), None)
            .await
            .expect("move");
    }
    costs.mv = per_sec(MOVES, started.elapsed().as_secs_f64());

    let started = Instant::now();
    for i in MOVES..MOVES + DELETES {
        storage
            .nodes()
            .delete(scope, &format!("c{i:05}"), Default::default())
            .await
            .expect("delete");
    }
    costs.delete = per_sec(DELETES, started.elapsed().as_secs_f64());
    costs
}

async fn p50(engine: &Engine) -> f64 {
    let mut times = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let started = Instant::now();
        let mut stream = engine.execute(SQL).await.expect("query");
        let mut rows = 0;
        while let Some(row) = stream.next().await {
            row.expect("row");
            rows += 1;
        }
        assert_eq!(rows, 20);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[RUNS / 2]
}

#[tokio::test]
#[ignore]
async fn compound_builtin_measure() {
    // OFF first, then ON, then each again (the second pair in the other
    // order), so warm-up and disk state do not favour one side.
    let mut off_runs = Vec::new();
    let mut on_runs = Vec::new();
    let mut read = (0.0, 0.0);
    for (round, order) in [[false, true], [true, false]].into_iter().enumerate() {
        for builtin in order {
            let repo = format!("r_cbm_{}_{round}", if builtin { "on" } else { "off" });
            let (engine, storage, _tmp) = setup_with(&repo, Owner::Builtin, false, builtin).await;
            if builtin {
                build_automatically(&storage, &repo).await; // over the 4-node seed
            }
            let costs = writes(&storage, &repo).await;
            eprintln!("round {round} builtin={builtin}: {costs:?} (ops/s)");
            if builtin {
                on_runs.push(costs);
                if round == 0 {
                    // Refill the folder to 10k children for the read.
                    for i in 0..(MOVES + DELETES) {
                        storage
                            .nodes()
                            .create(
                                StorageScope::new(TENANT, &repo, BRANCH, WS),
                                Node {
                                    id: format!("d{i:05}"),
                                    path: format!("/a/d{i:05}"),
                                    name: format!("d{i:05}"),
                                    parent: Some("a".to_string()),
                                    node_type: NOTE_TYPE.to_string(),
                                    ..Default::default()
                                },
                                CreateNodeOptions {
                                    validate_parent_allows_child: false,
                                    validate_workspace_allows_type: false,
                                    ..Default::default()
                                },
                            )
                            .await
                            .expect("refill");
                    }
                    let plan = explain(&engine, &format!("EXPLAIN {SQL}")).await;
                    assert!(plan.contains("@__children_by_created_at"), "{plan}");
                    let after = p50(&engine).await;
                    raisin_rocksdb::compound_state::CompoundStateStore::new(storage.db().clone())
                        .mark_workspace_stale(TENANT, &repo, BRANCH, WS)
                        .expect("mark");
                    let plan = explain(&engine, &format!("EXPLAIN {SQL}")).await;
                    assert!(!plan.contains("CompoundIndexScan"), "{plan}");
                    let before = p50(&engine).await;
                    read = (before, after);
                }
            } else {
                off_runs.push(costs);
            }
        }
    }
    let mean =
        |runs: &[Costs], f: fn(&Costs) -> f64| runs.iter().map(f).sum::<f64>() / runs.len() as f64;
    for (what, f) in [
        ("create", (|c: &Costs| c.create) as fn(&Costs) -> f64),
        ("update", |c: &Costs| c.update),
        ("move", |c: &Costs| c.mv),
        ("delete", |c: &Costs| c.delete),
    ] {
        let (off, on) = (mean(&off_runs, f), mean(&on_runs, f));
        eprintln!(
            "{what}: built-in off {off:.0}/s, on {on:.0}/s ({:+.1}% throughput)",
            (on / off - 1.0) * 100.0
        );
    }
    eprintln!(
        "newest-20 of {CHILDREN} children, p50 of {RUNS}: scan {:.2} ms, built-in index {:.2} ms ({:.0}x)",
        read.0,
        read.1,
        read.0 / read.1
    );
}
