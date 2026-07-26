// SPDX-License-Identifier: BSL-1.1
//
//! `Node.order_key` must agree with the `ORDERED_CHILDREN` index label.
//!
//! `order_key` is what every read surface reports — the node JSON, the `__order`
//! SQL column, and replication's fallback order key. It used to be written only
//! on create: a reorder updated `ORDERED_CHILDREN` and left the node record
//! holding a stale label. These tests pin that the two stay in sync across every
//! operation that can change a child's position.

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
                    description: Some("order_key persistence test".to_string()),
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
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
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

async fn seed(storage: &RocksDBStorage, count: usize) -> Result<String> {
    let nodes = storage.nodes();
    let parent = make_node("/parent");
    let parent_id = parent.id.clone();
    nodes.create(scope(), parent, no_validation()).await?;

    for i in 0..count {
        nodes
            .create(
                scope(),
                make_node(&format!("/parent/child-{i:02}")),
                no_validation(),
            )
            .await?;
    }
    Ok(parent_id)
}

/// Every child's stored `order_key` must equal its index label, and sorting the
/// children by `order_key` must reproduce the editorial order.
async fn assert_order_keys_match_index(
    storage: &RocksDBStorage,
    parent_id: &str,
    context: &str,
) -> Result<()> {
    let nodes = storage.nodes();

    let entries = nodes
        .list_ordered_children_page(scope(), parent_id, None, None, false, None)
        .await?;

    for entry in &entries {
        let node = nodes
            .get(scope(), &entry.child_id, None)
            .await?
            .unwrap_or_else(|| panic!("{context}: child {} should exist", entry.child_id));

        assert_eq!(
            node.order_key, entry.order_label,
            "{context}: node '{}' stored order_key must equal its ORDERED_CHILDREN label",
            node.name
        );
        assert!(
            !node.order_key.is_empty(),
            "{context}: node '{}' must have a non-empty order_key",
            node.name
        );
    }

    // Sorting by the stored key alone must reproduce editorial order — that is
    // the property the `__order` SQL column depends on.
    let index_order: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    let mut by_stored_key: Vec<(String, String)> = Vec::new();
    for entry in &entries {
        let node = nodes.get(scope(), &entry.child_id, None).await?.unwrap();
        by_stored_key.push((node.order_key, node.name));
    }
    by_stored_key.sort();
    let stored_order: Vec<String> = by_stored_key.into_iter().map(|(_, name)| name).collect();

    assert_eq!(
        stored_order, index_order,
        "{context}: sorting children by stored order_key must reproduce editorial order"
    );

    Ok(())
}

#[tokio::test]
async fn create_populates_order_key() -> Result<()> {
    let t = TestStorage::new().await?;
    let parent_id = seed(&t.storage, 5).await?;
    assert_order_keys_match_index(&t.storage, &parent_id, "after create").await
}

#[tokio::test]
async fn reorder_child_refreshes_stored_order_key() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 5).await?;

    let before = nodes
        .get_by_path(scope(), "/parent/child-04", None)
        .await?
        .expect("child-04 exists")
        .order_key;

    nodes
        .reorder_child(scope(), "/parent", "child-04", 0, Some("move"), Some("t"))
        .await?;

    let after = nodes
        .get_by_path(scope(), "/parent/child-04", None)
        .await?
        .expect("child-04 exists")
        .order_key;

    assert_ne!(
        before, after,
        "reorder must refresh the stored order_key (regression: it went stale)"
    );
    assert_order_keys_match_index(&t.storage, &parent_id, "after reorder_child").await
}

#[tokio::test]
async fn move_child_before_and_after_refresh_stored_order_key() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 5).await?;

    nodes
        .move_child_before(
            scope(),
            "/parent",
            "child-03",
            "child-01",
            Some("before"),
            Some("t"),
        )
        .await?;
    assert_order_keys_match_index(&t.storage, &parent_id, "after move_child_before").await?;

    nodes
        .move_child_after(
            scope(),
            "/parent",
            "child-00",
            "child-04",
            Some("after"),
            Some("t"),
        )
        .await?;
    assert_order_keys_match_index(&t.storage, &parent_id, "after move_child_after").await?;

    Ok(())
}

/// Reorder writes the node record, so the child gains a revision and the
/// reorder becomes visible in node history.
#[tokio::test]
async fn reorder_appears_in_node_history() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    seed(&t.storage, 3).await?;

    let child = nodes
        .get_by_path(scope(), "/parent/child-02", None)
        .await?
        .expect("child-02 exists");

    let before = nodes
        .get_node_history(scope(), &child.id, None)
        .await?
        .len();

    nodes
        .reorder_child(scope(), "/parent", "child-02", 0, Some("move"), Some("t"))
        .await?;

    let after = nodes.get_node_history(scope(), &child.id, None).await?;
    assert!(
        after.len() > before,
        "reorder should add a revision to the child's history (was {before}, now {})",
        after.len()
    );
    assert!(
        !after[0].deleted,
        "the newest history entry must not be a tombstone"
    );

    Ok(())
}

/// A reorder must not leave a second live index entry behind: the node's
/// `order_key` write goes through the ordering path, which preserves the
/// already-written label rather than appending a new one.
#[tokio::test]
async fn reorder_does_not_duplicate_index_entries() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 4).await?;

    for _ in 0..3 {
        nodes
            .reorder_child(scope(), "/parent", "child-03", 0, Some("move"), Some("t"))
            .await?;
        nodes
            .reorder_child(scope(), "/parent", "child-03", 3, Some("move"), Some("t"))
            .await?;
    }

    let entries = nodes
        .list_ordered_children_page(scope(), &parent_id, None, None, false, None)
        .await?;
    assert_eq!(
        entries.len(),
        4,
        "repeated reorders must not multiply index entries: {:?}",
        entries.iter().map(|e| &e.name).collect::<Vec<_>>()
    );

    // And the plain listing must agree.
    let listed = nodes
        .list_by_parent(scope(), &parent_id, ListOptions::for_api())
        .await?;
    assert_eq!(listed.len(), 4, "list_by_parent must also see exactly 4");

    assert_order_keys_match_index(&t.storage, &parent_id, "after repeated reorders").await
}
