//! `RESOLVE()`'s frontier engine: same output, fewer reads, one snapshot.
//!
//! The resolver was rewritten from "rewrite the whole document once per level"
//! to a frontier walk over targets with a per-statement memo. These pin that
//! the OUTPUT did not change (against a copy of the old algorithm), that a
//! target shared by every row is read once, that `RESOLVE(...)->>'k'` works,
//! and that a historical read inlines historical targets.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

mod fetch_count;
mod legacy;
use legacy::legacy_resolve;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "pages";

type Store = raisin_rocksdb::RocksDBStorage;

async fn setup() -> (Arc<Store>, QueryEngine<Store>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = Store::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
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
            serde_json::from_value(json!({ "name": "test:Doc" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system());
    (storage, engine, temp_dir)
}

async fn rows(engine: &QueryEngine<Store>, sql: &str) -> Vec<Value> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        out.push(serde_json::to_value(&row.columns).expect("row as json"));
    }
    out
}

async fn insert(engine: &QueryEngine<Store>, id: &str, path: &str, props: Value) {
    let props = props.to_string().replace('\'', "''");
    rows(
        engine,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('{id}', '{path}', 'test:Doc', '{props}'::jsonb)"
        ),
    )
    .await;
}

async fn one(engine: &QueryEngine<Store>, sql: &str) -> Value {
    rows(engine, sql).await[0]["r"].clone()
}

fn reference(id: &str) -> Value {
    json!({ "raisin:ref": id, "raisin:workspace": WS })
}

/// A cycle (`a` → `b` → `a`) and a shared DAG (`page` → `x`, `y`; both → `z`).
async fn seed_graph(engine: &QueryEngine<Store>) {
    insert(engine, "z", "/z", json!({ "title": "Z" })).await;
    insert(
        engine,
        "x",
        "/x",
        json!({ "title": "X", "z": reference("z") }),
    )
    .await;
    insert(
        engine,
        "y",
        "/y",
        json!({ "title": "Y", "zs": [reference("z"), reference("z")] }),
    )
    .await;
    insert(engine, "a", "/a", json!({ "title": "A" })).await;
    insert(
        engine,
        "b",
        "/b",
        json!({ "title": "B", "next": reference("a") }),
    )
    .await;
    rows(
        engine,
        &format!(
            "UPDATE '{WS}' SET properties = '{}'::jsonb WHERE path = '/a'",
            json!({ "title": "A", "next": reference("b") })
        ),
    )
    .await;
    insert(
        engine,
        "page",
        "/page",
        json!({
            "blocks": [{ "x": reference("x") }, { "y": reference("y") }],
            "loop": reference("a"),
            "gone": reference("missing"),
        }),
    )
    .await;
}

/// Every node as RESOLVE inlines it: identity members plus its properties.
async fn inlined_nodes(engine: &QueryEngine<Store>) -> HashMap<String, Value> {
    let found = rows(
        engine,
        &format!("SELECT id, name, path, node_type, properties FROM '{WS}'"),
    )
    .await;
    found
        .into_iter()
        .map(|row| {
            let mut node = Map::new();
            for key in ["id", "name", "path", "node_type"] {
                node.insert(key.to_string(), row[key].clone());
            }
            if let Some(props) = row["properties"].as_object() {
                node.extend(props.clone());
            }
            (
                row["id"].as_str().expect("id").to_string(),
                Value::Object(node),
            )
        })
        .collect()
}

#[tokio::test]
async fn resolve_cycle_and_shared_dag_output_unchanged() {
    let (_storage, engine, _dir) = setup().await;
    seed_graph(&engine).await;
    let nodes = inlined_nodes(&engine).await;
    let page = one(
        &engine,
        &format!("SELECT properties AS r FROM '{WS}' WHERE path = '/page'"),
    )
    .await;

    for depth in 0..=5u32 {
        let got = one(
            &engine,
            &format!("SELECT RESOLVE(properties, {depth}) AS r FROM '{WS}' WHERE path = '/page'"),
        )
        .await;
        assert_eq!(got, legacy_resolve(&page, depth, &nodes), "depth {depth}");
    }

    // Spot checks, so a broken oracle cannot pass silently.
    let got = one(
        &engine,
        &format!("SELECT RESOLVE(properties, 3) AS r FROM '{WS}' WHERE path = '/page'"),
    )
    .await;
    assert_eq!(got["blocks"][1]["y"]["zs"][1]["title"], "Z");
    assert_eq!(got["loop"]["next"]["next"]["title"], "A");
    assert_eq!(got["loop"]["next"]["next"]["next"]["raisin:ref"], "b");
    assert_eq!(got["gone"]["raisin:ref"], "missing");
}

#[tokio::test]
async fn resolve_json_extract_works() {
    let (_storage, engine, _dir) = setup().await;
    seed_graph(&engine).await;

    let found = rows(
        &engine,
        &format!(
            "SELECT RESOLVE(properties->'loop')->>'title' AS t, \
             RESOLVE(properties, 2)->'loop'->'next'->>'title' AS n \
             FROM '{WS}' WHERE path = '/page'"
        ),
    )
    .await;
    assert_eq!(found[0]["t"], "A");
    assert_eq!(found[0]["n"], "B");
}

/// A read pinned to an old revision inlines the target as it was then.
#[tokio::test]
async fn resolve_at_revision_inlines_historical_target() {
    let (storage, engine, _dir) = setup().await;
    insert(&engine, "img", "/img", json!({ "alt": "old" })).await;
    insert(
        &engine,
        "home",
        "/home",
        json!({ "image": reference("img") }),
    )
    .await;
    let then = storage
        .branches()
        .get_branch(TENANT, REPO, BRANCH)
        .await
        .expect("branch")
        .expect("exists")
        .head;
    rows(
        &engine,
        &format!("UPDATE '{WS}' SET properties = '{{\"alt\":\"new\"}}'::jsonb WHERE path = '/img'"),
    )
    .await;

    let now = one(
        &engine,
        &format!("SELECT RESOLVE(properties) AS r FROM '{WS}' WHERE path = '/home'"),
    )
    .await;
    assert_eq!(now["image"]["alt"], "new");

    let past = one(
        &engine,
        &format!(
            "SELECT RESOLVE(properties) AS r FROM '{WS}' \
             WHERE path = '/home' AND __revision = '{then}'"
        ),
    )
    .await;
    assert_eq!(past["image"]["alt"], "old");
}
