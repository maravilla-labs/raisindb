//! Plan Phase 7: `gc_then_restore` under skip-unchanged writes.
//!
//! With the delta writer an unchanged value's live entry sits at the revision
//! that FIRST wrote it, possibly far below a retained revision. History GC must
//! keep it answering there, and a restore — through `put_node`, the one restore
//! funnel — must leave the index exactly as it stood at the restored revision.
//!
//! Both doors: SQL `RESTORE NODE … TO REVISION`, and the write
//! `NodeService::restore_version` performs (historical content onto the current
//! node, `put_node`). `restore_version` itself cannot be driven on RocksDB: it
//! finds versions through a `TreeCommitMeta` nothing writes (plan Phase 10b,
//! "manual-version snapshot decode"), so its write is reproduced here verbatim.

use crate::mvcc_index_oracle::env::{query, Env, MAIN, PAGE, REPO, TENANT, WS};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage};
use std::collections::BTreeMap;
use std::time::Duration;

const VALUES: &[(&str, &[&str])] = &[
    ("title", &["t1", "t2", "t3", "t4", "t5", "t6"]),
    ("slug", &["s-a", "s-b"]),
    ("kind", &["k"]),
];

fn page(title: &str, slug: &str) -> Node {
    let mut properties = std::collections::HashMap::new();
    for (k, v) in [("title", title), ("slug", slug), ("kind", "k")] {
        properties.insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    Node {
        id: "n".to_string(),
        name: "n".to_string(),
        path: "/n".to_string(),
        parent: Some("/".to_string()),
        node_type: PAGE.to_string(),
        properties,
        ..Default::default()
    }
}

/// Whether `n` matches each `(property, value)` at `at`.
async fn index_state(env: &Env, at: Option<&HLC>) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    for (prop, values) in VALUES {
        for value in *values {
            let hits = env
                .storage
                .property_index()
                .find_by_property(
                    env.scope(MAIN),
                    prop,
                    &PropertyValue::String(value.to_string()),
                    false,
                    at,
                )
                .await
                .expect("index read");
            out.insert(format!("{prop}={value}"), hits.contains(&"n".to_string()));
        }
    }
    out
}

async fn put(env: &Env, node: &Node) {
    let ctx = env.tx(MAIN).await;
    ctx.put_node(WS, node).await.expect("put");
    ctx.commit().await.expect("commit");
}

/// Six revisions; `slug` and `kind` are written once and skipped after.
async fn history(env: &Env) -> Vec<HLC> {
    env.storage.nodes_impl().set_index_skip_unchanged(true);
    env.unlock_skip_unchanged(MAIN).await;
    let mut revs = Vec::new();
    for (i, title) in VALUES[0].1.iter().enumerate() {
        let slug = if i < 3 { "s-a" } else { "s-b" };
        put(env, &page(title, slug)).await;
        revs.push(env.head(MAIN).await);
    }
    let options = GcOptions {
        retention_override: Some(HistoryRetention {
            keep_days: None,
            keep_revisions: Some(3),
        }),
        tenant: Some(TENANT.to_string()),
        repo: Some(REPO.to_string()),
        min_age: Duration::ZERO,
        collect_orphaned_blobs: false,
        bound_job_results: false,
        sweep_unreferenced_blobs: false,
        compact: false,
        ..GcOptions::default()
    };
    run_history_gc(&env.storage, &options).expect("history gc");
    revs
}

async fn assert_restored(env: &Env, restored: &HLC, door: &str) {
    let then = index_state(env, Some(restored)).await;
    assert_eq!(
        then["kind=k"], true,
        "{door}: retained revision lost `kind`"
    );
    let now = index_state(env, None).await;
    assert_eq!(
        now, then,
        "{door}: index after restore != index at {restored}"
    );
}

#[tokio::test]
async fn gc_then_restore_through_sql_restore() {
    let env = Env::new(None).await;
    let revs = history(&env).await;
    let target = revs[4]; // retained (keep 3), and `slug = s-b`, `kind` skipped
    let sql = format!(
        "RESTORE NODE id='n' TO REVISION {}_{}",
        target.timestamp_ms, target.counter
    );
    query(&env.engine(MAIN), &sql).await.expect("restore");
    assert_restored(&env, &target, "SQL RESTORE").await;
}

#[tokio::test]
async fn gc_then_restore_through_restore_version_write() {
    let env = Env::new(None).await;
    let revs = history(&env).await;
    let target = revs[3];
    // `NodeService::restore_version`: the historical CONTENT onto the current
    // node (identity kept), through put_node.
    let historical = env
        .storage
        .nodes()
        .get(env.scope(MAIN), "n", Some(&target))
        .await
        .expect("read")
        .expect("node at the restored revision");
    let mut current = env
        .storage
        .nodes()
        .get(env.scope(MAIN), "n", None)
        .await
        .expect("read")
        .expect("node");
    current.properties = historical.properties;
    current.translations = historical.translations;
    current.node_type = historical.node_type;
    current.archetype = historical.archetype;
    current.updated_at = Some(chrono::Utc::now());
    put(&env, &current).await;
    assert_restored(&env, &target, "restore_version").await;
}
