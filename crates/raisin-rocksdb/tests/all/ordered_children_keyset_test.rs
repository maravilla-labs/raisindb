// SPDX-License-Identifier: BSL-1.1
//
//! Keyset pagination over the `ORDERED_CHILDREN` editorial order.
//!
//! `ORDERED_CHILDREN` is already a compound index on `(parent_id, order_label)`,
//! so paging is a native seek. These tests pin the properties that make it
//! usable as a cursor:
//!
//!  - forward and reverse paging each cover every child exactly once, in
//!    editorial order;
//!  - a page boundary that lands mid-label does not duplicate or drop the
//!    boundary child;
//!  - reordered children surface at their new label only (the tombstoned old
//!    label never resurfaces);
//!  - paging honours the MVCC `max_revision` bound.

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
                    description: Some("ordered children keyset test".to_string()),
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

/// Parent with `count` children named `child-00`, `child-01`, … in creation
/// (= editorial) order. Returns the parent id.
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

/// Page through the parent with the given page size, collecting names.
/// Uses the public trait surface: the order label of the last row of a page is
/// the cursor for the next.
async fn page_all(
    storage: &RocksDBStorage,
    parent_id: &str,
    page_size: usize,
    descending: bool,
) -> Result<Vec<String>> {
    let nodes = storage.nodes();
    let mut collected = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = nodes
            .list_ordered_children_page(
                scope(),
                parent_id,
                cursor.as_deref(),
                Some(page_size),
                descending,
                None,
            )
            .await?;

        if page.is_empty() {
            break;
        }
        assert!(
            page.len() <= page_size,
            "page returned {} rows for a limit of {page_size}",
            page.len()
        );

        cursor = Some(page.last().expect("non-empty").order_label.clone());
        collected.extend(page.into_iter().map(|c| c.name));

        // Guard against a cursor that fails to advance.
        assert!(
            collected.len() <= 1000,
            "pagination did not terminate; collected {} rows",
            collected.len()
        );
    }

    Ok(collected)
}

#[tokio::test]
async fn forward_paging_covers_every_child_exactly_once() -> Result<()> {
    let t = TestStorage::new().await?;
    let parent_id = seed(&t.storage, 25).await?;

    let expected: Vec<String> = t
        .storage
        .nodes()
        .list_by_parent(scope(), &parent_id, ListOptions::for_api())
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(expected.len(), 25, "seed should produce 25 children");

    // Page sizes that divide evenly, unevenly, and exceed the child count.
    for page_size in [1, 2, 5, 7, 25, 100] {
        let paged = page_all(&t.storage, &parent_id, page_size, false).await?;
        assert_eq!(
            paged, expected,
            "forward paging with page_size={page_size} must reproduce editorial order exactly"
        );
    }

    Ok(())
}

#[tokio::test]
async fn reverse_paging_covers_every_child_exactly_once() -> Result<()> {
    let t = TestStorage::new().await?;
    let parent_id = seed(&t.storage, 25).await?;

    let mut expected: Vec<String> = t
        .storage
        .nodes()
        .list_by_parent(scope(), &parent_id, ListOptions::for_api())
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    expected.reverse();

    for page_size in [1, 3, 7, 25, 100] {
        let paged = page_all(&t.storage, &parent_id, page_size, true).await?;
        assert_eq!(
            paged, expected,
            "reverse paging with page_size={page_size} must reproduce reversed editorial order"
        );
    }

    Ok(())
}

/// A reorder tombstones the old label and writes a new one. Paging must see the
/// child once, at its new position — never at the stale label.
#[tokio::test]
async fn paging_reflects_reorder_without_duplicating() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 6).await?;

    // Move the last child to the front.
    nodes
        .reorder_child(scope(), "/parent", "child-05", 0, Some("move"), Some("t"))
        .await?;

    let paged = page_all(&t.storage, &parent_id, 2, false).await?;
    let expected = vec![
        "child-05".to_string(),
        "child-00".to_string(),
        "child-01".to_string(),
        "child-02".to_string(),
        "child-03".to_string(),
        "child-04".to_string(),
    ];
    assert_eq!(
        paged, expected,
        "reordered child must appear once, at its new position"
    );

    // And the paged view must agree with the unpaginated read.
    let full: Vec<String> = nodes
        .list_by_parent(scope(), &parent_id, ListOptions::for_api())
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(paged, full, "paged and unpaginated reads must agree");

    Ok(())
}

/// The cursor is the label of the last row returned, so resuming from it must
/// yield exactly the rows after it — no repeat of the boundary row.
#[tokio::test]
async fn cursor_is_exclusive_at_the_page_boundary() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 5).await?;

    let first = nodes
        .list_ordered_children_page(scope(), &parent_id, None, Some(2), false, None)
        .await?;
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].name, "child-00");
    assert_eq!(first[1].name, "child-01");

    let second = nodes
        .list_ordered_children_page(
            scope(),
            &parent_id,
            Some(&first[1].order_label),
            Some(2),
            false,
            None,
        )
        .await?;
    assert_eq!(second.len(), 2);
    assert_eq!(
        second[0].name, "child-02",
        "cursor must be exclusive: child-01 must not repeat"
    );
    assert_eq!(second[1].name, "child-03");

    Ok(())
}

/// Paging must respect the MVCC bound: children created after `max_revision`
/// are invisible.
#[tokio::test]
async fn paging_honours_max_revision() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 3).await?;

    let revision_after_three = t.storage.branches().get_head(TENANT, REPO, "main").await?;

    // Two more children at later revisions.
    for i in 3..5 {
        nodes
            .create(
                scope(),
                make_node(&format!("/parent/child-{i:02}")),
                no_validation(),
            )
            .await?;
    }

    let at_head = nodes
        .list_ordered_children_page(scope(), &parent_id, None, None, false, None)
        .await?;
    assert_eq!(at_head.len(), 5, "HEAD should see all five children");

    let historical = nodes
        .list_ordered_children_page(
            scope(),
            &parent_id,
            None,
            None,
            false,
            Some(&revision_after_three),
        )
        .await?;
    let names: Vec<&str> = historical.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["child-00", "child-01", "child-02"],
        "a bounded read must not see children created at later revisions"
    );

    Ok(())
}

/// A limit of zero is a no-op, and an empty parent pages to nothing rather than
/// looping.
#[tokio::test]
async fn degenerate_limits_and_empty_parents() -> Result<()> {
    let t = TestStorage::new().await?;
    let nodes = t.storage.nodes();
    let parent_id = seed(&t.storage, 3).await?;

    let zero = nodes
        .list_ordered_children_page(scope(), &parent_id, None, Some(0), false, None)
        .await?;
    assert!(zero.is_empty(), "limit 0 must return nothing");

    // A childless parent.
    let leaf = make_node("/leaf");
    let leaf_id = leaf.id.clone();
    nodes.create(scope(), leaf, no_validation()).await?;

    let none = nodes
        .list_ordered_children_page(scope(), &leaf_id, None, Some(10), false, None)
        .await?;
    assert!(none.is_empty(), "a childless parent must page to nothing");

    // A cursor past the end yields nothing.
    let all = nodes
        .list_ordered_children_page(scope(), &parent_id, None, None, false, None)
        .await?;
    let past_end = nodes
        .list_ordered_children_page(
            scope(),
            &parent_id,
            Some(&all.last().expect("children exist").order_label),
            Some(10),
            false,
            None,
        )
        .await?;
    assert!(
        past_end.is_empty(),
        "a cursor at the last label must yield nothing"
    );

    Ok(())
}
