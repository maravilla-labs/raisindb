//! The reference index's writer, delete tombstoner and REBUILD walk ONE tree
//! the same way.
//!
//! Each used to carry its own recursion: the delete tombstoner never descended
//! `Composite`, and the REBUILD skipped `Element` and `Composite` altogether. So
//! a reference inside a block was indexed on write, never tombstoned on delete
//! (`REFERENCES()` kept matching a deleted node), and dropped by a rebuild (it
//! stopped matching a live one). These pin that all three address exactly the
//! same `(node, property path)` entries for every container kind.

use raisin_context::RepositoryConfig;
use raisin_models::nodes::properties::value::{Composite, Element};
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_rocksdb::management::async_indexing::rebuild_indexes;
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, IndexType, NodeRepository,
    RegistryRepository, RepoScope, RepositoryManagementRepository, Storage, StorageScope,
    WorkspaceRepository,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tempfile::TempDir;

const TENANT: &str = "rw-tenant";
const REPO: &str = "rw-repo";
const BRANCH: &str = "main";
const WS: &str = "content";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

async fn setup() -> (RocksDBStorage, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).expect("storage");
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await
        .expect("tenant");
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
        .await
        .expect("repo");
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await
        .expect("branch");
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");
    (storage, temp_dir)
}

fn options() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn node(name: &str, properties: HashMap<String, PropertyValue>) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: format!("/{name}"),
        name: name.to_string(),
        node_type: "raisin:Folder".to_string(),
        properties,
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

fn reference(target: &Node) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: target.id.clone(),
        workspace: WS.to_string(),
        path: target.path.clone(),
    })
}

fn element(element_type: &str, content: HashMap<String, PropertyValue>) -> Element {
    Element {
        uuid: uuid::Uuid::new_v4().to_string(),
        element_type: element_type.to_string(),
        content,
    }
}

/// A reference in every container the walker knows: top level, array, object,
/// element content and composite blocks.
fn every_container(target: &Node) -> HashMap<String, PropertyValue> {
    HashMap::from([
        ("hero".to_string(), reference(target)),
        (
            "tags".to_string(),
            PropertyValue::Array(vec![reference(target), reference(target)]),
        ),
        (
            "meta".to_string(),
            PropertyValue::Object(HashMap::from([("author".to_string(), reference(target))])),
        ),
        (
            "banner".to_string(),
            PropertyValue::Element(element(
                "x:Banner",
                HashMap::from([("image".to_string(), reference(target))]),
            )),
        ),
        (
            "blocks".to_string(),
            PropertyValue::Composite(Composite {
                uuid: uuid::Uuid::new_v4().to_string(),
                items: vec![
                    element(
                        "x:Teaser",
                        HashMap::from([("link".to_string(), reference(target))]),
                    ),
                    element(
                        "x:Gallery",
                        HashMap::from([(
                            "items".to_string(),
                            PropertyValue::Array(vec![reference(target)]),
                        )]),
                    ),
                ],
            }),
        ),
    ])
}

const EXPECTED_PATHS: [&str; 7] = [
    "banner.image",
    "blocks.0.link",
    "blocks.1.items.0",
    "hero",
    "meta.author",
    "tags.0",
    "tags.1",
];

/// The forward REFERENCE_INDEX entries of `node_id`: for each property path,
/// whether its newest entry is live.
fn forward_entries(storage: &RocksDBStorage, node_id: &str) -> BTreeMap<String, bool> {
    let prefix = keys::KeyBuilder::new()
        .push(TENANT)
        .push(REPO)
        .push(BRANCH)
        .push(WS)
        .push("ref")
        .push(node_id)
        .build_prefix();
    let cf = storage.db().cf_handle(cf::REFERENCE_INDEX).expect("cf");
    let mut newest: BTreeMap<String, bool> = BTreeMap::new();
    let iter = storage.db().iterator_cf(
        cf,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    );
    for item in iter {
        let (key, value) = item.expect("iter");
        if !key.starts_with(&prefix) {
            break;
        }
        // {prefix}{path}\0{16-byte descending revision}: newest first per path.
        let path = String::from_utf8_lossy(&key[prefix.len()..key.len() - 17]).into_owned();
        newest.entry(path).or_insert(value.as_ref() != b"T");
    }
    newest
}

fn live_paths(entries: &BTreeMap<String, bool>) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|(_, live)| **live)
        .map(|(p, _)| p.clone())
        .collect()
}

#[tokio::test]
async fn reference_walkers_agree() {
    let (storage, _dir) = setup().await;
    let target = node("target", HashMap::new());
    storage
        .nodes()
        .create(scope(), target.clone(), options())
        .await
        .expect("target");
    let source = node("source", every_container(&target));
    storage
        .nodes()
        .create(scope(), source.clone(), options())
        .await
        .expect("source");

    let expected: BTreeSet<String> = EXPECTED_PATHS.iter().map(|p| p.to_string()).collect();

    // WRITER: one live entry per reference, at every container kind.
    let written = forward_entries(&storage, &source.id);
    assert_eq!(live_paths(&written), expected, "writer");

    // REBUILD: clears and rewrites — the same set, Element and Composite included.
    rebuild_indexes(&storage, TENANT, REPO, BRANCH, WS, IndexType::Reference)
        .await
        .expect("rebuild");
    let rebuilt = forward_entries(&storage, &source.id);
    assert_eq!(live_paths(&rebuilt), expected, "rebuild");

    // DELETE: every one of those entries is tombstoned at HEAD.
    storage
        .nodes()
        .delete(scope(), &source.id, DeleteNodeOptions::default())
        .await
        .expect("delete");
    let deleted = forward_entries(&storage, &source.id);
    let still_live = live_paths(&deleted);
    assert!(
        still_live.is_empty(),
        "delete left reference entries live: {still_live:?}"
    );
    let tombstoned: BTreeSet<String> = deleted.keys().cloned().collect();
    assert_eq!(
        tombstoned, expected,
        "delete tombstoner addresses the same paths"
    );
}
