//! `has_children` is an existence probe over `ORDERED_CHILDREN` that stops at
//! the first live child. These pin that it answers exactly what the full child
//! listing answers — at HEAD and at every earlier revision — through every
//! surface that populates it:
//!
//! - flips false → true → false on the first create and the last delete;
//! - flips on move-out and move-in;
//! - is revision-bounded (time travel answers as of that revision);
//! - handles the root `/`, whose children are indexed under `"/"`;
//! - skips index entries whose child is gone, and gives up past its cap with
//!   the old, unconfirmed answer;
//! - drives the non-cascade delete guard.

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, RegistryRepository,
    RepoScope, RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use tempfile::TempDir;

const TENANT: &str = "hc-tenant";
const REPO: &str = "hc-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

async fn setup() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
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
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WORKSPACE.to_string()),
        )
        .await?;
    Ok((storage, temp_dir))
}

fn folder(path: &str) -> Node {
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
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
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

async fn create(storage: &RocksDBStorage, path: &str) -> Result<String> {
    let node = folder(path);
    let id = node.id.clone();
    storage
        .nodes()
        .create(scope(), node, no_validation())
        .await?;
    Ok(id)
}

async fn head(storage: &RocksDBStorage) -> Result<HLC> {
    storage.branches().get_head(TENANT, REPO, BRANCH).await
}

/// The probe through every door that answers it, which must all agree.
async fn has_children(storage: &RocksDBStorage, id: &str, rev: Option<&HLC>) -> Result<bool> {
    let nodes = storage.nodes();
    let direct = nodes.has_children(scope(), id, rev).await?;
    let node = nodes
        .get(scope(), id, rev)
        .await?
        .expect("probed node must be readable");
    assert_eq!(
        node.has_children,
        Some(direct),
        "get() must populate has_children with the probe's answer for {}",
        node.path
    );
    let by_path = nodes
        .get_by_path(scope(), &node.path, rev)
        .await?
        .expect("probed node must be readable by path");
    assert_eq!(
        by_path.has_children,
        Some(direct),
        "get_by_path() disagrees"
    );
    Ok(direct)
}

#[tokio::test]
async fn flips_on_first_create_and_last_delete_at_every_revision() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    let r0 = head(&storage).await?;
    assert!(!has_children(&storage, &parent, None).await?);

    let a = create(&storage, "/parent/a").await?;
    let r1 = head(&storage).await?;
    let b = create(&storage, "/parent/b").await?;
    let r2 = head(&storage).await?;
    assert!(has_children(&storage, &parent, None).await?);

    storage
        .nodes()
        .delete(scope(), &a, DeleteNodeOptions::default())
        .await?;
    let r3 = head(&storage).await?;
    assert!(has_children(&storage, &parent, None).await?, "b remains");

    storage
        .nodes()
        .delete(scope(), &b, DeleteNodeOptions::default())
        .await?;
    let r4 = head(&storage).await?;
    assert!(!has_children(&storage, &parent, None).await?);

    for (rev, expected) in [(r0, false), (r1, true), (r2, true), (r3, true), (r4, false)] {
        assert_eq!(
            has_children(&storage, &parent, Some(&rev)).await?,
            expected,
            "has_children as of {rev}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn flips_on_move_out_and_move_in() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    let other = create(&storage, "/other").await?;
    let child = create(&storage, "/parent/c").await?;
    let before_move = head(&storage).await?;
    assert!(has_children(&storage, &parent, None).await?);
    assert!(!has_children(&storage, &other, None).await?);

    storage
        .nodes()
        .move_node(scope(), &child, "/other/c", None)
        .await?;
    let moved_out = head(&storage).await?;
    assert!(!has_children(&storage, &parent, None).await?);
    assert!(has_children(&storage, &other, None).await?);

    storage
        .nodes()
        .move_node(scope(), &child, "/parent/c", None)
        .await?;
    assert!(has_children(&storage, &parent, None).await?);
    assert!(!has_children(&storage, &other, None).await?);

    // And as of each earlier revision.
    assert!(has_children(&storage, &parent, Some(&before_move)).await?);
    assert!(!has_children(&storage, &other, Some(&before_move)).await?);
    assert!(!has_children(&storage, &parent, Some(&moved_out)).await?);
    assert!(has_children(&storage, &other, Some(&moved_out)).await?);
    Ok(())
}

/// Top-level nodes are indexed under `"/"`, not under any root node's id.
#[tokio::test]
async fn root_children_are_indexed_under_slash() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let nodes = storage.nodes();
    assert!(!nodes.has_children(scope(), "/", None).await?);

    let top = create(&storage, "/top").await?;
    assert!(nodes.has_children(scope(), "/", None).await?);
    assert!(!has_children(&storage, &top, None).await?);

    let roots = nodes
        .list_root(scope(), raisin_storage::ListOptions::for_api())
        .await?;
    let listed = roots.iter().find(|n| n.id == top).expect("top is listed");
    assert_eq!(listed.has_children, Some(false));

    create(&storage, "/top/inner").await?;
    let roots = nodes
        .list_root(scope(), raisin_storage::ListOptions::for_api())
        .await?;
    let listed = roots.iter().find(|n| n.id == top).expect("top is listed");
    assert_eq!(listed.has_children, Some(true));
    Ok(())
}

/// Write a LIVE `ORDERED_CHILDREN` entry under `parent_id` naming a child that
/// does not exist — the shape the mis-keyed delete tombstone leaves behind.
fn plant_dead_entry(storage: &RocksDBStorage, parent_id: &str, label: &str, child_id: &str) {
    let key = keys::ordered_child_key_versioned(
        TENANT,
        REPO,
        BRANCH,
        WORKSPACE,
        parent_id,
        label,
        &HLC::new(1, 0),
        child_id,
    );
    let db = storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    db.put_cf(cf, key, child_id.as_bytes()).unwrap();
}

#[tokio::test]
async fn an_entry_whose_child_is_gone_is_not_a_child() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    plant_dead_entry(&storage, &parent, "a0", "ghost-0");
    plant_dead_entry(&storage, &parent, "a1", "ghost-1");
    assert!(!has_children(&storage, &parent, None).await?);

    // A live child after the dead ones is still found.
    create(&storage, "/parent/real").await?;
    assert!(has_children(&storage, &parent, None).await?);
    Ok(())
}

#[tokio::test]
async fn past_the_dead_entry_cap_the_index_answer_stands() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    for i in 0..100 {
        plant_dead_entry(
            &storage,
            &parent,
            &format!("a{i:03}"),
            &format!("ghost-{i}"),
        );
    }
    // Unconfirmed past the cap: exactly the pre-probe answer.
    assert!(has_children(&storage, &parent, None).await?);
    Ok(())
}

#[tokio::test]
async fn non_cascade_delete_is_guarded_by_the_probe() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    let child = create(&storage, "/parent/c").await?;
    let guarded = DeleteNodeOptions {
        cascade: false,
        check_has_children: true,
        operation_meta: None,
    };

    let refused = storage
        .nodes()
        .delete(scope(), &parent, guarded.clone())
        .await;
    assert!(refused.is_err(), "a parent with a child must not delete");

    storage
        .nodes()
        .delete(scope(), &child, DeleteNodeOptions::default())
        .await?;
    assert!(storage.nodes().delete(scope(), &parent, guarded).await?);
    Ok(())
}

/// The only child moves away, and an `ORDERED_CHILDREN` entry for it is left
/// live under the old parent (a move that failed to tombstone it). The child
/// is live — just elsewhere — so the probe must confirm placement, not only
/// liveness.
#[tokio::test]
async fn a_child_moved_away_behind_a_stale_entry_is_not_a_child() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    let other = create(&storage, "/other").await?;
    let child = create(&storage, "/parent/c").await?;
    let before_move = head(&storage).await?;

    storage
        .nodes()
        .move_node(scope(), &child, "/other/c", None)
        .await?;
    plant_dead_entry(&storage, &parent, "zz", &child);

    assert!(
        !has_children(&storage, &parent, None).await?,
        "the old parent has no child at HEAD"
    );
    assert!(
        has_children(&storage, &parent, Some(&before_move)).await?,
        "before the move, the child was there"
    );
    assert!(has_children(&storage, &other, None).await?);
    Ok(())
}
