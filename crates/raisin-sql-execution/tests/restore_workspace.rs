//! Regression test: RESTORE must resolve the node's real workspace.
//!
//! RESTORE carries no workspace (no FROM clause). The executor previously
//! hardcoded `workspace = "default"`, so restoring a node that lives in any
//! other workspace returned NotFound and silently did nothing. This verifies
//! RESTORE locates the node across workspaces and actually reverts content.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";

async fn engine_with_stories() -> (QueryEngine<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage"));
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await
        .ok();

    // A "default" workspace exists but does NOT contain our node — the node
    // lives in "stories". The old hardcoded-"default" code would miss it.
    for ws in ["default", "stories"] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .expect("create workspace");
    }

    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "story:Doc" })).unwrap(),
            CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("default".to_string());
    catalog.register_workspace("stories".to_string());

    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(raisin_models::auth::AuthContext::system());

    (engine, temp_dir)
}

async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

async fn read_title(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) -> Option<String> {
    let mut stream = engine
        .execute("SELECT properties->>'title'::String AS title FROM stories WHERE path = '/demo'")
        .await
        .expect("select title");
    let mut out = None;
    while let Some(row) = stream.next().await {
        let row = row.expect("row");
        if let Some(raisin_models::nodes::properties::PropertyValue::String(s)) =
            row.columns.get("title")
        {
            out = Some(s.clone());
        }
    }
    out
}

#[tokio::test]
async fn restore_resolves_non_default_workspace() {
    let (engine, _td) = engine_with_stories().await;

    // Revision 1: create in the "stories" workspace.
    run(
        &engine,
        "INSERT INTO stories (id, path, node_type, properties) VALUES \
         ('s1','/demo','story:Doc','{\"title\":\"v1\"}'::JSONB)",
    )
    .await;
    // Revision 2: update the title.
    run(
        &engine,
        "UPDATE stories SET properties = '{\"title\":\"v2\"}'::JSONB WHERE path = '/demo'",
    )
    .await;
    assert_eq!(read_title(&engine).await.as_deref(), Some("v2"));

    // RESTORE carries no workspace — the executor must locate '/demo' in the
    // "stories" workspace (not "default") and revert it to the prior revision.
    run(&engine, "RESTORE NODE path='/demo' TO REVISION HEAD~1").await;

    assert_eq!(
        read_title(&engine).await.as_deref(),
        Some("v1"),
        "RESTORE should have reverted the node in the 'stories' workspace"
    );
}
