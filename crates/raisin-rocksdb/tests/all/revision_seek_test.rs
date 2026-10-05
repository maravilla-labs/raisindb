//! Time-travel reads SEEK to the version they want instead of walking every
//! newer one.
//!
//! A node's versions sit under one prefix, newest first. Reading it as of an
//! old revision used to step over every newer version (and then point-read the
//! blob it had just passed), so the cost grew with the node's edit count. These
//! pin both halves: every recorded revision still reads back its own version,
//! and reading the OLDEST of 200 versions costs what reading the newest does.

use std::collections::HashMap;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use tempfile::TempDir;

use crate::perf_counters::count_async;

const TENANT: &str = "seek-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";
const NODE_ID: &str = "seek-node";
const EDITS: usize = 200;

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

async fn setup() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    // DB statistics on: RocksDB only feeds the `iter_read_bytes` perf counter
    // when they are, and that counter is the one that sees SST reads too.
    let mut config = RocksDBConfig::development().with_path(temp_dir.path());
    config.enable_statistics = true;
    let storage = RocksDBStorage::with_config(config)?;
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

fn version(i: usize) -> Node {
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(format!("v{i}")));
    Node {
        id: NODE_ID.to_string(),
        name: "seek".to_string(),
        path: "/seek".to_string(),
        node_type: "raisin:Folder".to_string(),
        properties,
        order_key: fractional_index::first(),
        parent: Some("/".to_string()),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

/// Commit `EDITS` versions of one node; returns HEAD after each commit.
async fn edit_many_times(storage: &RocksDBStorage) -> Result<Vec<HLC>> {
    let mut revisions = Vec::with_capacity(EDITS);
    for i in 0..EDITS {
        let tx = storage.begin_context().await?;
        tx.set_tenant_repo(TENANT, REPO)?;
        tx.set_branch(BRANCH)?;
        tx.set_message(&format!("edit {i}"))?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_validate_schema(false)?;
        tx.put_node(WORKSPACE, &version(i)).await?;
        tx.commit().await?;
        revisions.push(storage.branches().get_head(TENANT, REPO, BRANCH).await?);
    }
    Ok(revisions)
}

fn title(node: &Node) -> Option<&str> {
    match node.properties.get("title") {
        Some(PropertyValue::String(s)) => Some(s),
        _ => None,
    }
}

#[tokio::test]
async fn every_recorded_revision_reads_back_its_own_version() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let revisions = edit_many_times(&storage).await?;
    let nodes = storage.nodes();

    for (i, revision) in revisions.iter().enumerate() {
        let node = nodes
            .get(scope(), NODE_ID, Some(revision))
            .await?
            .unwrap_or_else(|| panic!("version {i} must exist at {revision}"));
        assert_eq!(
            title(&node),
            Some(format!("v{i}").as_str()),
            "at {revision}"
        );

        let by_path = nodes
            .get_by_path(scope(), "/seek", Some(revision))
            .await?
            .unwrap_or_else(|| panic!("version {i} must resolve by path at {revision}"));
        assert_eq!(by_path.id, NODE_ID);
        assert_eq!(title(&by_path), Some(format!("v{i}").as_str()));
    }

    // Before the first version there is nothing.
    let first = revisions[0];
    let before = HLC::new(first.timestamp_ms.saturating_sub(1), 0);
    assert!(nodes.get(scope(), NODE_ID, Some(&before)).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn reading_the_oldest_of_many_versions_costs_what_the_newest_does() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let revisions = edit_many_times(&storage).await?;
    let nodes = storage.nodes();
    let oldest = revisions[0];
    let newest = *revisions.last().unwrap();

    // Warm both paths once so neither measurement pays first-touch costs.
    nodes.get(scope(), NODE_ID, Some(&oldest)).await?;
    nodes.get(scope(), NODE_ID, Some(&newest)).await?;

    let (found, at_newest) = count_async(nodes.get(scope(), NODE_ID, Some(&newest))).await;
    assert!(found?.is_some());
    let (found, at_oldest) = count_async(nodes.get(scope(), NODE_ID, Some(&oldest))).await;
    assert!(found?.is_some());

    // The counters must be live, or every bound below passes vacuously.
    assert!(
        at_newest.seek_on_memtable > 0 && at_newest.iter_read_bytes > 0,
        "perf counters recorded nothing: {at_newest:?}"
    );

    // A walk would read all 199 newer versions' keys and blobs first.
    assert!(
        at_oldest.iter_read_bytes <= 2 * at_newest.iter_read_bytes + 256,
        "get at the oldest revision read {} iterator bytes vs {} at the newest — \
         the time-travel read is walking newer versions\n{at_oldest:?}\n{at_newest:?}",
        at_oldest.iter_read_bytes,
        at_newest.iter_read_bytes
    );
    assert!(
        at_oldest.next_on_memtable <= at_newest.next_on_memtable + 4,
        "get at the oldest revision stepped {} times vs {} at the newest\n\
         {at_oldest:?}\n{at_newest:?}",
        at_oldest.next_on_memtable,
        at_newest.next_on_memtable
    );

    let (found, by_path_newest) =
        count_async(nodes.get_by_path(scope(), "/seek", Some(&newest))).await;
    assert!(found?.is_some());
    let (found, by_path_oldest) =
        count_async(nodes.get_by_path(scope(), "/seek", Some(&oldest))).await;
    assert!(found?.is_some());
    assert!(
        by_path_oldest.iter_read_bytes <= 2 * by_path_newest.iter_read_bytes + 256,
        "get_by_path at the oldest revision: {by_path_oldest:?} vs newest {by_path_newest:?}"
    );
    Ok(())
}
