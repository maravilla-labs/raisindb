//! Keyed user homes on the write path (`raisin_models::auth::user_key`).
//!
//! A `raisin:User` created directly under `/users/internal` in
//! `raisin:access_control` is named by a minted key by every low-level writer
//! (transaction add/put, repository create), except inside a content import
//! (package install, restore), which keeps the path it carries. A home, and
//! the folders that hold every home, never move or rename outside an import.

use crate::node_path_writer_test::{setup, BRANCH, REPO, TENANT};
use raisin_error::Result;
use raisin_models::auth::user_key::{is_user_key, user_key_from_home};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    CreateNodeOptions, NodeRepository, NodeTypeRepository, RepoScope, Storage, StorageScope,
    WorkspaceRepository,
};

const AC: &str = "raisin:access_control";

fn ac() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, AC)
}

async fn setup_ac() -> Result<(RocksDBStorage, tempfile::TempDir)> {
    let (storage, dir) = setup().await?;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(AC.to_string()),
        )
        .await?;
    for name in ["raisin:Folder", "raisin:User"] {
        let mut nt: raisin_models::nodes::NodeType =
            serde_json::from_value(serde_json::json!({ "name": name })).expect("node type");
        nt.id = Some(name.to_string());
        nt.strict = Some(false);
        nt.allowed_children = vec!["*".to_string()];
        storage
            .node_types()
            .upsert(
                raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
                nt,
                raisin_storage::CommitMetadata::system("seed"),
            )
            .await?;
    }
    for (id, path) in [("f-users", "/users"), ("f-internal", "/users/internal")] {
        tx_write(&storage, &node(id, path, "raisin:Folder"), false, false).await?;
    }
    Ok((storage, dir))
}

fn node(id: &str, path: &str, node_type: &str) -> Node {
    Node {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap().to_string(),
        node_type: node_type.to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

async fn tx_write(storage: &RocksDBStorage, n: &Node, put: bool, import: bool) -> Result<()> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("test")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.set_content_import(import)?;
    if put {
        tx.put_node(AC, n).await?;
    } else {
        tx.add_node(AC, n).await?;
    }
    tx.commit().await
}

async fn path_of(storage: &RocksDBStorage, id: &str) -> Option<String> {
    storage
        .nodes()
        .get(ac(), id, None)
        .await
        .unwrap()
        .map(|n| n.path)
}

#[tokio::test]
async fn every_writer_names_a_new_home_by_a_key() -> Result<()> {
    let (storage, _dir) = setup_ac().await?;
    tx_write(
        &storage,
        &node("u1", "/users/internal/alice", "raisin:User"),
        false,
        false,
    )
    .await?;
    tx_write(
        &storage,
        &node("u2", "/users/internal/bob", "raisin:User"),
        true,
        false,
    )
    .await?;
    storage
        .nodes()
        .create(
            ac(),
            node("u3", "/users/internal/carol", "raisin:User"),
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await?;
    for id in ["u1", "u2", "u3"] {
        let path = path_of(&storage, id).await.expect("created");
        let key = user_key_from_home(&path).unwrap_or_else(|| panic!("{id} at {path}"));
        assert!(is_user_key(key));
        assert!(path.starts_with("/users/internal/u_"), "{path}");
    }
    // Elsewhere, and other node types, keep their names.
    tx_write(
        &storage,
        &node("sys", "/users/anon", "raisin:User"),
        false,
        false,
    )
    .await?;
    assert_eq!(
        path_of(&storage, "sys").await.as_deref(),
        Some("/users/anon")
    );
    tx_write(
        &storage,
        &node("fo", "/users/internal/shared", "raisin:Folder"),
        false,
        false,
    )
    .await?;
    assert_eq!(
        path_of(&storage, "fo").await.as_deref(),
        Some("/users/internal/shared")
    );
    Ok(())
}

#[tokio::test]
async fn an_import_keeps_the_path_it_carries() -> Result<()> {
    let (storage, _dir) = setup_ac().await?;
    let legacy = node("u1", "/users/internal/alice-at-x-com", "raisin:User");
    tx_write(&storage, &legacy, true, true).await?;
    assert_eq!(
        path_of(&storage, "u1").await.as_deref(),
        Some("/users/internal/alice-at-x-com")
    );
    // And an update of it (no import) that keeps the path is fine.
    tx_write(&storage, &legacy, true, false).await?;
    Ok(())
}

#[tokio::test]
async fn homes_do_not_move_or_rename() -> Result<()> {
    let (storage, _dir) = setup_ac().await?;
    tx_write(
        &storage,
        &node("u1", "/users/internal/alice", "raisin:User"),
        false,
        false,
    )
    .await?;
    let path = path_of(&storage, "u1").await.unwrap();

    // Repository move and rename.
    let err = storage
        .nodes()
        .move_node(ac(), "u1", "/users/internal/elsewhere", None)
        .await
        .expect_err("a home does not move");
    assert!(err.to_string().contains("stable key"), "{err}");
    assert!(storage
        .nodes()
        .rename_node(ac(), &path, "renamed")
        .await
        .is_err());
    assert!(storage
        .nodes()
        .move_node(ac(), "f-internal", "/people", None)
        .await
        .is_err());

    // Transaction move, and a put that changes the path.
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_auth_context(AuthContext::system())?;
    assert!(tx
        .move_node_tree(AC, "u1", "/users/internal/elsewhere")
        .await
        .is_err());
    tx.rollback().await?;
    let mut moved = node("u1", "/users/internal/elsewhere", "raisin:User");
    moved.name = "elsewhere".into();
    assert!(tx_write(&storage, &moved, true, false).await.is_err());

    assert_eq!(path_of(&storage, "u1").await, Some(path));
    Ok(())
}
