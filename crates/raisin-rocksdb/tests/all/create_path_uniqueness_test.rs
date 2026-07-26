//! Regression tests for path uniqueness under concurrent node creation.
//!
//! Node creation used to enforce path uniqueness with a plain read-check
//! ("does a node exist at this path?") followed by a later batch write —
//! a TOCTOU race. Two concurrent creates for the same (workspace, path)
//! could both pass the check before either write landed, producing TWO
//! physical rows at one path (observed live: indexed scans returned two
//! rows for one path, and deleting by path left orphaned index entries).
//!
//! The fix adds an in-process CREATE path-reservation registry shared by
//! the transactional (`add_node`) and non-transactional
//! (`NodeRepository::create`) write paths: a creator reserves the scoped
//! path BEFORE the committed-storage existence check and releases it only
//! after its write is durable (or on rollback/drop). Whoever loses the
//! reservation either gets a Conflict immediately or sees the winner's
//! committed row in the existence check.
//!
//! These tests force the race deterministically (barrier + concurrent
//! tasks) and assert exactly one create wins and exactly one row exists.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, CreateNodeOptions, ListOptions, NodeRepository,
    NodeTypeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "pathuniq-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn build_node(path: &str, title: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("user".to_string()),
        created_by: Some("user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

async fn setup_storage() -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let repo_config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: Some("Create path uniqueness test".to_string()),
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    // Seed the node type BEFORE creating the workspace: WorkspaceService::put
    // bootstraps a ROOT node for new workspaces, which requires the default
    // folder type to already be registered.
    let folder_type = NodeType {
        id: Some("raisin:Folder".to_string()),
        strict: Some(false),
        name: "raisin:Folder".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            folder_type,
            CommitMetadata::system("seed folder type"),
        )
        .await?;

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    Ok((storage, temp_dir))
}

/// Build a node destined for `create_deep_node`: name/parent left for the
/// deep-create implementation to derive from the path.
fn build_deep_node(path: &str, title: &str) -> Node {
    let mut node = build_node(path, title);
    node.name = String::new();
    node.parent = None;
    node
}

/// Run one full transactional create (begin -> add_node -> commit).
async fn transactional_create(storage: Arc<RocksDBStorage>, node: Node) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("create contested node")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.add_node(WORKSPACE, &node).await?;
    tx.commit().await
}

/// Run one full transactional put (begin -> put_node -> commit).
/// `put_node` is create-or-update by ID: a fresh id at a path is a CREATE.
async fn transactional_put(storage: Arc<RocksDBStorage>, node: Node) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("put contested node")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.put_node(WORKSPACE, &node).await?;
    tx.commit().await
}

/// Count live rows whose path equals `path` among `parent`'s children (each
/// duplicate physical row has a distinct id and its own index entries, so
/// duplicates show up as extra children).
async fn count_rows_under(storage: &RocksDBStorage, parent: &str, path: &str) -> Result<usize> {
    let children = storage
        .nodes()
        .list_children(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            parent,
            ListOptions {
                compute_has_children: false,
                max_revision: None,
            },
        )
        .await?;
    Ok(children.iter().filter(|n| n.path == path).count())
}

async fn count_rows_at_path(storage: &RocksDBStorage, path: &str) -> Result<usize> {
    count_rows_under(storage, "/", path).await
}

/// Two transactions racing to create the SAME path must yield exactly one
/// node: one commit succeeds, the other errors with a conflict.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_transactional_creates_same_path_yield_single_node() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let mut handles = Vec::new();
    for i in 0..2 {
        let storage = storage.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            let node = build_node("/contested", &format!("Writer {}", i));
            barrier.wait().await;
            transactional_create(storage, node).await
        }));
    }

    let mut successes = 0;
    let mut failures = Vec::new();
    for handle in handles {
        match handle.await.expect("task panicked") {
            Ok(()) => successes += 1,
            Err(e) => failures.push(e),
        }
    }

    assert_eq!(
        successes, 1,
        "exactly one of two racing creates for the same path must succeed \
         (got {} successes, failures: {:?})",
        successes, failures
    );

    // Exactly one physical row must exist at the contested path.
    let row_count = count_rows_at_path(&storage, "/contested").await?;
    assert_eq!(
        row_count, 1,
        "exactly one physical row must exist at /contested after the race"
    );

    // And the winner must be readable by path.
    let read = storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/contested",
            None,
        )
        .await?;
    assert!(read.is_some(), "winning node must be readable by path");

    Ok(())
}

/// Two direct repository creates (non-transactional path) racing to create
/// the SAME path must also yield exactly one node.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_repository_creates_same_path_yield_single_node() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let mut handles = Vec::new();
    for i in 0..2 {
        let storage = storage.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            let node = build_node("/contested-repo", &format!("Writer {}", i));
            barrier.wait().await;
            storage
                .nodes()
                .create(
                    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                    node,
                    CreateNodeOptions::default(),
                )
                .await
        }));
    }

    let mut successes = 0;
    let mut failures = Vec::new();
    for handle in handles {
        match handle.await.expect("task panicked") {
            Ok(()) => successes += 1,
            Err(e) => failures.push(e),
        }
    }

    assert_eq!(
        successes, 1,
        "exactly one of two racing repository creates for the same path must succeed \
         (got {} successes, failures: {:?})",
        successes, failures
    );

    let row_count = count_rows_at_path(&storage, "/contested-repo").await?;
    assert_eq!(
        row_count, 1,
        "exactly one physical row must exist at /contested-repo after the race"
    );

    Ok(())
}

/// Sequential semantics: creating a node at an already-occupied path is a
/// hard error (Conflict), never a silent success or an implicit upsert —
/// on BOTH the transactional and the direct repository create paths.
#[tokio::test]
async fn sequential_recreate_same_path_errors() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    // First create (transactional path) succeeds.
    transactional_create(storage.clone(), build_node("/only-once", "First")).await?;

    // Second transactional create at the same path must error.
    let err = transactional_create(storage.clone(), build_node("/only-once", "Second"))
        .await
        .expect_err("re-creating an existing path via a transaction must error");
    assert!(
        err.to_string().contains("already exists"),
        "error should identify the path conflict, got: {}",
        err
    );

    // Direct repository create at the same path must also error.
    let err = storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            build_node("/only-once", "Third"),
            CreateNodeOptions::default(),
        )
        .await
        .expect_err("re-creating an existing path via the repository must error");
    assert!(
        err.to_string().contains("already exists"),
        "error should identify the path conflict, got: {}",
        err
    );

    // Still exactly one row at the path.
    let row_count = count_rows_at_path(&storage, "/only-once").await?;
    assert_eq!(row_count, 1);

    Ok(())
}

/// A failed / rolled-back / dropped creator must NOT leave the path
/// permanently reserved: a later create for the same path must succeed.
#[tokio::test]
async fn reservation_released_after_rollback_and_drop() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    // Transaction reserves the path via add_node, then rolls back.
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("abandoned create")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.add_node(WORKSPACE, &build_node("/reclaimed", "Rolled back"))
        .await?;
    tx.rollback().await?;
    drop(tx);

    // While a second transaction merely gets DROPPED without commit/rollback.
    {
        let tx = storage.begin_context().await?;
        tx.set_tenant_repo(TENANT, REPO)?;
        tx.set_branch(BRANCH)?;
        tx.set_actor("test")?;
        tx.set_message("dropped create")?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_validate_schema(false)?;
        tx.add_node(WORKSPACE, &build_node("/reclaimed", "Dropped"))
            .await?;
        // no commit, no rollback — dropped here
    }

    // The path must be creatable again.
    transactional_create(storage.clone(), build_node("/reclaimed", "Winner")).await?;

    let row_count = count_rows_at_path(&storage, "/reclaimed").await?;
    assert_eq!(row_count, 1);

    Ok(())
}

/// A transactional put with a NEW id racing a repository create at the SAME
/// path must yield exactly one node: put's create branch takes the same path
/// reservation as create.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_put_new_id_vs_create_same_path_yield_single_node() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let put_handle = {
        let storage = storage.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            let node = build_node("/put-vs-create", "Put writer");
            barrier.wait().await;
            transactional_put(storage, node).await
        })
    };
    let create_handle = {
        let storage = storage.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            let node = build_node("/put-vs-create", "Create writer");
            barrier.wait().await;
            storage
                .nodes()
                .create(
                    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                    node,
                    CreateNodeOptions::default(),
                )
                .await
        })
    };

    let mut successes = 0;
    let mut failures = Vec::new();
    for result in [
        put_handle.await.expect("put task panicked"),
        create_handle.await.expect("create task panicked"),
    ] {
        match result {
            Ok(()) => successes += 1,
            Err(e) => failures.push(e),
        }
    }

    assert_eq!(
        successes, 1,
        "exactly one of put-new-id vs create for the same path must succeed \
         (got {} successes, failures: {:?})",
        successes, failures
    );

    let row_count = count_rows_at_path(&storage, "/put-vs-create").await?;
    assert_eq!(
        row_count, 1,
        "exactly one physical row must exist at /put-vs-create after the race"
    );

    Ok(())
}

/// Sequential semantics for put: a put with a NEW id targeting a path that is
/// already occupied by a DIFFERENT id must fail with a conflict, never create
/// a duplicate. (A put updating the EXISTING id in place must keep working.)
#[tokio::test]
async fn sequential_put_at_occupied_path_with_new_id_errors() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    let original = build_node("/occupied", "Original");
    transactional_create(storage.clone(), original.clone()).await?;

    // New id, same path -> Conflict.
    let err = transactional_put(storage.clone(), build_node("/occupied", "Impostor"))
        .await
        .expect_err("put with a new id at an occupied path must error");
    assert!(
        err.to_string().contains("already exists"),
        "error should identify the path conflict, got: {}",
        err
    );

    // Same id, same path -> in-place update must NOT be blocked.
    let mut updated = original.clone();
    updated.properties.insert(
        "title".to_string(),
        PropertyValue::String("Updated".to_string()),
    );
    transactional_put(storage.clone(), updated)
        .await
        .expect("put updating the existing id in place must succeed");

    let row_count = count_rows_at_path(&storage, "/occupied").await?;
    assert_eq!(row_count, 1);

    Ok(())
}

/// Two concurrent deep creates sharing missing parents must CONVERGE on the
/// intermediate folders (exactly one /shared and /shared/a) while both
/// distinct leaves succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_deep_creates_sharing_parents_converge() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let mut handles = Vec::new();
    for leaf in ["leaf1", "leaf2"] {
        let storage = storage.clone();
        let barrier = barrier.clone();
        let path = format!("/shared/a/{}", leaf);
        handles.push(tokio::spawn(async move {
            let node = build_deep_node(&path, leaf);
            barrier.wait().await;
            storage
                .nodes()
                .create_deep_node(
                    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                    &path,
                    node,
                    "raisin:Folder",
                    CreateNodeOptions::default(),
                )
                .await
        }));
    }

    for handle in handles {
        handle
            .await
            .expect("task panicked")
            .expect("both deep creates with distinct leaves must succeed");
    }

    // Intermediate folders converged: exactly one row each.
    assert_eq!(
        count_rows_at_path(&storage, "/shared").await?,
        1,
        "exactly one /shared folder must exist after concurrent deep creates"
    );
    assert_eq!(
        count_rows_under(&storage, "/shared", "/shared/a").await?,
        1,
        "exactly one /shared/a folder must exist after concurrent deep creates"
    );

    // Both leaves exist exactly once.
    assert_eq!(
        count_rows_under(&storage, "/shared/a", "/shared/a/leaf1").await?,
        1
    );
    assert_eq!(
        count_rows_under(&storage, "/shared/a", "/shared/a/leaf2").await?,
        1
    );

    Ok(())
}

/// Two concurrent deep creates for the SAME leaf must conflict on the leaf
/// (exactly one wins) while still converging on the shared parents.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_deep_creates_same_leaf_conflict() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let mut handles = Vec::new();
    for i in 0..2 {
        let storage = storage.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            let node = build_deep_node("/dc/x/leaf", &format!("Writer {}", i));
            barrier.wait().await;
            storage
                .nodes()
                .create_deep_node(
                    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                    "/dc/x/leaf",
                    node,
                    "raisin:Folder",
                    CreateNodeOptions::default(),
                )
                .await
        }));
    }

    let mut successes = 0;
    let mut failures = Vec::new();
    for handle in handles {
        match handle.await.expect("task panicked") {
            Ok(_) => successes += 1,
            Err(e) => failures.push(e),
        }
    }

    assert_eq!(
        successes, 1,
        "exactly one of two racing deep creates for the same leaf must succeed \
         (got {} successes, failures: {:?})",
        successes, failures
    );

    // Parents converged, leaf exists exactly once.
    assert_eq!(count_rows_at_path(&storage, "/dc").await?, 1);
    assert_eq!(count_rows_under(&storage, "/dc", "/dc/x").await?, 1);
    assert_eq!(count_rows_under(&storage, "/dc/x", "/dc/x/leaf").await?, 1);

    Ok(())
}

/// Copying into an already-occupied destination path must fail with a
/// conflict on BOTH the single-node copy and the tree copy — never produce a
/// second row at the destination.
#[tokio::test]
async fn copy_into_occupied_destination_conflicts() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    // Source /a, target parent /b, and an occupant at the destination /b/a.
    transactional_create(storage.clone(), build_node("/a", "Source")).await?;
    transactional_create(storage.clone(), build_node("/b", "Target parent")).await?;
    let mut occupant = build_node("/b/a", "Occupant");
    occupant.name = "a".to_string();
    occupant.parent = Some("/b".to_string());
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            occupant,
            CreateNodeOptions::default(),
        )
        .await?;

    // Single-node copy into the occupied destination must conflict.
    let err = storage
        .nodes()
        .copy_node(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/a",
            "/b",
            None,
            None,
        )
        .await
        .expect_err("copy_node into an occupied destination must error");
    assert!(
        err.to_string().contains("already exists"),
        "error should identify the destination conflict, got: {}",
        err
    );

    // Tree copy into the occupied destination must conflict.
    let err = storage
        .nodes()
        .copy_node_tree(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            "/a",
            "/b",
            None,
            None,
        )
        .await
        .expect_err("copy_node_tree into an occupied destination must error");
    assert!(
        err.to_string().contains("already exists"),
        "error should identify the destination conflict, got: {}",
        err
    );

    // Still exactly one row at the destination.
    assert_eq!(count_rows_under(&storage, "/b", "/b/a").await?, 1);

    Ok(())
}

/// Two concurrent tree copies of the same source into the same destination
/// must yield exactly one copied tree: the destination-root reservation makes
/// the loser conflict instead of writing a duplicate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_tree_copies_same_destination_yield_single_copy() -> Result<()> {
    let (storage, _temp_dir) = setup_storage().await?;

    transactional_create(storage.clone(), build_node("/a", "Source")).await?;
    transactional_create(storage.clone(), build_node("/b", "Target parent")).await?;

    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let mut handles = Vec::new();
    for _ in 0..2 {
        let storage = storage.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            storage
                .nodes()
                .copy_node_tree(
                    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
                    "/a",
                    "/b",
                    None,
                    None,
                )
                .await
        }));
    }

    let mut successes = 0;
    let mut failures = Vec::new();
    for handle in handles {
        match handle.await.expect("task panicked") {
            Ok(_) => successes += 1,
            Err(e) => failures.push(e),
        }
    }

    assert_eq!(
        successes, 1,
        "exactly one of two racing tree copies to the same destination must succeed \
         (got {} successes, failures: {:?})",
        successes, failures
    );

    assert_eq!(
        count_rows_under(&storage, "/b", "/b/a").await?,
        1,
        "exactly one copied row must exist at /b/a after the race"
    );

    Ok(())
}
