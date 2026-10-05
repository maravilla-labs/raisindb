//! Path reads on the LEGACY record format vs the ONE format (plan Phase 10b).
//!
//! ```bash
//! cargo test -p raisin-sql-execution --test all index_read_bench_paths -- --ignored --nocapture
//! ```
//!
//! A tree of `1 + SECTIONS + SECTIONS * PAGES` nodes (311 by default) is
//! written through SQL — the transaction path — and read four ways:
//! `get_by_path`, `SELECT … WHERE path = …`, `CHILD_OF` (one section) and
//! `DESCENDANT_OF` (the whole tree), plus `get` by id as a control. Three
//! stores, identical content:
//!
//! - `one_format` — what every writer stores now: a `StorageNode` blob and a
//!   `NODE_PATH` entry;
//! - `legacy` — every record rewritten raw into what the transaction path
//!   stored before Phase 10: the full `Node`, path embedded, NO `NODE_PATH`;
//! - `legacy_backfilled` — `legacy` after the `node_path` backfill (the blobs
//!   still embed their paths; `NODE_PATH` now answers too).
//!
//! Each line reports the median wall time over `REPS` runs and the RocksDB
//! operations ONE run moved on this thread (`PerfContext`; work moved to
//! another thread is not counted).

use super::index_read_bench::{
    bootstrap, insert_many, record, run, Store, BRANCH, REPO, REPS, TENANT, WS,
};
use futures::StreamExt;
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use raisin_sql_execution::QueryEngine;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use rocksdb::perf::{set_perf_stats, PerfContext, PerfMetric, PerfStatsLevel};
use serde_json::{json, Value};
use std::future::Future;
use std::time::{Duration, Instant};

const SECTIONS: usize = 10;
const PAGES: usize = 30;

async fn build_tree(engine: &QueryEngine<Store>) {
    let row = |path: String| {
        let id = path.trim_start_matches('/').replace('/', "-");
        (id, path, json!({ "title": "t" }))
    };
    insert_many(engine, &[row("/root".into())]).await;
    let sections: Vec<_> = (0..SECTIONS).map(|s| row(format!("/root/s{s}"))).collect();
    insert_many(engine, &sections).await;
    let pages: Vec<_> = (0..SECTIONS)
        .flat_map(|s| (0..PAGES).map(move |p| format!("/root/s{s}/p{p}")))
        .map(row)
        .collect();
    insert_many(engine, &pages).await;
}

/// TEST-ONLY: rewrite every node record of the workspace raw into the
/// pre-Phase-10 transaction format. Returns how many records it rewrote.
fn rewrite_as_legacy(storage: &Store) -> usize {
    let db = storage.db();
    let cf_nodes = db.cf_handle(raisin_rocksdb::cf::NODES).unwrap();
    let cf_node_path = db.cf_handle(raisin_rocksdb::cf::NODE_PATH).unwrap();
    let prefix = format!("{TENANT}\0{REPO}\0{BRANCH}\0{WS}\0nodes\0").into_bytes();
    let rows: Vec<(Box<[u8]>, Box<[u8]>)> = db
        .prefix_iterator_cf(cf_nodes, &prefix)
        .flatten()
        .take_while(|(k, _)| k.starts_with(&prefix))
        .collect();
    let mut rewritten = 0;
    for (key, value) in rows {
        let rest = &key[prefix.len()..];
        let Some(id) = rest
            .len()
            .checked_sub(17)
            .and_then(|n| std::str::from_utf8(&rest[..n]).ok())
        else {
            continue;
        };
        if id.contains('\0') || raisin_rocksdb::keys::is_tombstone_value(&value) {
            continue;
        }
        let rev = raisin_rocksdb::keys::extract_revision_from_key(&key).unwrap();
        let entry =
            raisin_rocksdb::keys::node_path_key_versioned(TENANT, REPO, BRANCH, WS, id, &rev);
        let Some(path) = db.get_cf(cf_node_path, &entry).unwrap() else {
            continue;
        };
        let (mut node, _) = raisin_rocksdb::decode_node_blob(&value).unwrap();
        node.path = String::from_utf8(path).unwrap();
        db.put_cf(cf_nodes, &key, rmp_serde::to_vec_named(&node).unwrap())
            .unwrap();
        db.delete_cf(cf_node_path, &entry).unwrap();
        rewritten += 1;
    }
    rewritten
}

/// RocksDB operations one future moved on this thread.
async fn count_ops<F: Future>(fut: F) -> (F::Output, Value) {
    set_perf_stats(PerfStatsLevel::EnableCount);
    let mut ctx = PerfContext::default();
    ctx.reset();
    let out = fut.await;
    let ops = json!({
        "gets": ctx.metric(PerfMetric::GetFromMemtableCount),
        "seeks": ctx.metric(PerfMetric::SeekOnMemtableCount),
        "seek_child": ctx.metric(PerfMetric::SeekChildSeekCount),
        "nexts": ctx.metric(PerfMetric::NextOnMemtableCount),
        "keys_skipped": ctx.metric(PerfMetric::InternalKeySkippedCount),
    });
    set_perf_stats(PerfStatsLevel::Disable);
    (out, ops)
}

/// Median of `REPS` timed runs of `op` (after one warm-up), and the
/// operation counts of one run.
async fn measure<F, Fut>(mut op: F) -> (Duration, Value)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = usize>,
{
    op().await;
    let (_, ops) = count_ops(op()).await;
    let mut samples = Vec::with_capacity(REPS);
    for _ in 0..REPS {
        let start = Instant::now();
        op().await;
        samples.push(start.elapsed());
    }
    samples.sort();
    (samples[REPS / 2], ops)
}

async fn explain(engine: &QueryEngine<Store>, sql: &str) -> String {
    let mut stream = engine
        .execute(&format!("EXPLAIN {sql}"))
        .await
        .expect("explain");
    match stream.next().await {
        Some(Ok(row)) => format!("{:?}", row.columns.get("QUERY PLAN")),
        other => format!("{other:?}"),
    }
}

async fn measure_all(format: &str, engine: &QueryEngine<Store>, storage: &Store) {
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let target = "/root/s5/p17";
    let path_eq = format!("SELECT id FROM '{WS}' WHERE path = '{target}'");
    let child_of = format!("SELECT id, path FROM '{WS}' WHERE CHILD_OF('/root/s5')");
    let descendant_of = format!("SELECT id, path FROM '{WS}' WHERE DESCENDANT_OF('/root')");
    let descendants = SECTIONS + SECTIONS * PAGES;
    for (sql, rows) in [
        (&path_eq, 1),
        (&child_of, PAGES),
        (&descendant_of, descendants),
    ] {
        assert_eq!(run(engine, sql).await, rows, "{format}: {sql}");
    }

    let cases: Vec<(&str, Value, (Duration, Value))> = vec![
        (
            "get_by_path",
            json!({ "path": target }),
            measure(|| async {
                let n = storage.nodes().get_by_path(scope, target, None).await;
                assert_eq!(n.unwrap().expect("found").path, target);
                1
            })
            .await,
        ),
        (
            "get_by_id",
            json!({ "id": "root-s5-p17" }),
            measure(|| async {
                let n = storage.nodes().get(scope, "root-s5-p17", None).await;
                assert_eq!(n.unwrap().expect("found").path, target);
                1
            })
            .await,
        ),
        (
            "sql_path_eq",
            json!({ "rows": 1 }),
            measure(|| run(engine, &path_eq)).await,
        ),
        (
            "sql_child_of",
            json!({ "rows": PAGES }),
            measure(|| run(engine, &child_of)).await,
        ),
        (
            "sql_descendant_of",
            json!({ "rows": descendants }),
            measure(|| run(engine, &descendant_of)).await,
        ),
    ];
    for (case, mut params, (took, ops)) in cases {
        params["format"] = json!(format);
        params["ops_one_run"] = ops;
        record(&format!("E_path_{case}"), params, took);
    }
    for sql in [&path_eq, &child_of, &descendant_of] {
        println!("{format} EXPLAIN {sql}\n  {}", explain(engine, sql).await);
    }
}

/// Scenarios E/F: the same tree on the three formats.
#[tokio::test]
#[ignore = "read-path baseline; run with --ignored --nocapture"]
async fn index_read_bench_paths_legacy_vs_one_format() {
    for format in ["one_format", "legacy", "legacy_backfilled"] {
        let (engine, storage, _dir) = bootstrap().await;
        build_tree(&engine).await;
        if format != "one_format" {
            let rewritten = rewrite_as_legacy(&storage);
            assert_eq!(rewritten, 1 + SECTIONS + SECTIONS * PAGES);
        }
        if format == "legacy_backfilled" {
            let options = RepairOptions {
                check_headroom: false,
                max_bytes_per_sec: 0,
                ..RepairOptions::default()
            };
            run_repair(
                &storage,
                TENANT,
                REPO,
                Some(BRANCH),
                RepairKind::NodePath,
                options,
            )
            .await
            .expect("backfill");
        }
        measure_all(format, &engine, &storage).await;
    }
}
