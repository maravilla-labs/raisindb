//! Cursor-paginated child listing (`NodeService::list_children_page`).
//!
//! This used to load *every* child and then `skip_while(name != cursor)` — O(N)
//! per page, keyed on the node **name**, so a rename or a duplicate name silently
//! broke pagination. It now seeks on the editorial order index, and the cursor is
//! the order label.
//!
//! These tests pin the resulting contract: pages cover the children exactly once
//! in editorial order, the cursor survives a rename, and a cursor from the old
//! scheme is rejected rather than mis-paginating.

use raisin_context::RepositoryConfig;
use raisin_core::NodeService;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::tree::{PageCursor, PageCursorKind};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const WORKSPACE: &str = "default";

async fn setup() -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
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
                description: Some("child listing keyset test".to_string()),
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, "main", "test-user", None, None, false, false)
        .await?;

    Ok((Arc::new(storage), temp_dir))
}

fn service(storage: Arc<RocksDBStorage>) -> NodeService<RocksDBStorage> {
    // RLS denies everything when no auth context is set (deliberate: callers must
    // opt in explicitly). These tests are about pagination, not permissions.
    NodeService::new_with_context(
        storage,
        TENANT.to_string(),
        REPO.to_string(),
        "main".to_string(),
        WORKSPACE.to_string(),
    )
    .with_auth(raisin_models::auth::AuthContext::system())
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
        id: nanoid::nanoid!(16),
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

async fn seed(storage: &Arc<RocksDBStorage>, count: usize) -> Result<()> {
    let nodes = storage.nodes();
    nodes
        .create(scope(), make_node("/parent"), no_validation())
        .await?;
    for i in 0..count {
        nodes
            .create(
                scope(),
                make_node(&format!("/parent/child-{i:02}")),
                no_validation(),
            )
            .await?;
    }
    Ok(())
}

/// Walk every page and return the names, in order.
async fn page_through(
    svc: &NodeService<RocksDBStorage>,
    parent_path: &str,
    page_size: usize,
) -> Result<Vec<String>> {
    let mut collected = Vec::new();
    let mut cursor: Option<PageCursor> = None;

    loop {
        let page = svc
            .list_children_page(parent_path, cursor.as_ref(), page_size)
            .await?;
        collected.extend(page.items.iter().map(|n| n.name.clone()));

        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
        assert!(
            collected.len() <= 500,
            "pagination did not terminate at page_size={page_size}"
        );
    }
    Ok(collected)
}

#[tokio::test]
async fn pages_cover_children_exactly_once_in_editorial_order() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    seed(&storage, 25).await?;
    let svc = service(storage.clone());

    let expected: Vec<String> = svc
        .list_children("/parent")
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(expected.len(), 25);

    for page_size in [1, 2, 7, 25, 100] {
        let paged = page_through(&svc, "/parent", page_size).await?;
        assert_eq!(
            paged, expected,
            "page_size={page_size} must cover every child exactly once, in editorial order"
        );
    }

    Ok(())
}

/// The cursor is the order label, not the node name — so renaming the node a
/// cursor points at must not break the following page. Under the old
/// name-keyed scheme this silently restarted from the beginning.
#[tokio::test]
async fn cursor_survives_renaming_the_node_it_points_at() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    seed(&storage, 6).await?;
    let svc = service(storage.clone());

    let first = svc.list_children_page("/parent", None, 2).await?;
    assert_eq!(first.items.len(), 2);
    let cursor = first.next_cursor.expect("more pages expected");

    // Rename the last node of page 1 — the one the cursor was derived from.
    storage
        .nodes()
        .rename_node(scope(), "/parent/child-01", "renamed")
        .await?;

    let second = svc.list_children_page("/parent", Some(&cursor), 2).await?;
    let names: Vec<&str> = second.items.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["child-02", "child-03"],
        "a rename must not disturb the cursor; got {names:?}"
    );

    Ok(())
}

/// A cursor minted before cursors were tagged cannot be honoured correctly, so it
/// must fail loudly rather than silently paginating from the wrong place.
#[tokio::test]
async fn legacy_and_mismatched_cursors_are_rejected() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    seed(&storage, 5).await?;
    let svc = service(storage.clone());

    // Untagged (old server) cursor carrying a node name.
    let legacy = PageCursor::new("child-01".to_string(), None);
    let err = svc
        .list_children_page("/parent", Some(&legacy), 2)
        .await
        .expect_err("a legacy cursor must be rejected");
    assert!(
        err.to_string().contains("older version"),
        "error should explain the cursor is stale and pagination must restart, got: {err}"
    );

    // A cursor from a different ordering (revision-snapshot paging).
    let wrong_kind =
        PageCursor::with_kind("some-entry".to_string(), None, PageCursorKind::TreeEntry);
    let err = svc
        .list_children_page("/parent", Some(&wrong_kind), 2)
        .await
        .expect_err("a cursor from another ordering must be rejected");
    assert!(
        err.to_string().contains("different ordering"),
        "error should name the mismatch, got: {err}"
    );

    Ok(())
}

/// The cursor must be tagged, so the next request can be validated.
#[tokio::test]
async fn emitted_cursor_is_tagged_as_an_order_label() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    seed(&storage, 5).await?;
    let svc = service(storage.clone());

    let page = svc.list_children_page("/parent", None, 2).await?;
    let cursor = page.next_cursor.expect("more pages expected");
    assert_eq!(cursor.kind, PageCursorKind::OrderLabel);
    assert!(
        !cursor.last_key.is_empty(),
        "the cursor must carry the order label"
    );

    // And it must round-trip through the wire encoding with its tag intact.
    let encoded = cursor.encode().expect("encode");
    let decoded = PageCursor::decode(&encoded).expect("decode");
    assert_eq!(decoded.kind, PageCursorKind::OrderLabel);
    assert_eq!(decoded.last_key, cursor.last_key);

    Ok(())
}

/// Paging must follow a reorder rather than reporting a stale order.
#[tokio::test]
async fn pages_reflect_a_reorder() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    seed(&storage, 5).await?;
    let svc = service(storage.clone());

    svc.reorder_child("/parent", "child-04", 0, Some("move"), Some("t"))
        .await?;

    let paged = page_through(&svc, "/parent", 2).await?;
    assert_eq!(
        paged,
        vec![
            "child-04".to_string(),
            "child-00".to_string(),
            "child-01".to_string(),
            "child-02".to_string(),
            "child-03".to_string(),
        ],
        "paging must reflect the reorder"
    );

    Ok(())
}

/// Root-level children are addressed under "/" in the ordering index; the same
/// keyset must work there (this path previously had its own scan-and-skip).
#[tokio::test]
async fn root_level_listing_paginates() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let nodes = storage.nodes();
    for i in 0..6 {
        nodes
            .create(
                scope(),
                make_node(&format!("/root-{i:02}")),
                no_validation(),
            )
            .await?;
    }
    let svc = service(storage.clone());

    let paged = page_through(&svc, "/", 2).await?;
    assert_eq!(
        paged.len(),
        6,
        "every root node must appear once: {paged:?}"
    );

    let mut unique = paged.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), paged.len(), "no duplicates across pages");

    Ok(())
}
