// SPDX-License-Identifier: BSL-1.1
//
//! Subtree document order and its resumable depth-first cursor.
//!
//! `scan_descendants_ordered_page` walks a subtree pre-order depth-first and
//! hands back each node's `tree_order`. These tests pin the two properties that
//! make that one opaque string usable as both a sort key and a page cursor:
//!
//!  - byte-wise sorting the paths reproduces the traversal exactly;
//!  - resuming from a path continues where the previous page stopped, covering
//!    every node exactly once.

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
                    description: Some("subtree document order test".to_string()),
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

/// A three-level tree, created so that creation order IS editorial order:
///
/// ```text
/// /root
///   a
///     a1
///       a1x
///     a2
///   b
///     b1
///   c
/// ```
async fn seed_tree(storage: &RocksDBStorage) -> Result<String> {
    let nodes = storage.nodes();
    let root = make_node("/root");
    let root_id = root.id.clone();
    nodes.create(scope(), root, no_validation()).await?;

    for path in [
        "/root/a",
        "/root/a/a1",
        "/root/a/a1/a1x",
        "/root/a/a2",
        "/root/b",
        "/root/b/b1",
        "/root/c",
    ] {
        nodes
            .create(scope(), make_node(path), no_validation())
            .await?;
    }
    Ok(root_id)
}

/// Expected pre-order depth-first walk, including the traversal root.
fn expected_document_order() -> Vec<&'static str> {
    vec!["root", "a", "a1", "a1x", "a2", "b", "b1", "c"]
}

#[tokio::test]
async fn traversal_is_preorder_depth_first() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;

    let walked = t
        .storage
        .nodes()
        .scan_descendants_ordered_page(scope(), &root_id, None, None, ListOptions::for_api())
        .await?;

    let names: Vec<&str> = walked.iter().map(|(n, _)| n.name.as_str()).collect();
    assert_eq!(
        names,
        expected_document_order(),
        "traversal must be pre-order depth-first (parent, then its subtree, then next sibling)"
    );

    Ok(())
}

/// The property that makes `__tree_order` a valid ORDER BY key.
#[tokio::test]
async fn sorting_tree_orders_reproduces_the_traversal() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;

    let walked = t
        .storage
        .nodes()
        .scan_descendants_ordered_page(scope(), &root_id, None, None, ListOptions::for_api())
        .await?;

    let mut by_path: Vec<(String, String)> = walked
        .iter()
        .map(|(node, path)| (path.clone(), node.name.clone()))
        .collect();
    by_path.sort();

    let sorted_names: Vec<String> = by_path.into_iter().map(|(_, name)| name).collect();
    let walked_names: Vec<String> = walked.iter().map(|(n, _)| n.name.clone()).collect();

    assert_eq!(
        sorted_names, walked_names,
        "byte-wise sorting tree_order must reproduce the traversal order"
    );

    // The traversal root anchors the order with an empty path.
    assert_eq!(
        walked[0].1, "",
        "the traversal root's tree_order should be empty"
    );
    assert!(
        walked[1..].iter().all(|(_, path)| !path.is_empty()),
        "every descendant must have a non-empty tree_order"
    );

    Ok(())
}

/// Deeper nodes must carry their ancestors' labels as a prefix — that is what
/// keeps a subtree contiguous under sorting.
#[tokio::test]
async fn tree_orders_nest_by_prefix() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;

    let walked = t
        .storage
        .nodes()
        .scan_descendants_ordered_page(scope(), &root_id, None, None, ListOptions::for_api())
        .await?;

    let path_of = |name: &str| -> String {
        walked
            .iter()
            .find(|(node, _)| node.name == name)
            .map(|(_, path)| path.clone())
            .unwrap_or_else(|| panic!("{name} should be in the walk"))
    };

    let a = path_of("a");
    let a1 = path_of("a1");
    let a1x = path_of("a1x");
    let b = path_of("b");

    assert!(a1.starts_with(&a), "a1's path must extend a's: {a1} vs {a}");
    assert!(a1x.starts_with(&a1), "a1x's path must extend a1's");
    assert!(a1x < b, "the whole 'a' subtree must precede sibling 'b'");
    assert!(
        !b.starts_with(&a),
        "a sibling's path must not extend its sibling's"
    );

    Ok(())
}

/// Paging with a cursor must cover the subtree exactly once.
#[tokio::test]
async fn resumable_cursor_covers_the_subtree_exactly_once() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;
    let nodes = t.storage.nodes();

    for page_size in [1, 2, 3, 5, 8, 50] {
        let mut collected: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let page = nodes
                .scan_descendants_ordered_page(
                    scope(),
                    &root_id,
                    cursor.as_deref(),
                    Some(page_size),
                    ListOptions::for_api(),
                )
                .await?;
            if page.is_empty() {
                break;
            }
            assert!(
                page.len() <= page_size,
                "page returned {} rows for limit {page_size}",
                page.len()
            );

            cursor = Some(page.last().expect("non-empty").1.clone());
            collected.extend(page.into_iter().map(|(node, _)| node.name));

            assert!(
                collected.len() <= 100,
                "pagination failed to terminate at page_size={page_size}"
            );
        }

        assert_eq!(
            collected,
            expected_document_order(),
            "page_size={page_size} must reproduce document order exactly once"
        );
    }

    Ok(())
}

/// Resuming mid-subtree must continue *inside* that subtree before moving on to
/// the next sibling — the case a naive "skip N" cursor gets wrong.
#[tokio::test]
async fn resuming_mid_subtree_continues_depth_first() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;
    let nodes = t.storage.nodes();

    let all = nodes
        .scan_descendants_ordered_page(scope(), &root_id, None, None, ListOptions::for_api())
        .await?;

    // Resume right after "a1" — the next node must be its child "a1x", not "a2".
    let a1_path = all
        .iter()
        .find(|(node, _)| node.name == "a1")
        .map(|(_, path)| path.clone())
        .expect("a1 present");

    let rest = nodes
        .scan_descendants_ordered_page(
            scope(),
            &root_id,
            Some(&a1_path),
            None,
            ListOptions::for_api(),
        )
        .await?;
    let names: Vec<&str> = rest.iter().map(|(n, _)| n.name.as_str()).collect();

    assert_eq!(
        names,
        vec!["a1x", "a2", "b", "b1", "c"],
        "resuming after 'a1' must descend into 'a1x' before advancing to 'a2'"
    );

    Ok(())
}

/// A cursor whose node has since been deleted must not lose the rest of the
/// walk, and must not loop.
#[tokio::test]
async fn cursor_survives_deletion_of_its_own_node() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;
    let nodes = t.storage.nodes();

    let all = nodes
        .scan_descendants_ordered_page(scope(), &root_id, None, None, ListOptions::for_api())
        .await?;
    let (a2_node, a2_path) = all
        .iter()
        .find(|(node, _)| node.name == "a2")
        .cloned()
        .expect("a2 present");

    nodes
        .delete(scope(), &a2_node.id, Default::default())
        .await?;

    let rest = nodes
        .scan_descendants_ordered_page(
            scope(),
            &root_id,
            Some(&a2_path),
            None,
            ListOptions::for_api(),
        )
        .await?;
    let names: Vec<&str> = rest.iter().map(|(n, _)| n.name.as_str()).collect();

    assert_eq!(
        names,
        vec!["b", "b1", "c"],
        "a cursor pointing at a deleted node must still yield the remaining walk"
    );

    Ok(())
}

/// The unpaginated wrapper must agree with the paged walk, so existing callers
/// (subtree copy / move / prune) see the same set and parent-before-child order.
#[tokio::test]
async fn unpaginated_scan_agrees_and_keeps_parents_before_children() -> Result<()> {
    let t = TestStorage::new().await?;
    let root_id = seed_tree(&t.storage).await?;
    let nodes = t.storage.nodes();

    let flat = nodes
        .scan_descendants_ordered(scope(), &root_id, ListOptions::for_api())
        .await?;
    let paged = nodes
        .scan_descendants_ordered_page(scope(), &root_id, None, None, ListOptions::for_api())
        .await?;

    let flat_names: Vec<String> = flat.iter().map(|n| n.name.clone()).collect();
    let paged_names: Vec<String> = paged.iter().map(|(n, _)| n.name.clone()).collect();
    assert_eq!(flat_names, paged_names, "both scans must agree");

    // Parent-before-child is what the copy/move/prune callers rely on.
    let position = |name: &str| flat_names.iter().position(|n| n == name).unwrap();
    for (parent, child) in [("root", "a"), ("a", "a1"), ("a1", "a1x"), ("b", "b1")] {
        assert!(
            position(parent) < position(child),
            "{parent} must be emitted before {child}"
        );
    }

    Ok(())
}
