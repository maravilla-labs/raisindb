//! One test per row of the maintenance matrix: copies, publish, prune,
//! delete, restore, `versionable=false`.

use super::support::*;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_storage::localized::LocalizedServedBy::Index;
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use raisin_storage::{
    BranchRepository, CommitMetadata, DeleteNodeOptions, NodeRepository, NodeTypeRepository,
};

async fn publish(storage: &raisin_rocksdb::RocksDBStorage, roots: &[&str], delete_missing: bool) {
    let roots: Vec<String> = roots.iter().map(|p| p.to_string()).collect();
    storage
        .nodes()
        .copy_nodes_across_branches(
            T,
            R,
            B,
            "publish",
            WS,
            &roots,
            true,
            delete_missing,
            None,
            None,
        )
        .await
        .unwrap();
}

async fn publish_branch(storage: &raisin_rocksdb::RocksDBStorage) {
    storage
        .branches()
        .create_branch(T, R, "publish", "test", None, None, false, false)
        .await
        .unwrap();
    build(storage, "publish").await;
}

#[tokio::test]
async fn carried_by_copy_cross_branch_copy_and_fork() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (products, chair) = catalog(&storage).await;

    // Tree copy: a fresh id under another parent, its overlays carried.
    let archive = create(&storage, "/archive", &[]).await;
    set_name(&storage, &archive, "fr", "archives").await;
    let copy = storage
        .nodes()
        .copy_node_tree(scope(B), "/products/chair", "/archive", None, None)
        .await
        .unwrap();
    assert_ne!(copy.id, chair);
    assert_eq!(
        id_via(&storage, B, "fr", "/archives/chaise", Index),
        Some(copy.id)
    );
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/chaise", Index),
        Some(chair.clone())
    );

    // Cross-branch copy (publish): same ids, overlays carried.
    publish_branch(&storage).await;
    publish(&storage, &["/products"], false).await;
    assert_eq!(
        id_via(&storage, "publish", "fr", "/produits/chaise", Index),
        Some(chair.clone())
    );
    assert_eq!(
        claimants(&storage, "publish", "fr", &products, "chaise"),
        vec![chair.clone()]
    );

    // Fork: the claims travel with the branch copy.
    storage
        .branches()
        .create_branch(
            T,
            R,
            "fork",
            "test",
            None,
            Some(B.to_string()),
            false,
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        claimants(&storage, "fork", "fr", &products, "chaise"),
        vec![chair]
    );
}

#[tokio::test]
async fn pruned_by_cross_branch_prune() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (products, chair) = catalog(&storage).await;
    publish_branch(&storage).await;
    publish(&storage, &["/products"], false).await;
    assert_eq!(
        id_via(&storage, "publish", "fr", "/produits/chaise", Index),
        Some(chair.clone())
    );

    storage
        .nodes()
        .delete(scope(B), &chair, DeleteNodeOptions::default())
        .await
        .unwrap();
    publish(&storage, &["/products"], true).await;
    assert_eq!(
        resolve_at(&storage, "publish", "fr", "/produits/chaise", None),
        None
    );
    assert!(claimants(&storage, "publish", "fr", &products, "chaise").is_empty());
}

#[tokio::test]
async fn removed_on_delete() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    let (products, chair) = catalog(&storage).await;
    storage
        .nodes()
        .delete(scope(B), &chair, DeleteNodeOptions::default())
        .await
        .unwrap();
    assert_eq!(resolve(&storage, "fr", "/produits/chaise"), None);
    assert!(claimants(&storage, B, "fr", &products, "chaise").is_empty());
    // A cascade takes the subtree's claims too.
    storage
        .nodes()
        .delete(scope(B), &products, DeleteNodeOptions::default())
        .await
        .unwrap();
    assert!(claimants(&storage, B, "fr", "/", "produits").is_empty());
}

/// `NodeService::restore_version` copies a version's content onto the node
/// and writes it through the transaction's `put_node` — the restore funnel.
/// A node's localized name is its translation's (`/__node_name`), never its
/// content, so restoring older content keeps the CURRENT name. (The
/// manual-version listing it reads first needs the async snapshot job; the
/// write is what this index sees, so the test performs exactly that write
/// with the content read back by time travel.)
#[tokio::test]
async fn restore_version_keeps_the_translated_name() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    register_type(&storage, PAGE, true).await;
    let typed = |path: &str| {
        let mut node = page(path, &[("title", "v1")]);
        node.node_type = PAGE.to_string();
        node
    };
    let products = create_on(&storage, B, typed("/products")).await;
    let chair = create_on(&storage, B, typed("/products/chair")).await;
    set_name(&storage, &products, "fr", "produits").await;
    set_name(&storage, &chair, "fr", "chaise").await;
    let version_one = head(&storage, B).await;
    update_props(&storage, &chair, &[("title", "v2")]).await;
    set_name(&storage, &chair, "fr", "siege").await;

    let snapshot = storage
        .nodes()
        .get(scope(B), &chair, Some(&version_one))
        .await
        .unwrap()
        .unwrap();
    let mut restored = get(&storage, &chair).await;
    restored.properties = snapshot.properties;
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_message("Restored from manual version 1").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.put_node(WS, &restored).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        id_via(&storage, B, "fr", "/produits/siege", Index),
        Some(chair)
    );
    assert_eq!(resolve(&storage, "fr", "/produits/chaise"), None);
}

const PING: &str = "test:LocalizedPing";
const PAGE: &str = "test:LocalizedPage";

async fn register_type(storage: &raisin_rocksdb::RocksDBStorage, name: &str, versionable: bool) {
    let ty = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
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
        versionable: Some(versionable),
        immutable: None,
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
            BranchScope::new(T, R, B),
            ty,
            CommitMetadata::system("seed"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn versionable_false_with_translated_names() {
    let (storage, _dir) = open().await;
    build(&storage, B).await;
    register_type(&storage, PING, false).await;
    let mut node = page("/health", &[("status", "ok")]);
    node.node_type = PING.to_string();
    let id = create_on(&storage, B, node).await;
    // Names from overlays written ABOVE the node's (reused) revision.
    set_name(&storage, &id, "fr", "sante").await;
    set_name(&storage, &id, "de", "gesundheit").await;
    // In-place rewrites at the node's own, older revision keep them.
    update_props(&storage, &id, &[("status", "degraded")]).await;
    set_name(&storage, &id, "fr", "etat").await;
    update_props(&storage, &id, &[("status", "ok")]).await;
    assert_eq!(id_via(&storage, B, "fr", "/etat", Index), Some(id.clone()));
    assert_eq!(resolve(&storage, "fr", "/sante"), None);
    assert_eq!(id_via(&storage, B, "de", "/gesundheit", Index), Some(id));
}
