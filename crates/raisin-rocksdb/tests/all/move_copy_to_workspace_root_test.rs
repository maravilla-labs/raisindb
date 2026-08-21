// SPDX-License-Identifier: BSL-1.1
//
//! The WORKSPACE ROOT is a valid destination for a move or a copy.
//!
//! `/` addresses a workspace's top level and stores no node of its own, so a
//! "does the target parent exist?" lookup can only ever fail there. Every
//! same-branch move and copy performed that lookup unconditionally, so **no node
//! could be moved or copied to the top level of a workspace** — while
//! cross-branch copy (publishing) had always answered `/` correctly via
//! `resolve_parent_id_opt`, which is why publishing a root-level node worked and
//! moving one did not.
//!
//! It stayed latent until the package installer started adopting nodes whose
//! derived name changed (`properties.name` → path-derived). Adopting
//! `mcp:/Studio` to `mcp:/studio` is a move whose target parent is `/`, so the
//! entry was rejected with `Target parent '/' not found`, and one rejected entry
//! fails the whole package install: two live tenants lost the `studio` package
//! and could not reinstall it.
//!
//! Of the 65 move/rename/copy call sites in this test suite before this file,
//! not one targeted a root — which is exactly how it shipped.

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::StorageScope;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const WORKSPACE: &str = "default";

struct TestStorage {
    storage: RocksDBStorage,
    _temp_dir: TempDir,
}

impl TestStorage {
    async fn new() -> Result<Self> {
        let temp_dir =
            tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
        let storage = RocksDBStorage::new(temp_dir.path())?;

        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;
        storage
            .repository_management()
            .create_repository(
                TENANT,
                REPO,
                RepositoryConfig {
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: "main".to_string(),
                    description: Some("move/copy to workspace root".to_string()),
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(TENANT, REPO, "main", "test-user", None, None, false, false)
            .await?;

        Ok(Self {
            storage,
            _temp_dir: temp_dir,
        })
    }
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, "main", WORKSPACE)
}

fn make_node(path: &str) -> Node {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.rsplit('/').next().unwrap_or(p).to_string()),
        _ => Some("/".to_string()),
    };
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        children: Vec::new(),
        order_key: String::new(),
        has_children: None,
        version: 1,
        archetype: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        created_by: Some("test-user".to_string()),
        updated_by: Some("test-user".to_string()),
        published_at: None,
        published_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

fn no_validation() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

async fn create(storage: &RocksDBStorage, paths: &[&str]) -> Result<()> {
    for path in paths {
        storage
            .nodes()
            .create(scope(), make_node(path), no_validation())
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn move_a_tree_to_the_workspace_root() -> Result<()> {
    let t = TestStorage::new().await?;
    create(
        &t.storage,
        &["/holder", "/holder/movable", "/holder/movable/kid"],
    )
    .await?;
    let nodes = t.storage.nodes();

    let before = nodes
        .get_by_path(scope(), "/holder/movable", None)
        .await?
        .expect("seeded");

    nodes
        .move_node_tree(scope(), &before.id, "/movable", None)
        .await?;

    let moved = nodes
        .get_by_path(scope(), "/movable", None)
        .await?
        .expect("a move to the workspace root must land");
    assert_eq!(
        moved.id, before.id,
        "a move keeps the node's id — that is what makes it a move and not a copy"
    );
    assert!(
        nodes
            .get_by_path(scope(), "/holder/movable", None)
            .await?
            .is_none(),
        "the node must not remain at its old path"
    );
    assert!(
        nodes
            .get_by_path(scope(), "/movable/kid", None)
            .await?
            .is_some(),
        "descendants must be re-pathed under the new root-level path"
    );
    Ok(())
}

/// The exact shape that broke the package installs: a node at the workspace
/// ROOT renamed to a different name at the same level. `rename_node` delegates
/// to the tree move, so its target parent is `/`.
#[tokio::test]
async fn rename_a_node_that_sits_at_the_workspace_root() -> Result<()> {
    let t = TestStorage::new().await?;
    create(&t.storage, &["/Studio", "/Studio/tool"]).await?;
    let nodes = t.storage.nodes();

    let before = nodes
        .get_by_path(scope(), "/Studio", None)
        .await?
        .expect("seeded");

    nodes.rename_node(scope(), "/Studio", "studio").await?;

    let renamed = nodes
        .get_by_path(scope(), "/studio", None)
        .await?
        .expect("renaming a root-level node must land at the new name");
    assert_eq!(
        renamed.id, before.id,
        "adoption relies on the id surviving, so history and references follow"
    );
    assert_eq!(renamed.name, "studio");
    assert!(
        nodes.get_by_path(scope(), "/Studio", None).await?.is_none(),
        "exactly one node must remain — a twin beside the original is the bug this guards"
    );
    assert!(
        nodes
            .get_by_path(scope(), "/studio/tool", None)
            .await?
            .is_some(),
        "children of a renamed root-level node must be re-pathed too"
    );
    Ok(())
}

#[tokio::test]
async fn copy_a_tree_to_the_workspace_root() -> Result<()> {
    let t = TestStorage::new().await?;
    create(
        &t.storage,
        &["/holder", "/holder/source", "/holder/source/kid"],
    )
    .await?;
    let nodes = t.storage.nodes();

    let copied = nodes
        .copy_node_tree(scope(), "/holder/source", "/", Some("copied"), None)
        .await?;

    assert_eq!(
        copied.path, "/copied",
        "the copy lands at the workspace root"
    );
    assert!(
        nodes.get_by_path(scope(), "/copied", None).await?.is_some(),
        "a tree copy to the workspace root must land"
    );
    assert!(
        nodes
            .get_by_path(scope(), "/copied/kid", None)
            .await?
            .is_some(),
        "descendants come with a tree copy"
    );
    assert!(
        nodes
            .get_by_path(scope(), "/holder/source", None)
            .await?
            .is_some(),
        "a copy leaves the source alone"
    );
    Ok(())
}

#[tokio::test]
async fn copy_a_single_node_to_the_workspace_root() -> Result<()> {
    let t = TestStorage::new().await?;
    create(&t.storage, &["/holder", "/holder/leaf"]).await?;
    let nodes = t.storage.nodes();

    let copied = nodes
        .copy_node(scope(), "/holder/leaf", "/", Some("leaf-copy"), None)
        .await?;

    assert_eq!(
        copied.path, "/leaf-copy",
        "joining a root parent naively would yield '//leaf-copy'"
    );
    assert!(
        nodes
            .get_by_path(scope(), "/leaf-copy", None)
            .await?
            .is_some(),
        "a single-node copy to the workspace root must land"
    );
    Ok(())
}

/// The guards that are still meaningful at a root must keep firing — the fix
/// skips only the checks that need a parent NODE.
#[tokio::test]
async fn a_name_already_taken_at_the_root_is_still_refused() -> Result<()> {
    let t = TestStorage::new().await?;
    create(&t.storage, &["/taken", "/holder", "/holder/movable"]).await?;
    let nodes = t.storage.nodes();

    let movable = nodes
        .get_by_path(scope(), "/holder/movable", None)
        .await?
        .expect("seeded");

    let clash = nodes
        .move_node_tree(scope(), &movable.id, "/taken", None)
        .await;

    assert!(
        clash.is_err(),
        "the unique-child-name check must still apply at the workspace root"
    );
    Ok(())
}
