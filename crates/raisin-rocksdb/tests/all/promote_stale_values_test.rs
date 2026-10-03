//! A cross-branch promotion retires what the source no longer has.
//!
//! Promotion (`copy_nodes_across_branches`) is an UPSERT onto the target
//! branch, and it used to tombstone only the destination's old compound and
//! unique entries. A property or reference REMOVED on the source therefore
//! stayed live in the target's PROPERTY_INDEX and REFERENCE_INDEX: the
//! published branch kept answering `properties->>'k' = old` and
//! `REFERENCES(...)` for content that no longer said so. The repository's own
//! `find_by_property` re-checks the decoded node and hides it; these assert on
//! the INDEXES, which is what a scan's candidates and a pushed-down COUNT read.

use raisin_context::RepositoryConfig;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, PropertyIndexRepository,
    ReferenceIndexRepository, RegistryRepository, RepoScope, RepositoryManagementRepository,
    Storage, StorageScope, UpdateNodeOptions, WorkspaceRepository,
};
use std::collections::HashMap;
use tempfile::TempDir;

const TENANT: &str = "ps-tenant";
const REPO: &str = "ps-repo";
const WS: &str = "content";

fn main_scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, "main", WS)
}

fn publish_scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, "publish", WS)
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
                default_branch: "main".to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await
        .expect("repo");
    for branch in ["main", "publish"] {
        storage
            .branches()
            .create_branch(TENANT, REPO, branch, "system", None, None, false, false)
            .await
            .expect("branch");
    }
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

fn node(name: &str) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: format!("/{name}"),
        name: name.to_string(),
        node_type: "raisin:Page".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

async fn promote(storage: &RocksDBStorage, roots: &[String]) {
    storage
        .nodes()
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WS, roots, true, false, None, None,
        )
        .await
        .expect("promote");
}

#[tokio::test]
async fn promote_removed_property_not_matched_on_publish_branch() {
    let (storage, _dir) = setup().await;
    let nodes = storage.nodes();
    let create = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };

    let target = node("target");
    nodes
        .create(main_scope(), target.clone(), create.clone())
        .await
        .expect("target");
    let mut page = node("page");
    page.properties.insert(
        "campaign".to_string(),
        PropertyValue::String("summer".to_string()),
    );
    page.properties.insert(
        "related".to_string(),
        PropertyValue::Reference(RaisinReference {
            id: target.id.clone(),
            workspace: WS.to_string(),
            path: target.path.clone(),
        }),
    );
    nodes
        .create(main_scope(), page.clone(), create)
        .await
        .expect("page");

    let roots = vec!["/target".to_string(), "/page".to_string()];
    promote(&storage, &roots).await;

    let campaign = PropertyValue::String("summer".to_string());
    let indexed = storage
        .property_index()
        .find_by_property(publish_scope(), "campaign", &campaign, false, None)
        .await
        .expect("index read");
    assert_eq!(indexed, vec![page.id.clone()], "first promotion indexes it");
    let referrers = storage
        .reference_index()
        .find_referencing_nodes(publish_scope(), WS, &target.id, false)
        .await
        .expect("reference read");
    assert_eq!(referrers.len(), 1, "first promotion indexes the reference");

    // The source drops both, and is promoted again.
    let mut edited = nodes
        .get(main_scope(), &page.id, None)
        .await
        .expect("get")
        .expect("page");
    edited.properties.remove("campaign");
    edited.properties.remove("related");
    nodes
        .update(
            main_scope(),
            edited,
            UpdateNodeOptions {
                validate_schema: false,
                allow_type_change: false,
                operation_meta: None,
            },
        )
        .await
        .expect("update");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    promote(&storage, &roots).await;

    let indexed = storage
        .property_index()
        .find_by_property(publish_scope(), "campaign", &campaign, false, None)
        .await
        .expect("index read");
    assert!(
        indexed.is_empty(),
        "a property removed on the source still matches on the publish branch: {indexed:?}"
    );
    let referrers = storage
        .reference_index()
        .find_referencing_nodes(publish_scope(), WS, &target.id, false)
        .await
        .expect("reference read");
    assert!(
        referrers.is_empty(),
        "a reference removed on the source still matches on the publish branch: {referrers:?}"
    );
}
