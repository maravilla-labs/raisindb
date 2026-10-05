//! Copies carry translations through the one translation reader.
//!
//! - A publish (cross-branch copy) wrote the source's live overlays and left
//!   every overlay the target held but the source no longer did: a translation
//!   deleted on the draft stayed on the published site for good.
//! - Both copies read overlays through a hand-rolled second reader that
//!   decoded a block's orphan marker as an overlay, so one
//!   `mark_blocks_orphaned` made every later publish and tree copy of that
//!   node fail.

use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_rocksdb::{cf, RocksDBStorage};
use raisin_storage::scope::StorageScope;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, TranslationRepository,
};
use std::collections::HashMap;
use tempfile::TempDir;

const T: &str = "copy-tenant";
const R: &str = "copy-repo";
const WS: &str = "default";

async fn open() -> (RocksDBStorage, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(dir.path()).unwrap();
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(
            T,
            R,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".into(), "de".into(), "fr".into()],
                locale_fallback_chains: HashMap::new(),
                default_branch: "main".to_string(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await
        .unwrap();
    for branch in ["main", "publish"] {
        storage
            .branches()
            .create_branch(T, R, branch, "test", None, None, false, false)
            .await
            .unwrap();
    }
    (storage, dir)
}

async fn page(storage: &RocksDBStorage, path: &str) -> String {
    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: path.trim_start_matches('/').to_string(),
        path: path.to_string(),
        node_type: "raisin:Page".to_string(),
        order_key: "a0".to_string(),
        created_at: Some(chrono::Utc::now()),
        workspace: Some(WS.to_string()),
        tenant_id: Some(T.to_string()),
        ..Node::default()
    };
    let id = node.id.clone();
    storage
        .nodes()
        .create(
            StorageScope::new(T, R, "main", WS),
            node,
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await
        .unwrap();
    id
}

fn latest() -> HLC {
    HLC::new(u64::MAX, u64::MAX)
}

fn overlay(text: &str) -> LocaleOverlay {
    crate::translation_substrate_test::title(text)
}

fn code(locale: &str) -> LocaleCode {
    LocaleCode::parse(locale).unwrap()
}

/// A translation written at a revision below every HEAD the test reaches.
async fn translate(storage: &RocksDBStorage, id: &str, locale: &str, at: u64) {
    let meta = TranslationMeta::system(code(locale), HLC::new(at, 0), "t".to_string());
    storage
        .translations()
        .store_translation(T, R, "main", WS, id, &code(locale), &overlay(locale), &meta)
        .await
        .unwrap();
}

/// What deleting a translation stores: a `T` version (raw, as the one writer
/// stages it).
fn delete_translation(storage: &RocksDBStorage, id: &str, locale: &str, at: u64) {
    let db = storage.db();
    let mut key = format!("{T}\0{R}\0main\0{WS}\0translations\0{id}\0{locale}\0").into_bytes();
    key.extend_from_slice(&HLC::new(at, 0).encode_descending());
    db.put_cf(db.cf_handle(cf::TRANSLATION_DATA).unwrap(), key, b"T")
        .unwrap();
}

async fn publish(storage: &RocksDBStorage, paths: &[&str]) {
    let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
    storage
        .nodes()
        .copy_nodes_across_branches(T, R, "main", "publish", WS, &paths, true, false, None, None)
        .await
        .unwrap();
}

async fn published(storage: &RocksDBStorage, id: &str, locale: &str) -> Option<LocaleOverlay> {
    storage
        .translations()
        .get_translation(T, R, "publish", WS, id, &code(locale), &latest())
        .await
        .unwrap()
}

#[tokio::test]
async fn publish_removes_a_translation_deleted_on_the_source() {
    let (storage, _dir) = open().await;
    let id = page(&storage, "/about").await;
    translate(&storage, &id, "de", 10).await;
    translate(&storage, &id, "fr", 11).await;
    publish(&storage, &["/about"]).await;
    assert_eq!(published(&storage, &id, "de").await, Some(overlay("de")));

    // The draft deletes its German translation; the next publish must too.
    delete_translation(&storage, &id, "de", 20);
    publish(&storage, &["/about"]).await;
    assert_eq!(published(&storage, &id, "de").await, None);
    assert_eq!(published(&storage, &id, "fr").await, Some(overlay("fr")));
    let listed = storage
        .translations()
        .list_translations_for_node(T, R, "publish", WS, &id, &latest())
        .await
        .unwrap();
    assert_eq!(listed, vec![code("fr")]);
}

#[tokio::test]
async fn copy_of_a_node_with_an_orphan_marker_succeeds() {
    let (storage, _dir) = open().await;
    let id = page(&storage, "/blocks").await;
    let repo = storage.translations();
    let meta = TranslationMeta::system(code("de"), HLC::new(10, 0), "t".to_string());
    for block in ["kept", "dropped"] {
        repo.store_block_translation(
            T,
            R,
            "main",
            WS,
            &id,
            block,
            &code("de"),
            &overlay(block),
            &meta,
        )
        .await
        .unwrap();
    }
    repo.mark_blocks_orphaned(
        T,
        R,
        "main",
        WS,
        &id,
        &["dropped".to_string()],
        &HLC::new(11, 0),
    )
    .await
    .unwrap();

    // Publish (cross-branch copy) and tree copy both go through the reader.
    publish(&storage, &["/blocks"]).await;
    let block_on = |branch: &'static str, node: String, block: &'static str| {
        let repo = storage.translations().clone();
        async move {
            repo.get_block_translation(T, R, branch, WS, &node, block, &code("de"), &latest())
                .await
                .unwrap()
        }
    };
    assert_eq!(
        block_on("publish", id.clone(), "kept").await,
        Some(overlay("kept"))
    );

    let copy = storage
        .nodes()
        .copy_node_tree(
            StorageScope::new(T, R, "main", WS),
            "/blocks",
            "/",
            Some("blocks-copy"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        block_on("main", copy.id.clone(), "kept").await,
        Some(overlay("kept"))
    );
}
