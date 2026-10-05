//! Scenario P of the read bench (plan Phase 13b): what a SQL point lookup
//! costs over the storage read it is built on, in a RELEASE build.
//!
//! ```bash
//! cargo test --release -p raisin-sql-execution --test all index_read_bench_p -- --ignored --nocapture
//! ```
//!
//! - **P** — one row by path, by id and by an indexed property, through SQL
//!   (`SELECT *` and `SELECT id`) and through the storage API directly
//!   (`get_by_path`, `get`, `find_by_property_with_limit` + `get`), among
//!   `BENCH_M` (default 2000) distractors. Each line carries p50/p95 over
//!   `BENCH_REPS` (default 400) runs and, for SQL, the ratio to its storage
//!   counterpart.
//! - **R / L** — `index_read_bench_point_resolve`: RESOLVE over 50 references
//!   at depth 1 and 2, and a localized path lookup at depth 3 and 6.
//!
//! **Profiling.** `BENCH_HOT=<case>` (a case name printed by the run, e.g.
//! `sql_path_star`) with `BENCH_HOT_SECS=<n>` loops that one case for `n`
//! seconds after measuring it, after printing `HOT <case> pid=<pid>`, so a
//! sampling profiler (`sample <pid> 5`) can attach.

use super::index_read_bench::{bootstrap, insert_many, run, BRANCH, REPO, TENANT, WS};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage, StorageScope};
use serde_json::{json, Value};
use std::future::Future;
use std::io::Write;
use std::time::{Duration, Instant};

pub(super) fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// p50 and p95 of one measured case.
#[derive(Clone, Copy)]
pub(super) struct Pct {
    pub p50: Duration,
    pub p95: Duration,
}

/// `BENCH_REPS` timed runs of `op` after a few warm-up runs.
pub(super) async fn pct<F, Fut>(name: &str, mut op: F) -> Pct
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let reps = env_usize("BENCH_REPS", 400).max(20);
    for _ in 0..10 {
        op().await;
    }
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        let start = Instant::now();
        op().await;
        samples.push(start.elapsed());
    }
    samples.sort();
    let out = Pct {
        p50: samples[reps / 2],
        p95: samples[(reps * 95 / 100).min(reps - 1)],
    };
    hot_loop(name, &mut op).await;
    out
}

/// Loop `op` for `BENCH_HOT_SECS` when `BENCH_HOT` names this case.
async fn hot_loop<F, Fut>(name: &str, op: &mut F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    if std::env::var("BENCH_HOT").ok().as_deref() != Some(name) {
        return;
    }
    let secs = env_usize("BENCH_HOT_SECS", 10) as u64;
    println!("HOT {name} pid={}", std::process::id());
    let _ = std::io::stdout().flush();
    let until = Instant::now() + Duration::from_secs(secs);
    let mut n = 0u64;
    while Instant::now() < until {
        op().await;
        n += 1;
    }
    println!("HOT {name} done: {n} runs in {secs}s");
}

/// One JSON line (stdout and `target/bench/index_read.jsonl`) for a case,
/// with its ratio to `baseline` when one is given.
pub(super) fn record_pct(scenario: &str, mut params: Value, took: Pct, baseline: Option<Pct>) {
    if let Some(base) = baseline {
        let ratio = took.p50.as_secs_f64() / base.p50.as_secs_f64().max(1e-9);
        params["ratio_to_storage_p50"] = json!((ratio * 100.0).round() / 100.0);
    }
    params["build"] = json!(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    let line = json!({
        "bench": "index_read",
        "scenario": scenario,
        "params": params,
        "p50_us": took.p50.as_secs_f64() * 1e6,
        "p95_us": took.p95.as_secs_f64() * 1e6,
        "unix_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
    });
    println!("{line}");
    let target = std::env::var("CARGO_TARGET_DIR")
        .unwrap_or_else(|_| format!("{}/../../target", env!("CARGO_MANIFEST_DIR")));
    let dir = std::path::Path::new(&target).join("bench");
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("index_read.jsonl"))
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// A page-sized document: what a point lookup materializes.
pub(super) fn doc(i: usize) -> Value {
    json!({
        "slug": format!("slug-{i}"),
        "title": format!("Document {i}"),
        "summary": "A short summary of the document that a listing would show.",
        "body": "lorem ipsum dolor sit amet ".repeat(20),
        "tags": ["news", "product", "release"],
        "rank": i,
        "published": true,
    })
}

/// Scenario P: point lookups through SQL vs the storage API.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "read-path baseline; run with --release --ignored --nocapture"]
async fn index_read_bench_p_point_lookups() {
    let distractors = env_usize("BENCH_M", 2000);
    let (engine, storage, _dir) = bootstrap().await;
    let rows: Vec<(String, String, Value)> = (0..distractors)
        .map(|i| (format!("d{i}"), format!("/d{i}"), doc(i)))
        .collect();
    insert_many(&engine, &rows).await;
    let target = distractors / 2;
    let (id, path, slug) = (
        format!("d{target}"),
        format!("/d{target}"),
        format!("slug-{target}"),
    );
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    let params = json!({ "distractors": distractors });

    let st_path = pct("storage_get_by_path", || async {
        let n = storage.nodes().get_by_path(scope, &path, None).await;
        assert!(n.unwrap().is_some());
    })
    .await;
    let st_id = pct("storage_get", || async {
        let n = storage.nodes().get(scope, &id, None).await;
        assert!(n.unwrap().is_some());
    })
    .await;
    let value = PropertyValue::String(slug.clone());
    let st_prop = pct("storage_property", || async {
        let ids = storage
            .property_index()
            .find_by_property_with_limit(scope, "slug", &value, false, None, Some(1))
            .await
            .unwrap();
        let n = storage.nodes().get(scope, &ids[0], None).await;
        assert!(n.unwrap().is_some());
    })
    .await;
    record_pct("P_storage_get_by_path", params.clone(), st_path, None);
    record_pct("P_storage_get_by_id", params.clone(), st_id, None);
    record_pct("P_storage_property_lookup", params.clone(), st_prop, None);

    let cases: [(&str, String, Pct); 6] = [
        (
            "sql_path_star",
            format!("SELECT * FROM '{WS}' WHERE path = '{path}'"),
            st_path,
        ),
        (
            "sql_path_id",
            format!("SELECT id FROM '{WS}' WHERE path = '{path}'"),
            st_path,
        ),
        (
            "sql_id_star",
            format!("SELECT * FROM '{WS}' WHERE id = '{id}'"),
            st_id,
        ),
        (
            "sql_id_id",
            format!("SELECT id FROM '{WS}' WHERE id = '{id}'"),
            st_id,
        ),
        (
            "sql_prop_star",
            format!("SELECT * FROM '{WS}' WHERE properties->>'slug'::String = '{slug}'"),
            st_prop,
        ),
        (
            "sql_prop_star_limit_1",
            format!("SELECT * FROM '{WS}' WHERE properties->>'slug'::String = '{slug}' LIMIT 1"),
            st_prop,
        ),
    ];
    for (name, sql, baseline) in cases {
        assert_eq!(run(&engine, &sql).await, 1, "{sql}");
        let took = pct(name, || async {
            run(&engine, &sql).await;
        })
        .await;
        record_pct(&format!("P_{name}"), params.clone(), took, Some(baseline));
    }

    // The batch entry point the HTTP, WS and pgwire transports call.
    let star = format!("SELECT * FROM '{WS}' WHERE path = '{path}'");
    let took = pct("sql_path_star_batch", || async {
        let mut stream = engine.execute_batch(&star).await.expect("batch");
        while let Some(row) = futures::StreamExt::next(&mut stream).await {
            row.expect("row");
        }
    })
    .await;
    record_pct("P_sql_path_star_batch", params.clone(), took, Some(st_path));

    // A prepared statement (plan Phase 13d): ONE text with `$1`, a different
    // value every run — what pgwire's Parse/Bind/Execute, and HTTP/WS/function
    // calls with `params`, hand the engine (`execute_batch_with_params`).
    for (name, columns) in [("sql_path_star_param", "*"), ("sql_path_id_param", "id")] {
        let sql = format!("SELECT {columns} FROM '{WS}' WHERE path = $1");
        let next = std::cell::Cell::new(0usize);
        let took = pct(name, || {
            let i = next.get();
            next.set(i + 1);
            let params = [json!(format!("/d{}", i % distractors))];
            let (engine, sql) = (&engine, &sql);
            async move {
                let mut stream = engine
                    .execute_batch_with_params(
                        sql,
                        &params,
                        &raisin_sql_execution::format_param_value,
                    )
                    .await
                    .expect("prepared");
                let mut n = 0;
                while let Some(row) = futures::StreamExt::next(&mut stream).await {
                    row.expect("row");
                    n += 1;
                }
                assert_eq!(n, 1);
            }
        })
        .await;
        record_pct(&format!("P_{name}"), params.clone(), took, Some(st_path));
    }

    // A FRESH SQL text every run (another node's path): what the statement
    // costs when the prepared-statement cache cannot help.
    for (name, columns) in [("sql_path_star_fresh", "*"), ("sql_path_id_fresh", "id")] {
        let next = std::cell::Cell::new(0usize);
        let took = pct(name, || {
            let i = next.get();
            next.set(i + 1);
            let sql = format!(
                "SELECT {columns} FROM '{WS}' WHERE path = '/d{}'",
                i % distractors
            );
            let engine = &engine;
            async move {
                assert_eq!(run(engine, &sql).await, 1);
            }
        })
        .await;
        record_pct(&format!("P_{name}"), params.clone(), took, Some(st_path));
    }
}
