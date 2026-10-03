//! The PROPERTY_INDEX read at one revision, end to end.
//!
//! Every property-index executor now reads the index AS OF the statement's
//! snapshot, which is also the revision the matched nodes are decoded at. These
//! pin the four shapes that used to disagree with the node records: a timestamp
//! order after an update, timestamp range bounds, a pseudo-property lookup at a
//! past revision, and a pushed-down COUNT at a past revision.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "items";

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
            serde_json::from_value(serde_json::json!({ "name": "test:Item" })).expect("nt"),
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

fn names(found: &[Value]) -> Vec<String> {
    found
        .iter()
        .map(|r| r["name"].as_str().expect("name").to_string())
        .collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

async fn insert(engine: &QueryEngine<Store>, name: &str, status: &str) {
    rows(
        engine,
        &format!(
            "INSERT INTO '{WS}' (id, path, node_type, properties) \
             VALUES ('{name}', '/{name}', 'test:Item', '{{\"status\":\"{status}\"}}'::jsonb)"
        ),
    )
    .await;
}

async fn set_status(engine: &QueryEngine<Store>, name: &str, status: &str) {
    rows(
        engine,
        &format!(
            "UPDATE '{WS}' SET properties = '{{\"status\":\"{status}\"}}'::jsonb \
             WHERE path = '/{name}'"
        ),
    )
    .await;
}

async fn head(storage: &Arc<Store>) -> raisin_hlc::HLC {
    storage
        .branches()
        .get_branch(TENANT, REPO, BRANCH)
        .await
        .expect("branch")
        .expect("exists")
        .head
}

/// A strictly later wall-clock instant, at nanosecond precision so a bound
/// built from it lands BETWEEN two microseconds.
async fn instant() -> String {
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let t = chrono::Utc::now() + chrono::Duration::nanoseconds(500);
    let t = t.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    t
}

/// An updated node sorts by its NEW `updated_at`, once — not at the position
/// of the entry its update superseded, and not twice.
#[tokio::test]
async fn order_by_updated_at_asc_includes_updated_nodes() {
    let (_storage, engine, _dir) = setup().await;
    for name in ["a", "b", "c"] {
        insert(&engine, name, "new").await;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    set_status(&engine, "a", "edited").await;

    let asc = names(
        &rows(
            &engine,
            &format!("SELECT name FROM '{WS}' ORDER BY updated_at ASC LIMIT 10"),
        )
        .await,
    );
    assert_eq!(asc.last().map(String::as_str), Some("a"), "{asc:?}");
    assert_eq!(asc.iter().filter(|n| *n == "a").count(), 1, "{asc:?}");

    let desc = names(
        &rows(
            &engine,
            &format!("SELECT name FROM '{WS}' ORDER BY updated_at DESC LIMIT 1"),
        )
        .await,
    );
    assert_eq!(desc, ["a"]);
}

/// Lower and upper bounds on `updated_at` are both applied, exactly, including
/// a bound that falls between two microseconds.
#[tokio::test]
async fn updated_at_range_lower_and_upper_bounds() {
    let (_storage, engine, _dir) = setup().await;
    for name in ["a", "b", "c"] {
        insert(&engine, name, "new").await;
    }
    let t0 = instant().await;
    set_status(&engine, "a", "edited").await;
    let t1 = instant().await;
    set_status(&engine, "b", "edited").await;
    let t2 = instant().await;
    set_status(&engine, "c", "edited").await;

    let range = |cond: String| format!("SELECT name FROM '{WS}' WHERE {cond}");
    let ts = |t: &str| format!("'{t}'::TIMESTAMPTZ");

    let both = range(format!(
        "updated_at >= {} AND updated_at <= {}",
        ts(&t1),
        ts(&t2)
    ));
    let plan = rows(&engine, &format!("EXPLAIN {both}")).await;
    let plan = plan[0]["QUERY PLAN"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(plan.contains("PropertyRangeScan"), "{plan}");
    assert_eq!(names(&rows(&engine, &both).await), ["b"]);

    let lower = range(format!("updated_at > {}", ts(&t1)));
    assert_eq!(sorted(names(&rows(&engine, &lower).await)), ["b", "c"]);

    let upper = range(format!("updated_at < {}", ts(&t2)));
    assert_eq!(sorted(names(&rows(&engine, &upper).await)), ["a", "b"]);

    let all = range(format!("updated_at >= {}", ts(&t0)));
    assert_eq!(sorted(names(&rows(&engine, &all).await)), ["a", "b", "c"]);
}

/// A pseudo-property lookup at a past revision sees the nodes of that revision.
#[tokio::test]
async fn name_eq_at_historical_revision() {
    let (storage, engine, _dir) = setup().await;
    insert(&engine, "gone", "x").await;
    let then = head(&storage).await;
    rows(&engine, &format!("DELETE FROM '{WS}' WHERE path = '/gone'")).await;
    insert(&engine, "later", "x").await;

    let at = |name: &str, rev: Option<&raisin_hlc::HLC>| match rev {
        Some(rev) => {
            format!("SELECT name FROM '{WS}' WHERE name = '{name}' AND __revision = '{rev}'")
        }
        None => format!("SELECT name FROM '{WS}' WHERE name = '{name}'"),
    };
    assert_eq!(
        names(&rows(&engine, &at("gone", Some(&then))).await),
        ["gone"]
    );
    assert!(rows(&engine, &at("gone", None)).await.is_empty());
    assert!(rows(&engine, &at("later", Some(&then))).await.is_empty());
    assert_eq!(names(&rows(&engine, &at("later", None)).await), ["later"]);
}

async fn count(engine: &QueryEngine<Store>, sql: &str) -> i64 {
    let found = rows(engine, sql).await;
    let row = found[0].as_object().expect("one row");
    let value = row.values().next().expect("one column").clone();
    match serde_json::from_value::<PropertyValue>(value.clone()) {
        Ok(PropertyValue::Integer(n)) => n,
        _ => value.as_i64().unwrap_or_else(|| panic!("count: {value}")),
    }
}

/// COUNT(*) at a past revision counts that revision's matches — through the
/// pushed-down user-property count and through the pseudo-property path that
/// is no longer pushed down.
#[tokio::test]
async fn count_at_historical_revision() {
    let (storage, engine, _dir) = setup().await;
    for name in ["a", "b", "c"] {
        insert(&engine, name, "open").await;
    }
    let then = head(&storage).await;
    set_status(&engine, "a", "closed").await;
    rows(&engine, &format!("DELETE FROM '{WS}' WHERE path = '/b'")).await;

    let status = |rev: Option<&raisin_hlc::HLC>| {
        let at = rev
            .map(|r| format!(" AND __revision = '{r}'"))
            .unwrap_or_default();
        format!("SELECT COUNT(*) FROM '{WS}' WHERE properties->>'status'::String = 'open'{at}")
    };
    assert_eq!(count(&engine, &status(Some(&then))).await, 3);
    assert_eq!(count(&engine, &status(None)).await, 1);

    let typed = |rev: Option<&raisin_hlc::HLC>| {
        let at = rev
            .map(|r| format!(" AND __revision = '{r}'"))
            .unwrap_or_default();
        format!("SELECT COUNT(*) FROM '{WS}' WHERE node_type = 'test:Item'{at}")
    };
    assert_eq!(count(&engine, &typed(Some(&then))).await, 3);
    assert_eq!(count(&engine, &typed(None)).await, 2);
}
