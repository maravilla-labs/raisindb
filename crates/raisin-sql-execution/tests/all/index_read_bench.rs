//! Read-path baselines for the read/index/resolve plan (`docs/perf/`).
//!
//! Every scenario appends JSON lines to `target/bench/index_read.jsonl`, so a
//! run before a change and a run after it can be diffed. All `#[ignore]`:
//!
//! ```bash
//! cargo test --release -p raisin-sql-execution --test all index_read_bench -- --ignored --nocapture
//! ```
//!
//! - **A** — point lookup by an UNCHANGED text property
//!   (`properties->>'slug'::String = …`) on a node edited N times, among M
//!   distractors (`BENCH_M`, default 2000). N is 1, 10 and 100, plus 1000
//!   when `BENCH_FULL` is set. Measures what superseded index entries cost a
//!   lookup that should not care about them.
//! - **A2** — the same lookup with `LIMIT 1`, at the same edit counts.
//! - **B** — the same lookup at an early `__revision` (right after the node
//!   was created, before any of its edits), measured on the same data as A.
//! - **C** — `index_read_bench_resolve`: `RESOLVE(properties, 2)` over a
//!   header/footer/settings "chrome" node (25 references, 3 targets shared),
//!   as one statement and as 50 rows that all reference the chrome; **C3**,
//!   a 50-row listing whose rows share no target (100 distinct).
//!
//! - **H (writes)** — `index_read_bench_writes`: one property updated on a
//!   node with 30, `index.skip_unchanged` off and on (plan Phase 7).
//!   Every scenario here runs with the delta writer in effect (the default
//!   since plan Phase 7b; the branch is rebuilt at bootstrap);
//!   `RAISIN_INDEX_SKIP_UNCHANGED=0` runs them with full puts.
//!
//! - **E/F (paths)** — `index_read_bench_paths`: `get_by_path`, `WHERE path =`,
//!   `CHILD_OF` and `DESCENDANT_OF` on the legacy record format vs the one
//!   format (plan Phase 10b).
//!
//! - **P / R / L (plan Phase 13b, release build, p50/p95)** —
//!   `index_read_bench_point`: point lookups by path, id and indexed property,
//!   SQL vs the storage API (hot and fresh SQL text); and
//!   `index_read_bench_point_resolve`: RESOLVE over 50 references at depth 1
//!   and 2 vs the batched-read floor, and a localized path lookup at depth 3
//!   and 6 (replacing scenario I's `url_fr` property, which no longer exists).
//!
//! Set `RAISIN_SQL_PHASE_TIMING=1` to also get per-statement phase lines.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) const TENANT: &str = "bench";
pub(super) const REPO: &str = "bench";
pub(super) const BRANCH: &str = "main";
pub(super) const WS: &str = "site";
/// Timed repetitions per measurement; the median is reported.
pub(super) const REPS: usize = 50;

pub(super) type Store = raisin_rocksdb::RocksDBStorage;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub(super) async fn bootstrap() -> (QueryEngine<Store>, Arc<Store>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let storage = Arc::new(Store::new(dir.path()).expect("storage"));
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "bench", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(json!({ "name": "bench:Doc" })).expect("nt"),
            CommitMetadata::system("bench types"),
        )
        .await
        .expect("nodetype");
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system());
    // With `index.skip_unchanged` on (the default since Phase 7b) every
    // scenario runs with the Phase 7 delta writer in effect.
    unlock_skip_unchanged(&storage).await;
    (engine, storage, dir)
}

/// When `index.skip_unchanged` is on, rebuild the branch's property index so
/// the delta writer skips there (plan Phase 7); a no-op otherwise.
pub(super) async fn unlock_skip_unchanged(storage: &Store) {
    use raisin_rocksdb::management::async_indexing::repair::{
        run_repair, RepairKind, RepairOptions,
    };
    if !storage.nodes_impl().index_skip_unchanged() {
        return;
    }
    let options = RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    };
    run_repair(
        storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::PropertyIndex,
        options,
    )
    .await
    .expect("property_index rebuild");
}

/// The median of `samples`.
pub(super) fn median_of(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Run a statement to completion; returns the row count.
pub(super) async fn run(engine: &QueryEngine<Store>, sql: &str) -> usize {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut n = 0;
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        n += 1;
    }
    n
}

pub(super) fn jsonb(value: &Value) -> String {
    format!("'{}'::jsonb", value.to_string().replace('\'', "''"))
}

/// Insert `(id, path, properties)` rows, a few hundred per statement.
pub(super) async fn insert_many(engine: &QueryEngine<Store>, rows: &[(String, String, Value)]) {
    for chunk in rows.chunks(250) {
        let values: Vec<String> = chunk
            .iter()
            .map(|(id, path, props)| format!("('{id}', '{path}', 'bench:Doc', {})", jsonb(props)))
            .collect();
        run(
            engine,
            &format!(
                "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES {}",
                values.join(", ")
            ),
        )
        .await;
    }
}

/// Median wall time of `REPS` runs of `sql`, after one warm-up run.
pub(super) async fn median(engine: &QueryEngine<Store>, sql: &str, expect_rows: usize) -> Duration {
    assert_eq!(run(engine, sql).await, expect_rows, "{sql}");
    let mut samples = Vec::with_capacity(REPS);
    for _ in 0..REPS {
        let start = Instant::now();
        run(engine, sql).await;
        samples.push(start.elapsed());
    }
    samples.sort();
    samples[REPS / 2]
}

/// Append one result line to `target/bench/index_read.jsonl`.
pub(super) fn record(scenario: &str, params: Value, took: Duration) {
    let line = json!({
        "bench": "index_read",
        "scenario": scenario,
        "params": params,
        "median_us": took.as_micros() as u64,
        "reps": REPS,
        "unix_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
    });
    println!("{line}");
    let target = std::env::var("CARGO_TARGET_DIR")
        .unwrap_or_else(|_| format!("{}/../../target", env!("CARGO_MANIFEST_DIR")));
    let dir = std::path::Path::new(&target).join("bench");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("index_read.jsonl"))
    {
        let _ = writeln!(file, "{line}");
    }
}

/// Scenarios A and B: lookup by an unchanged slug on a node edited N times,
/// at HEAD (A) and at the revision right after the node was created (B).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "read-path baseline; run with --ignored --nocapture"]
async fn index_read_bench_a_lookup_by_unchanged_property() {
    let distractors = env_usize("BENCH_M", 2000);
    let mut edit_counts = vec![1usize, 10, 100];
    if std::env::var("BENCH_FULL").is_ok() {
        edit_counts.push(1000);
    }
    for edits in edit_counts {
        let (engine, storage, _dir) = bootstrap().await;
        let rows: Vec<(String, String, Value)> = (0..distractors)
            .map(|i| {
                (
                    format!("d{i}"),
                    format!("/d{i}"),
                    json!({ "slug": format!("slug-{i}"), "n": 0 }),
                )
            })
            .collect();
        insert_many(&engine, &rows).await;
        insert_many(
            &engine,
            &[(
                "target".to_string(),
                "/target".to_string(),
                json!({ "slug": "the-target", "n": 0 }),
            )],
        )
        .await;
        let early = storage
            .branches()
            .get_branch(TENANT, REPO, BRANCH)
            .await
            .expect("branch")
            .expect("exists")
            .head;
        for n in 1..edits {
            let props = json!({ "slug": "the-target", "n": n });
            run(
                &engine,
                &format!(
                    "UPDATE '{WS}' SET properties = {} WHERE path = '/target'",
                    jsonb(&props)
                ),
            )
            .await;
        }

        let params = json!({
            "edits": edits,
            "distractors": distractors,
            "skip_unchanged": storage.nodes_impl().index_skip_unchanged(),
        });
        let sql = format!("SELECT id FROM '{WS}' WHERE properties->>'slug'::String = 'the-target'");
        let took = median(&engine, &sql, 1).await;
        record("A_lookup_unchanged_property", params.clone(), took);

        // A2: the same lookup when the caller asks for one row — the shape of
        // a slug-to-page resolution, where the index can stop at the first hit.
        let one = format!("{sql} LIMIT 1");
        let took = median(&engine, &one, 1).await;
        record("A2_lookup_unchanged_property_limit_1", params.clone(), took);

        let at_early = format!("{sql} AND __revision = '{early}'");
        let took = median(&engine, &at_early, 1).await;
        record(
            "B_lookup_unchanged_property_at_early_revision",
            params,
            took,
        );
    }
}
