//! `MOVE`, `COPY` and `ORDER` all treat the workspace root the same way.
//!
//! `/` addresses a workspace's top level. It is a valid DESTINATION for a move
//! or a copy — it is where every root-level node already lives — but it is not
//! itself a node, so it can never be the thing being moved, copied or ordered.
//!
//! The analyzer used one validator for both roles, so `MOVE … TO path='/'` was
//! refused before it ever reached the executor, and the refusal came back as
//! `ORDER: Root node '/' cannot be reordered` — an ORDER error for a MOVE. The
//! executor then had its own parent-exists lookup that `/` could not satisfy.
//! Together they meant nothing could be moved or copied to the top level of a
//! workspace, which is what broke package installs when the installer began
//! adopting (moving) root-level nodes.

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    CreateNodeOptions, NodeRepository, NodeTypeRepository, Storage, StorageScope,
    WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "menu";

async fn create_test_storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    use raisin_storage::BranchRepository;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path())
        .expect("Failed to create RocksDB storage");

    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;

    let storage = Arc::new(storage);

    // DML validates against the REGISTERED workspace and node type, not just the
    // SQL catalog, so both have to exist before a statement can write.
    storage
        .workspaces()
        .put(
            raisin_storage::RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("create workspace");
    storage
        .node_types()
        .create(
            raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "raisin:Folder" }))
                .expect("nodetype"),
            raisin_storage::CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");

    (storage, temp_dir)
}

fn engine(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage,
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    // Seeding and DML both write, so the engine needs an authority.
    .with_auth(raisin_models::auth::AuthContext::system())
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

fn node(id: &str, path: &str, parent: &str) -> Node {
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        parent: Some(parent.to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

async fn create_node(storage: &Arc<raisin_rocksdb::RocksDBStorage>, n: Node) {
    storage
        .nodes()
        .create(
            scope(),
            n,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to create node");
}

/// Seed through SQL, not the repository.
///
/// A DML statement runs inside a transaction whose own read path does not see
/// nodes written directly through the repository in this harness — an ordinary
/// folder-to-folder MOVE fails with `Node <id> not found` when seeded that way.
/// Inserting through the engine puts them where the statement will look.
async fn seed(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, paths: &[&str]) {
    for path in paths {
        let id = path.trim_start_matches('/').replace('/', "-");
        run(
            engine,
            &format!(
                "INSERT INTO menu (id, path, node_type, properties) \
                 VALUES ('{id}', '{path}', 'raisin:Folder', '{{}}'::JSONB)"
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("seeding {path} failed: {e}"));
    }
}

/// Drain a statement, returning the error string when it fails.
async fn run(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
) -> Result<(), String> {
    let mut stream = engine.execute(sql).await.map_err(|e| e.to_string())?;
    while let Some(row) = stream.next().await {
        row.map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn exists(storage: &Arc<raisin_rocksdb::RocksDBStorage>, path: &str) -> bool {
    storage
        .nodes()
        .get_by_path(scope(), path, None)
        .await
        .expect("read should succeed")
        .is_some()
}

/// CONTROL: an ordinary move between two folders, to tell a harness problem
/// apart from a root-handling one.
#[tokio::test]
async fn control_move_between_two_ordinary_folders() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = engine(storage.clone());
    seed(&engine, &["/holder", "/other", "/holder/movable"]).await;

    run(
        &engine,
        "MOVE menu SET path='/holder/movable' TO path='/other'",
    )
    .await
    .expect("an ordinary MOVE must work in this harness");

    assert!(exists(&storage, "/other/movable").await);
}

#[tokio::test]
async fn move_to_the_workspace_root() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = engine(storage.clone());
    seed(&engine, &["/holder", "/holder/movable"]).await;

    run(&engine, "MOVE menu SET path='/holder/movable' TO path='/'")
        .await
        .expect("a MOVE to the workspace root must be accepted and executed");

    assert!(
        exists(&storage, "/movable").await,
        "the node must land at the workspace root"
    );
    assert!(
        !exists(&storage, "/holder/movable").await,
        "and must not remain at its old path"
    );
}

#[tokio::test]
async fn copy_to_the_workspace_root() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = engine(storage.clone());
    seed(&engine, &["/holder", "/holder/source"]).await;

    run(
        &engine,
        "COPY menu SET path='/holder/source' TO path='/' AS 'copied'",
    )
    .await
    .expect("a COPY to the workspace root must be accepted and executed");

    assert!(
        exists(&storage, "/copied").await,
        "the copy must land at the workspace root, not at '//copied'"
    );
    assert!(
        exists(&storage, "/holder/source").await,
        "a copy leaves the source alone"
    );
}

/// The root is a destination, never the thing being operated on — and the
/// refusal must name the statement it came from.
#[tokio::test]
async fn the_root_itself_can_never_be_the_source() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = engine(storage.clone());
    seed(&engine, &["/holder"]).await;

    let moved = run(&engine, "MOVE menu SET path='/' TO path='/holder'").await;
    let move_err = moved.expect_err("the workspace root cannot be moved");
    assert!(
        move_err.contains("MOVE"),
        "a MOVE failure must report itself as a MOVE error, not borrow ORDER's: {move_err}"
    );

    let copied = run(&engine, "COPY menu SET path='/' TO path='/holder'").await;
    let copy_err = copied.expect_err("the workspace root cannot be copied");
    assert!(
        copy_err.contains("COPY"),
        "a COPY failure must report itself as a COPY error: {copy_err}"
    );
}

/// Ordering root-level siblings already worked; pin it so the three statements
/// stay consistent about what `/` means.
#[tokio::test]
async fn order_siblings_that_live_at_the_workspace_root() {
    let (storage, _tmp) = create_test_storage().await;
    let engine = engine(storage.clone());
    seed(&engine, &["/first", "/second"]).await;

    run(&engine, "ORDER menu SET path='/second' ABOVE path='/first'")
        .await
        .expect("root-level siblings must be orderable");

    let err = run(&engine, "ORDER menu SET path='/' ABOVE path='/first'")
        .await
        .expect_err("the root itself is not orderable");
    assert!(
        err.contains("ORDER"),
        "and that refusal belongs to ORDER: {err}"
    );
}
