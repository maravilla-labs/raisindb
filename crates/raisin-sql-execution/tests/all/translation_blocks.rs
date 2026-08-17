//! BLOCK-level locale overlays: resolution and what COPY carries.
//!
//! Block overlays live in their own key space, keyed by `(node_id, block_uuid,
//! locale)`, and are the half of the translation system nothing in the product
//! writes yet — which is exactly why both bugs here went unnoticed:
//!
//!   * they were only consulted when the NODE also had an overlay, so a block
//!     translated in a locale the node itself has nothing in resolved to nothing;
//!   * `COPY` carried the node's overlays and silently left the block ones behind,
//!     while a branch fork has always copied both column families.
//!
//! `NodeService::new` pins tenant/repo/workspace to `default` and the branch to
//! `main`, so these tests use those rather than their own names.

use raisin_context::RepositoryConfig;
use raisin_core::{BlockTranslationService, NodeService, TranslationResolver};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_models::workspace::Workspace;
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "default";
const REPO: &str = "default";
const BRANCH: &str = "main";
const WS: &str = "default";
const BLOCK: &str = "hero-block";

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
    storage
        .workspaces()
        .put(RepoScope::new(TENANT, REPO), Workspace::new(WS.to_string()))
        .await
        .expect("workspace");
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Page" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (storage, temp_dir)
}

#[allow(deprecated)]
fn nodes(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> NodeService<raisin_rocksdb::RocksDBStorage> {
    NodeService::new(storage.clone()).with_auth(AuthContext::system())
}

fn config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".into(), "de".into()],
        ..Default::default()
    }
}

/// A page whose `content` holds one uuid-addressed block.
fn page(name: &str) -> Node {
    let mut node = Node {
        id: String::new(),
        name: name.to_string(),
        path: String::new(),
        node_type: "test:Page".to_string(),
        version: 1,
        ..Default::default()
    };
    node.properties.insert(
        "title".to_string(),
        PropertyValue::String("Home".to_string()),
    );
    let mut block = HashMap::new();
    block.insert("uuid".to_string(), PropertyValue::String(BLOCK.to_string()));
    block.insert(
        "headline".to_string(),
        PropertyValue::String("Hello".to_string()),
    );
    node.properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(block)]),
    );
    node
}

/// Write a block overlay through the one service that writes them.
async fn translate_block(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    node_id: &str,
    value: &str,
) {
    let svc = BlockTranslationService::new(Arc::new(storage.translations().clone()));
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/headline"),
        PropertyValue::String(value.to_string()),
    );
    svc.update_block_translation(
        TENANT,
        REPO,
        BRANCH,
        WS,
        node_id,
        BLOCK,
        &LocaleCode::parse("de").unwrap(),
        data,
        "test",
        None,
        raisin_hlc::HLC::now(),
    )
    .await
    .expect("block translation write");
}

async fn resolve(storage: &Arc<raisin_rocksdb::RocksDBStorage>, node: Node) -> Node {
    let resolver = TranslationResolver::new(Arc::new(storage.translations().clone()), config());
    resolver
        .resolve_node(
            TENANT,
            REPO,
            BRANCH,
            WS,
            node,
            &LocaleCode::parse("de").unwrap(),
            &raisin_hlc::HLC::now(),
        )
        .await
        .expect("resolve")
        .expect("node visible")
}

fn headline(node: &Node) -> String {
    let PropertyValue::Array(items) = node.properties.get("content").expect("content") else {
        panic!("content is not an array");
    };
    let PropertyValue::Object(block) = &items[0] else {
        panic!("block is not an object");
    };
    match block.get("headline") {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("unexpected headline: {other:?}"),
    }
}

/// Block resolution used to hang off the node-overlay branch, so a block
/// translated in a locale where the NODE has no overlay was stored, listed, and
/// never applied.
#[tokio::test]
async fn a_block_overlay_resolves_without_a_node_overlay() {
    let (storage, _td) = setup().await;
    let created = nodes(&storage)
        .add_node("/", page("page"))
        .await
        .expect("create");

    translate_block(&storage, &created.id, "Hallo").await;

    let resolved = resolve(&storage, created).await;
    assert_eq!(
        headline(&resolved),
        "Hallo",
        "the block overlay must apply even though the node has no overlay of its own"
    );
}

/// COPY carried node overlays and dropped block ones — the copy looked complete
/// and was quietly less translated than the original.
#[tokio::test]
async fn copying_a_node_carries_its_block_overlays() {
    let (storage, _td) = setup().await;
    let svc = nodes(&storage);
    let source = svc.add_node("/", page("source")).await.expect("create");
    translate_block(&storage, &source.id, "Hallo").await;

    let copied = svc
        .copy_node("/source", "/", Some("copy"))
        .await
        .expect("copy");
    assert_ne!(copied.id, source.id, "a copy is a new node");

    let resolved = resolve(&storage, copied).await;
    assert_eq!(
        headline(&resolved),
        "Hallo",
        "the copy must carry the source's block overlays"
    );

    // And the original is unaffected.
    let source_again: Node = svc
        .get_by_path("/source")
        .await
        .expect("get")
        .expect("source");
    assert_eq!(headline(&resolve(&storage, source_again).await), "Hallo");
}
