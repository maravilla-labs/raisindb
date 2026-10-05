//! Phase 2 read-side fixes: the readers stay correct over data the old writers
//! left behind.
//!
//! - item 2: a one-byte `\0` PATH_INDEX value (what merge apply used to write
//!   as its tombstone) is a tombstone to every path reader;
//! - item 3: a bulk descendant read at a past revision includes a node deleted
//!   only later;
//! - item 4: a child looked up by name is matched on its NEWEST index entry,
//!   never on an older one carrying a previous name.

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

const TENANT: &str = "p2-tenant";
const REPO: &str = "repo";
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
                localized_names: Default::default(),
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

async fn create(storage: &RocksDBStorage, path: &str) -> Result<String> {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    let id = node.id.clone();
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    storage.nodes().create(scope(), node, options).await?;
    Ok(id)
}

async fn head(storage: &RocksDBStorage) -> Result<HLC> {
    storage.branches().get_head(TENANT, REPO, BRANCH).await
}

#[tokio::test]
async fn path_reader_treats_nul_byte_as_tombstone() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let id = create(&storage, "/a").await?;
    let before = head(&storage).await?;
    create(&storage, "/b").await?;
    let after = head(&storage).await?;

    // The marker merge apply used to write for a vacated path.
    let key = keys::path_index_key_versioned(TENANT, REPO, BRANCH, WORKSPACE, "/a", &after);
    let db = storage.db();
    db.put_cf(db.cf_handle(cf::PATH_INDEX).unwrap(), key, b"\x00")
        .unwrap();

    let nodes = storage.nodes();
    assert_eq!(nodes.get_node_id_by_path(scope(), "/a", None).await?, None);
    assert!(nodes.get_by_path(scope(), "/a", None).await?.is_none());
    assert_eq!(
        nodes
            .get_node_id_by_path(scope(), "/a", Some(&before))
            .await?
            .as_deref(),
        Some(id.as_str()),
        "below the marker the path still resolves"
    );
    Ok(())
}

#[tokio::test]
async fn bulk_descendants_at_past_revision_includes_later_deleted() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create(&storage, "/p").await?;
    let x = create(&storage, "/p/x").await?;
    let past = head(&storage).await?;
    storage
        .nodes()
        .delete(scope(), &x, DeleteNodeOptions::default())
        .await?;

    let ids = |nodes: Vec<raisin_models::nodes::NodeWithChildren>| {
        nodes.into_iter().map(|n| n.node.id).collect::<Vec<_>>()
    };
    let nodes = storage.nodes();
    assert_eq!(
        ids(nodes
            .deep_children_array(scope(), "/p", 1, Some(&past))
            .await?),
        vec![x.clone()],
        "as of before the delete, x was there"
    );
    assert!(ids(nodes.deep_children_array(scope(), "/p", 1, None).await?).is_empty());
    Ok(())
}

/// The child's index entry under one label was rewritten with a new name; the
/// OLD name sits in an older revision of the same `(label, child)` entry and
/// must never be matched.
#[tokio::test]
async fn find_child_by_name_ignores_renamed_entry() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let parent = create(&storage, "/parent").await?;
    create(&storage, "/parent/x").await?;
    let c = create(&storage, "/parent/c").await?;

    let db = storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    for (revision, name) in [(HLC::new(1, 0), "old-name"), (HLC::new(2, 0), "c")] {
        let key = keys::ordered_child_key_versioned(
            TENANT, REPO, BRANCH, WORKSPACE, &parent, "zz", &revision, &c,
        );
        db.put_cf(cf, key, name.as_bytes()).unwrap();
    }

    let nodes = storage.nodes();
    let by_old_name = nodes
        .move_child_before(scope(), "/parent", "old-name", "x", None, None)
        .await;
    assert!(
        by_old_name.is_err(),
        "a superseded name must not resolve to the child"
    );
    nodes
        .move_child_before(scope(), "/parent", "c", "x", None, None)
        .await?;
    Ok(())
}
