//! Localized-read throughput: what a `WHERE locale = …` predicate actually costs.
//!
//! Run:
//!   BENCH_N=5000 cargo test --release -p raisin-sql-execution --test all -- \
//!     --ignored --nocapture translation_throughput
//!
//! Three numbers, each a full SQL scan of the same N nodes:
//!
//!   base      no locale predicate — the floor
//!   locale    `AND locale = 'de'` with NOTHING translated — the tax every
//!             multilingual read pays for nodes that have no overlay
//!   resolved  the same read with every node translated — the work itself
//!
//! Why this exists: resolution used to walk each node's whole property tree and
//! then issue one storage read per block uuid it found, per locale in the fallback
//! chain, before it could conclude "no block overlays" — on every localized read of
//! every node. The inventory is now one prefix scan per node, reused across the
//! chain, and skipped entirely when empty. This test is the measurement that keeps
//! that honest: `locale` should sit within a small multiple of `base`, and the
//! per-node cost must not grow with the size of a node's content.

use futures::StreamExt;
use raisin_context::RepositoryConfig;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

const TENANT: &str = "bench_tenant";
const REPO: &str = "bench_repo";
const BRANCH: &str = "main";
const WS: &str = "pages";
fn bench_n() -> usize {
    std::env::var("BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

/// Blocks per node — the dimension the old per-block point reads scaled with.
/// Run the test at two values (`BENCH_BLOCKS=8` and `=64`) and the locale/base
/// ratio should not move: that is the N+1 being gone, stated as a measurement
/// rather than as a claim about the code.
fn bench_blocks() -> usize {
    std::env::var("BENCH_BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8)
}

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "bench", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
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
            serde_json::from_value(serde_json::json!({ "name": "bench:Page" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (storage, temp_dir)
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_repository_config(RepositoryConfig {
            default_language: "en".to_string(),
            supported_languages: vec!["en".into(), "de".into()],
            ..Default::default()
        })
        .with_auth(AuthContext::system())
}

async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> usize {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut n = 0;
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error: {e}"));
        n += 1;
    }
    n
}

/// One scan, timed, reported as total + per-node.
async fn timed(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    label: &str,
    sql: &str,
    n: usize,
) -> f64 {
    // Warm the block cache so the first query does not pay for all the others.
    run(engine, sql).await;
    let start = Instant::now();
    let rows = run(engine, sql).await;
    let elapsed = start.elapsed();
    let per_node_us = elapsed.as_secs_f64() * 1e6 / n as f64;
    println!(
        "  {label:<10} {:>8.1} ms   {:>7.2} µs/node   ({rows} rows)",
        elapsed.as_secs_f64() * 1000.0,
        per_node_us
    );
    per_node_us
}

#[tokio::test]
#[ignore = "localized read throughput baseline; run with --ignored --nocapture"]
async fn translation_throughput() {
    let n = bench_n();
    let blocks = bench_blocks();
    let (storage, _td) = setup().await;
    let e = engine(&storage);

    println!("\n=== localized read throughput (RocksDB, real QueryEngine) ===");
    println!("BENCH_N = {n}, {blocks} blocks/node\n");

    let seed = Instant::now();
    for i in 0..n {
        let content: Vec<_> = (0..blocks)
            .map(|b| {
                // Plain uuid-keyed objects, not `element_type` blocks: an element
                // type would have to be registered and validated, and the resolver
                // walks both shapes the same way.
                serde_json::json!({
                    "uuid": format!("n{i}-b{b}"),
                    "headline": format!("Headline {i}/{b}"),
                })
            })
            .collect();
        let props = serde_json::json!({ "title": format!("Page {i}"), "content": content });
        run(
            &e,
            &format!(
                "INSERT INTO {WS} (id, path, node_type, properties) VALUES \
                 ('p{i}','/p{i}','bench:Page','{props}'::JSONB)"
            ),
        )
        .await;
    }
    println!("seeded in {:.1}s\n", seed.elapsed().as_secs_f64());

    let select = format!("SELECT path, properties FROM {WS}");
    let base = timed(&e, "base", &select, n).await;
    let untranslated = timed(&e, "locale", &format!("{select} WHERE locale = 'de'"), n).await;

    // Translate every node — one nested leaf each, so resolution has real work.
    let translate = Instant::now();
    for i in 0..n {
        run(
            &e,
            &format!(
                "UPDATE {WS} FOR LOCALE 'de' SET title = 'Seite {i}', \
                 content[uuid='n{i}-b0'].headline = 'Schlagzeile {i}' WHERE path = '/p{i}'"
            ),
        )
        .await;
    }
    println!(
        "\ntranslated in {:.1}s\n",
        translate.elapsed().as_secs_f64()
    );

    let resolved = timed(&e, "resolved", &format!("{select} WHERE locale = 'de'"), n).await;

    println!(
        "\n  locale/base   {:>5.2}x      resolved/base {:>5.2}x",
        untranslated / base,
        resolved / base
    );
    println!(
        "\n  The `locale` row is the one to watch: it is the cost of ASKING for a\n  \
         locale on content that has none. It must not scale with blocks/node.\n"
    );
}
