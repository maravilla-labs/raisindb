//! The engine-owned property shield on mount-owned nodes.
//!
//! A client that loads a node, edits one field and saves the whole map back
//! echoes a STALE `__pushed_state` / `__etag` over the sync engine's current
//! ones. The engine's divergence check then reads "already pushed" for the
//! very value the user just set, and the edit is silently never written to
//! the provider — in production this surfaced as a Hue lamp whose every
//! second toggle did nothing. These tests pin the shield on both write paths:
//! the commit-path MERGE and the UpdateBuilder REPLACE.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::{properties::PropertyValue, Node, SYNC_ACTOR};

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

#[cfg(not(feature = "storage-rocksdb"))]
fn setup_storage() -> Arc<InMemoryStorage> {
    Arc::new(InMemoryStorage::default())
}

#[cfg(feature = "storage-rocksdb")]
fn setup_storage() -> Arc<RocksDBStorage> {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let storage = RocksDBStorage::open(path).unwrap();
    std::mem::forget(dir);
    Arc::new(storage)
}

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// A mount-owned lamp node as the sync engine would materialize it.
fn lamp(path: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert("on".to_string(), PropertyValue::Boolean(false));
    properties.insert("__virtual".to_string(), PropertyValue::Boolean(true));
    properties.insert("__mount_id".to_string(), s("mount-1"));
    properties.insert("__external_id".to_string(), s("svc-light"));
    properties.insert("__etag".to_string(), s("etag-current"));
    properties.insert("__pushed_state".to_string(), s("{\"on\":false}"));

    Node {
        id: nanoid::nanoid!(),
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        node_type: "test:Lamp".to_string(),
        properties,
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    }
}

/// Register a minimal NON-STRICT `test:Lamp` type: the UpdateBuilder path
/// resolves the node type before saving, and the shield must be exercised
/// through it, not around it.
async fn register_lamp_type<S: raisin_storage::Storage>(storage: &S) {
    use raisin_storage::{BranchScope, CommitMetadata, NodeTypeRepository};

    let node_type = raisin_models::nodes::types::NodeType {
        id: Some("test:Lamp".to_string()),
        name: "test:Lamp".to_string(),
        strict: Some(false),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(false),
        auditable: Some(false),
        indexable: Some(false),
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
            BranchScope::new("default", "default", "main"),
            node_type,
            CommitMetadata::system("create test node type"),
        )
        .await
        .unwrap();
}

async fn create_lamp<S>(service: &NodeService<S>, path: &str)
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage,
{
    let mut tx = service.transaction();
    tx.create(lamp(path));
    tx.commit("materialize", SYNC_ACTOR).await.unwrap();
}

fn prop_str(node: &Node, key: &str) -> String {
    match node.properties.get(key) {
        Some(PropertyValue::String(v)) => v.clone(),
        other => panic!("{key} is not a string: {other:?}"),
    }
}

/// Commit-path merge: a user commit echoing stale engine keys must move the
/// content and leave the bookkeeping alone.
#[tokio::test]
async fn a_user_commit_cannot_roll_back_the_sync_baseline() {
    let storage = setup_storage();
    register_lamp_type(storage.as_ref()).await;
    let service = NodeService::new(storage.clone()).with_auth(AuthContext::system());
    create_lamp(&service, "/lamp-merge").await;
    let node = service.get_by_path("/lamp-merge").await.unwrap().unwrap();

    // The console's payload: the toggled field PLUS the whole stale snapshot.
    let mut tx = service.transaction();
    tx.update(
        node.id.clone(),
        serde_json::json!({
            "on": true,
            "__etag": "etag-STALE",
            "__pushed_state": "{\"on\":true}",
            "__external_id": "svc-FORGED",
        }),
    );
    tx.commit("toggle", "console-user").await.unwrap();

    let after = service.get_by_path("/lamp-merge").await.unwrap().unwrap();
    assert_eq!(
        after.properties.get("on"),
        Some(&PropertyValue::Boolean(true)),
        "the user's actual edit must land"
    );
    // The engine's bookkeeping is untouched — this is what keeps the drain's
    // divergence check honest ("on: true" vs pushed "on: false" => push).
    assert_eq!(prop_str(&after, "__etag"), "etag-current");
    assert_eq!(prop_str(&after, "__pushed_state"), "{\"on\":false}");
    assert_eq!(prop_str(&after, "__external_id"), "svc-light");
}

/// The same commit path must stay fully open to the sync engine itself, or the
/// shield would lock the engine out of its own metadata.
#[tokio::test]
async fn the_sync_actor_still_moves_its_own_bookkeeping() {
    let storage = setup_storage();
    register_lamp_type(storage.as_ref()).await;
    let service = NodeService::new(storage.clone()).with_auth(AuthContext::system());
    create_lamp(&service, "/lamp-sync").await;
    let node = service.get_by_path("/lamp-sync").await.unwrap().unwrap();

    let mut tx = service.transaction();
    tx.update(
        node.id.clone(),
        serde_json::json!({
            "__etag": "etag-new",
            "__pushed_state": "{\"on\":true}",
        }),
    );
    tx.commit("stamp", SYNC_ACTOR).await.unwrap();

    let after = service.get_by_path("/lamp-sync").await.unwrap().unwrap();
    assert_eq!(prop_str(&after, "__etag"), "etag-new");
    assert_eq!(prop_str(&after, "__pushed_state"), "{\"on\":true}");
}

/// UpdateBuilder REPLACES the property map, so the shield must both drop
/// echoed stale values AND carry stored engine keys a client omitted —
/// otherwise a plain save would detach the node from its mount.
#[tokio::test]
async fn a_wholesale_replace_keeps_the_stored_engine_keys() {
    let storage = setup_storage();
    register_lamp_type(storage.as_ref()).await;
    let service = NodeService::new(storage.clone()).with_auth(AuthContext::system());
    create_lamp(&service, "/lamp-replace").await;

    // The client sends only content plus one stale engine key; every other
    // engine key is omitted entirely.
    let mut props = HashMap::new();
    props.insert("on".to_string(), PropertyValue::Boolean(true));
    props.insert("__etag".to_string(), s("etag-STALE"));

    service
        .update("default", "/lamp-replace")
        .with_properties(props)
        .save()
        .await
        .unwrap();

    let after = service.get_by_path("/lamp-replace").await.unwrap().unwrap();
    assert_eq!(
        after.properties.get("on"),
        Some(&PropertyValue::Boolean(true))
    );
    assert_eq!(
        prop_str(&after, "__etag"),
        "etag-current",
        "stale echo dropped"
    );
    assert_eq!(
        prop_str(&after, "__pushed_state"),
        "{\"on\":false}",
        "omitted key carried"
    );
    assert_eq!(
        prop_str(&after, "__mount_id"),
        "mount-1",
        "mount ownership survives a save"
    );
    assert_eq!(
        after.properties.get("__virtual"),
        Some(&PropertyValue::Boolean(true))
    );
}

/// A node that is NOT mount-owned keeps today's behaviour: the shield keys on
/// `__virtual`, not on the key prefix alone, so import/restore flows that
/// write `__`-style properties on ordinary nodes are untouched.
#[tokio::test]
async fn ordinary_nodes_are_not_shielded() {
    let storage = setup_storage();
    register_lamp_type(storage.as_ref()).await;
    let service = NodeService::new(storage.clone()).with_auth(AuthContext::system());

    let mut node = lamp("/plain");
    node.properties.remove("__virtual");
    let mut tx = service.transaction();
    tx.create(node);
    tx.commit("create", "importer").await.unwrap();
    let created = service.get_by_path("/plain").await.unwrap().unwrap();

    let mut tx = service.transaction();
    tx.update(
        created.id.clone(),
        serde_json::json!({ "__etag": "imported" }),
    );
    tx.commit("import", "importer").await.unwrap();

    let after = service.get_by_path("/plain").await.unwrap().unwrap();
    assert_eq!(prop_str(&after, "__etag"), "imported");
}
