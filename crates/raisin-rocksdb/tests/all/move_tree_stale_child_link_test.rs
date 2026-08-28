//! A child listed under a parent it no longer lives under must not be dragged
//! along — and above all must not be given a nonsense path.
//!
//! `move_node_tree` finds a subtree by walking ORDERED_CHILDREN, then rewrites
//! each node's path by swapping the root's old prefix for its new one. Those are
//! two different indexes, and they can disagree: a child's ordering entry under
//! its previous parent survives when the tombstone that should remove it is
//! skipped (the parent is resolved BY PATH, and that lookup can miss while the
//! path index is still catching up with an earlier move).
//!
//! The prefix swap used to be `strip_prefix(..).unwrap_or(&node.path)`, so a node
//! that was not really in the subtree had its own ABSOLUTE path appended to the
//! new root path:
//!
//! ```text
//! /site/moved/page//site/elsewhere/orphan
//! ```
//!
//! It still answered by id, but no parent listed it and no scan found it, and the
//! next subtree delete above it took it for good. Measured in Maravilla Studio:
//! converting a page to a variation set and then reverting it silently destroyed
//! the page's subpages.
//!
//! What this asserts is the property that matters: a MOVE never invents a path
//! for a node outside the subtree, and never loses it.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{fractional_index, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeRepository, RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;

const TENANT: &str = "stalechild-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "default";

async fn setup() -> (Arc<RocksDBStorage>, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());

    storage
        .repository_management()
        .create_repository(TENANT, REPO, RepositoryConfig::default())
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await
        .unwrap();

    let folder = NodeType {
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
            folder,
            CommitMetadata::system("seed folder type"),
        )
        .await
        .unwrap();

    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await
        .unwrap();

    (storage, dir)
}

fn node(path: &str) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.clone()));
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        ..Default::default()
    }
}

/// Seed a set of paths, one committed transaction, returning their ids in order.
async fn seed(storage: &Arc<RocksDBStorage>, paths: &[&str]) -> Vec<String> {
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_message("seed").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    let mut ids = Vec::new();
    for path in paths {
        let n = node(path);
        ids.push(n.id.clone());
        tx.upsert_deep_node(WS, &n, "raisin:Folder").await.unwrap();
    }
    tx.commit().await.unwrap();
    ids
}

async fn move_to(storage: &Arc<RocksDBStorage>, id: &str, new_path: &str) {
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_message("move").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.move_node_tree(WS, id, new_path).await.unwrap();
    tx.commit().await.unwrap();
}

async fn path_of(storage: &Arc<RocksDBStorage>, id: &str) -> Option<String> {
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.get_node(WS, id).await.unwrap().map(|n| n.path)
}

/// The shape the Studio page-variation conversion produces: a page's child is
/// re-homed onto a sibling container, and the page is then moved again.
///
/// Whatever the child-order index believes, the child's path is the authority on
/// whether it is in the subtree — so after moving the page, the child must still
/// sit exactly where it was put, with a path that is a real address.
#[tokio::test]
async fn a_child_moved_out_is_not_dragged_along_by_its_old_parent() {
    let (storage, _dir) = setup().await;

    let ids = seed(&storage, &["/box", "/container", "/page", "/page/sub"]).await;
    let (page_id, sub_id) = (ids[2].clone(), ids[3].clone());

    // The child is re-homed onto the container — as the conversion moves a
    // page's subpages onto the variation container.
    move_to(&storage, &sub_id, "/container/sub").await;
    assert_eq!(
        path_of(&storage, &sub_id).await.as_deref(),
        Some("/container/sub"),
        "precondition: the child is re-homed"
    );

    // Now the old parent moves. Anything still linking the child to it must not
    // change where the child lives.
    move_to(&storage, &page_id, "/box/page").await;

    assert_eq!(
        path_of(&storage, &page_id).await.as_deref(),
        Some("/box/page"),
        "the page itself moved"
    );
    let sub_path = path_of(&storage, &sub_id).await;
    assert_eq!(
        sub_path.as_deref(),
        Some("/container/sub"),
        "the re-homed child must be untouched by its old parent's move"
    );

    // The strongest statement, and the one the old code broke: whatever path the
    // child has, it must be a real address — reachable by path, not a splice.
    let by_path = {
        let tx = storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch(BRANCH).unwrap();
        tx.set_auth_context(AuthContext::system()).unwrap();
        tx.get_node_by_path(WS, sub_path.as_deref().unwrap())
            .await
            .unwrap()
    };
    assert_eq!(
        by_path.map(|n| n.id),
        Some(sub_id),
        "the child must resolve at its own path — a spliced path resolves to nothing"
    );
}

/// The reproduction, in the shape that actually creates the stale link: the
/// parent is moved and the child re-homed IN ONE TRANSACTION.
///
/// The child's move resolves its old parent BY PATH to decide which
/// ORDERED_CHILDREN entry to tombstone. Inside the transaction that just moved
/// that parent, the path it looks up is the one the child still records — which
/// no longer resolves — so no tombstone is written and the child stays listed
/// under a parent it has left. Moving the parent again then walks into it.
#[tokio::test]
async fn a_child_rehomed_in_the_same_transaction_as_its_parent_is_not_corrupted() {
    let (storage, _dir) = setup().await;

    let ids = seed(&storage, &["/box", "/container", "/page", "/page/sub"]).await;
    let (page_id, sub_id) = (ids[2].clone(), ids[3].clone());

    // ONE transaction: the page moves, then its child is re-homed onto the
    // container — the exact sequence `convert-to-page-variations` performs.
    {
        let tx = storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch(BRANCH).unwrap();
        tx.set_actor("test").unwrap();
        tx.set_message("convert").unwrap();
        tx.set_auth_context(AuthContext::system()).unwrap();
        tx.move_node_tree(WS, &page_id, "/box/page").await.unwrap();
        tx.move_node_tree(WS, &sub_id, "/container/sub")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    assert_eq!(
        path_of(&storage, &sub_id).await.as_deref(),
        Some("/container/sub"),
        "precondition: the child is re-homed"
    );

    // Now move the old parent again. If it still lists the child, the child must
    // NOT be relocated — and above all must not be given a spliced path.
    move_to(&storage, &page_id, "/page-again").await;

    let sub_path = path_of(&storage, &sub_id).await;
    assert_eq!(
        sub_path.as_deref(),
        Some("/container/sub"),
        "the re-homed child must survive its old parent's move untouched"
    );

    let by_path = {
        let tx = storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch(BRANCH).unwrap();
        tx.set_auth_context(AuthContext::system()).unwrap();
        tx.get_node_by_path(WS, sub_path.as_deref().unwrap())
            .await
            .unwrap()
    };
    assert_eq!(
        by_path.map(|n| n.id),
        Some(sub_id.clone()),
        "the child must still resolve at its own path"
    );

    // And the stale link is HEALED, so a second move of the parent no longer even
    // reaches the child.
    move_to(&storage, &page_id, "/box/page").await;
    assert_eq!(
        path_of(&storage, &sub_id).await.as_deref(),
        Some("/container/sub"),
        "and again, now that the stale ordering entry is gone"
    );
}

/// The same guarantee for a move that only RENAMES (same parent, new name),
/// which takes the identical path-rewrite branch.
#[tokio::test]
async fn a_rename_does_not_invent_paths_for_outsiders() {
    let (storage, _dir) = setup().await;

    let ids = seed(&storage, &["/parent", "/parent/kid", "/elsewhere"]).await;
    let (parent_id, kid_id, elsewhere_id) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

    move_to(&storage, &kid_id, "/elsewhere/kid").await;
    move_to(&storage, &parent_id, "/parent-renamed").await;

    assert_eq!(
        path_of(&storage, &kid_id).await.as_deref(),
        Some("/elsewhere/kid"),
        "a child that left before the rename stays where it went"
    );
    assert_eq!(
        path_of(&storage, &elsewhere_id).await.as_deref(),
        Some("/elsewhere"),
        "and its new parent is undisturbed"
    );
}
