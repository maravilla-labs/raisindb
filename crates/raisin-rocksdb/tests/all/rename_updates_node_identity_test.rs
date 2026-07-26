// SPDX-License-Identifier: BSL-1.1
//
//! A rename must update the node's own identity, not just the paths around it.
//!
//! `rename_node` routes through the tree move, which is deliberately index-only:
//! `Node` stores its parent's NAME rather than a path, so most descendants' blobs
//! stay valid and never need rewriting. But two kinds of node DO go stale, and
//! both used to be missed — leaving `node.name` reporting the old name forever
//! even though every path around it had been updated:
//!
//!   * the renamed node itself (`name`, `parent`, `order_key`);
//!   * its DIRECT children, which hold the old name in `parent`.

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::StorageScope;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, ListOptions, NodeRepository, RegistryRepository,
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
                    description: Some("rename identity test".to_string()),
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

/// `/root/before` with a child and a grandchild.
async fn seed(storage: &RocksDBStorage) -> Result<()> {
    let nodes = storage.nodes();
    for path in [
        "/root",
        "/root/before",
        "/root/before/kid",
        "/root/before/kid/grandkid",
    ] {
        nodes
            .create(scope(), make_node(path), no_validation())
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn rename_updates_the_nodes_own_name() -> Result<()> {
    let t = TestStorage::new().await?;
    seed(&t.storage).await?;
    let nodes = t.storage.nodes();

    nodes.rename_node(scope(), "/root/before", "after").await?;

    let renamed = nodes
        .get_by_path(scope(), "/root/after", None)
        .await?
        .expect("node should exist at its new path");

    assert_eq!(
        renamed.name, "after",
        "the renamed node's own `name` must be updated, not just its path"
    );
    assert_eq!(renamed.path, "/root/after");
    assert_eq!(
        renamed.parent.as_deref(),
        Some("root"),
        "parent name is unchanged by a rename in place"
    );

    Ok(())
}

/// Direct children store the parent's NAME, so a rename must refresh them.
#[tokio::test]
async fn rename_updates_direct_children_parent_name() -> Result<()> {
    let t = TestStorage::new().await?;
    seed(&t.storage).await?;
    let nodes = t.storage.nodes();

    nodes.rename_node(scope(), "/root/before", "after").await?;

    let kid = nodes
        .get_by_path(scope(), "/root/after/kid", None)
        .await?
        .expect("child should exist under the new path");
    assert_eq!(
        kid.parent.as_deref(),
        Some("after"),
        "a direct child must report the renamed parent, not the old name"
    );
    assert_eq!(kid.name, "kid", "the child's own name is unchanged");

    // Deeper descendants are untouched: their parent's name did not change.
    let grandkid = nodes
        .get_by_path(scope(), "/root/after/kid/grandkid", None)
        .await?
        .expect("grandchild should exist under the new path");
    assert_eq!(
        grandkid.parent.as_deref(),
        Some("kid"),
        "a grandchild's parent name is unaffected by renaming its grandparent"
    );

    Ok(())
}

/// The listing surfaces must agree with the stored node, or the UI shows one
/// name while the record holds another.
#[tokio::test]
async fn rename_is_consistent_across_read_surfaces() -> Result<()> {
    let t = TestStorage::new().await?;
    seed(&t.storage).await?;
    let nodes = t.storage.nodes();

    nodes.rename_node(scope(), "/root/before", "after").await?;

    let listed = nodes
        .list_children(scope(), "/root", ListOptions::for_api())
        .await?;
    let names: Vec<&str> = listed.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["after"], "listing must show the new name");

    // The ordering index carries the child name in its entry value too.
    let root = nodes
        .get_by_path(scope(), "/root", None)
        .await?
        .expect("root exists");
    let ordered = nodes
        .list_ordered_children_page(scope(), &root.id, None, None, false, None)
        .await?;
    let ordered_names: Vec<&str> = ordered.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        ordered_names,
        vec!["after"],
        "the ordering index entry must carry the new name"
    );

    // And the node's own order_key must still match its index label.
    let renamed = nodes
        .get_by_path(scope(), "/root/after", None)
        .await?
        .expect("renamed node exists");
    assert_eq!(
        renamed.order_key, ordered[0].order_label,
        "stored order_key must stay in step with the index after a rename"
    );

    Ok(())
}

/// A move to a different parent must update `parent` — and must NOT rename the
/// node, since the leaf name is unchanged.
#[tokio::test]
async fn move_updates_parent_without_renaming() -> Result<()> {
    let t = TestStorage::new().await?;
    seed(&t.storage).await?;
    let nodes = t.storage.nodes();
    nodes
        .create(scope(), make_node("/elsewhere"), no_validation())
        .await?;

    let moving = nodes
        .get_by_path(scope(), "/root/before", None)
        .await?
        .expect("source exists");
    nodes
        .move_node_tree(scope(), &moving.id, "/elsewhere/before", None)
        .await?;

    let moved = nodes
        .get_by_path(scope(), "/elsewhere/before", None)
        .await?
        .expect("node should exist at its new location");
    assert_eq!(moved.name, "before", "a move must not rename the node");
    assert_eq!(
        moved.parent.as_deref(),
        Some("elsewhere"),
        "a move must update the parent name"
    );

    // The child's parent name did not change, so it should be left alone.
    let kid = nodes
        .get_by_path(scope(), "/elsewhere/before/kid", None)
        .await?
        .expect("child moved with its parent");
    assert_eq!(kid.parent.as_deref(), Some("before"));

    Ok(())
}
