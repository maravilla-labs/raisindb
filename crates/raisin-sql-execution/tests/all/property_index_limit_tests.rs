//! `LIMIT` reaches the property index.
//!
//! A slug-to-page lookup (`WHERE properties->>'slug'::String = $1 LIMIT 1`) used
//! to read every index entry for the value: the driving equality stayed as a
//! residual filter above the scan, and a LIMIT cannot be pushed under a filter
//! that may drop rows. The scan now verifies that equality itself, so the LIMIT
//! bounds the index read — and a candidate the scan drops (denied by RLS,
//! hidden in the locale, an orphan entry) is refilled from the index instead of
//! costing the caller a row.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
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
const WS: &str = "pages";

type Store = raisin_rocksdb::RocksDBStorage;

async fn setup() -> (Arc<Store>, TempDir) {
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
            serde_json::from_value(serde_json::json!({ "name": "test:Page" })).expect("nt"),
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

fn engine(storage: &Arc<Store>, auth: AuthContext) -> QueryEngine<Store> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(auth)
}

/// May read `/open/**` only.
fn open_reader() -> AuthContext {
    AuthContext::for_user("reader").with_permissions(ResolvedPermissions {
        user_id: "reader".into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new("/open/**", vec![Operation::Read]).with_workspace(WS)],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
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

async fn insert(engine: &QueryEngine<Store>, path: &str, slug: &str) {
    rows(
        engine,
        &format!(
            "INSERT INTO '{WS}' (path, node_type, properties) \
             VALUES ('{path}', 'test:Page', '{{\"slug\":\"{slug}\"}}'::jsonb)"
        ),
    )
    .await;
}

fn paths(found: &[Value]) -> Vec<String> {
    found
        .iter()
        .map(|r| r["path"].as_str().expect("path").to_string())
        .collect()
}

#[tokio::test]
async fn limit_reaches_the_property_index_scan() {
    let (storage, _dir) = setup().await;
    let sys = engine(&storage, AuthContext::system());

    for sql in [
        format!("EXPLAIN SELECT id FROM '{WS}' WHERE properties->>'slug'::String = 'x' LIMIT 1"),
        format!("EXPLAIN SELECT id FROM '{WS}' WHERE properties->>'slug' = 'x' LIMIT 1"),
    ] {
        let found = rows(&sys, &sql).await;
        let plan = found[0]["QUERY PLAN"].as_str().expect("plan");
        assert!(
            plan.contains("PropertyIndexScan: slug=x limit=1"),
            "{sql}\n{plan}"
        );
    }

    // OFFSET is added to the bound: the scan must produce the skipped rows too.
    let sql = format!(
        "EXPLAIN SELECT id FROM '{WS}' WHERE properties->>'slug'::String = 'x' LIMIT 1 OFFSET 2"
    );
    let found = rows(&sys, &sql).await;
    let plan = found[0]["QUERY PLAN"].as_str().expect("plan");
    assert!(plan.contains("PropertyIndexScan: slug=x limit=3"), "{plan}");
}

/// The first candidates the index returns are ones the caller may not read;
/// `LIMIT 1` must still return the one it may.
///
/// The readable node is written FIRST and the denied ones after it: the index
/// orders a value's entries newest-first, so the denied candidates lead.
#[tokio::test]
async fn limit_one_skips_a_denied_first_candidate() {
    let (storage, _dir) = setup().await;
    let sys = engine(&storage, AuthContext::system());
    insert(&sys, "/open", "folder").await;
    insert(&sys, "/secret", "folder").await;
    insert(&sys, "/open/page", "x").await;
    for i in 0..5 {
        insert(&sys, &format!("/secret/p{i}"), "x").await;
    }

    let reader = engine(&storage, open_reader());
    let lookup = |limit: &str| {
        format!("SELECT path FROM '{WS}' WHERE properties->>'slug'::String = 'x' {limit}")
    };
    assert_eq!(
        paths(&rows(&reader, &lookup("LIMIT 1")).await),
        ["/open/page"]
    );
    assert_eq!(paths(&rows(&reader, &lookup("")).await), ["/open/page"]);

    // A system caller sees all six, and LIMIT still bounds it exactly.
    assert_eq!(rows(&sys, &lookup("")).await.len(), 6);
    assert_eq!(rows(&sys, &lookup("LIMIT 2")).await.len(), 2);
    assert_eq!(rows(&sys, &lookup("LIMIT 2 OFFSET 5")).await.len(), 1);
}

/// The equality the scan now verifies itself keeps the residual's semantics: a
/// node whose stored value changed matches its new value and not its old one.
#[tokio::test]
async fn verified_equality_matches_the_row_filter() {
    let (storage, _dir) = setup().await;
    let sys = engine(&storage, AuthContext::system());
    insert(&sys, "/a", "old").await;
    rows(
        &sys,
        &format!("UPDATE '{WS}' SET properties = '{{\"slug\":\"new\"}}'::jsonb WHERE path = '/a'"),
    )
    .await;

    let lookup = |v: &str| {
        format!("SELECT path FROM '{WS}' WHERE properties->>'slug'::String = '{v}' LIMIT 1")
    };
    assert!(rows(&sys, &lookup("old")).await.is_empty());
    assert_eq!(paths(&rows(&sys, &lookup("new")).await), ["/a"]);
}
