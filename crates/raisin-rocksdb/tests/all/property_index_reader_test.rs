//! The one revision-bounded `PROPERTY_INDEX` reader.
//!
//! Every read decides each `(value, node)` pair on its own, as of a revision
//! (the caller's, else the branch HEAD). These pin the bugs the five loops it
//! replaced had:
//!
//! - no revision bound: a historical read matched today's values;
//! - a NODE-WIDE tombstone set: a node's tombstone under its OLD value hid its
//!   live entry under the NEW one, so an ascending `updated_at` scan dropped
//!   every node ever updated;
//! - timestamp range bounds encoded as a 20-digit nanosecond STRING against
//!   keys holding 8-byte big-endian microseconds;
//! - `list_by_type` trusting an orphan `__node_type` entry.

use std::collections::HashMap;
use std::time::Duration;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::{
    cf, detect_property_index_orphans, fractional_index, keys, PropertyIndexOrphanReason,
    RocksDBConfig, RocksDBStorage,
};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, ListOptions, NodeRepository, PropertyIndexRepository, RegistryRepository,
    RepoScope, RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use tempfile::TempDir;

use crate::perf_counters::count_async;

const TENANT: &str = "pidx-tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

async fn setup() -> Result<(RocksDBStorage, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    // Statistics on: RocksDB only feeds `iter_read_bytes` when they are.
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

fn node(id: &str, node_type: &str, props: &[(&str, &str)]) -> Node {
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        node_type: node_type.to_string(),
        properties: props
            .iter()
            .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
            .collect(),
        order_key: fractional_index::first(),
        parent: Some("/".to_string()),
        ..Node::default()
    }
}

/// Commit one version of `node` through the transaction write path; returns
/// the HEAD after the commit.
async fn put(storage: &RocksDBStorage, node: &Node) -> Result<HLC> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("write")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.put_node(WORKSPACE, node).await?;
    tx.commit().await?;
    // Keep successive writes' timestamps apart.
    tokio::time::sleep(Duration::from_millis(2)).await;
    storage.branches().get_head(TENANT, REPO, BRANCH).await
}

async fn find(storage: &RocksDBStorage, prop: &str, value: &str, at: Option<&HLC>) -> Vec<String> {
    let mut ids = storage
        .property_index()
        .find_by_property(
            scope(),
            prop,
            &PropertyValue::String(value.to_string()),
            false,
            at,
        )
        .await
        .unwrap();
    ids.sort();
    ids
}

#[tokio::test]
async fn property_eq_at_historical_revision() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let r1 = put(&storage, &node("a", "raisin:Folder", &[("slug", "old")])).await?;
    let r2 = put(&storage, &node("a", "raisin:Folder", &[("slug", "new")])).await?;

    assert_eq!(find(&storage, "slug", "old", Some(&r1)).await, vec!["a"]);
    assert!(find(&storage, "slug", "new", Some(&r1)).await.is_empty());
    assert!(find(&storage, "slug", "old", None).await.is_empty());
    assert_eq!(find(&storage, "slug", "new", None).await, vec!["a"]);
    assert_eq!(find(&storage, "slug", "new", Some(&r2)).await, vec!["a"]);

    let count_at_r1 = storage
        .property_index()
        .count_by_property(
            scope(),
            "slug",
            &PropertyValue::String("old".into()),
            false,
            Some(&r1),
        )
        .await?;
    assert_eq!(count_at_r1, 1, "COUNT is as of the revision too");

    // Pseudo-property equality at a historical revision.
    assert_eq!(find(&storage, "__name", "a", Some(&r1)).await, vec!["a"]);
    Ok(())
}

/// An unchanged value on a node edited many times: one entry per edit sits in
/// its value group, and a LIMIT 1 read must stop at the first.
#[tokio::test]
async fn unchanged_value_on_an_edited_node_is_found_in_constant_reads() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    for i in 0..100 {
        let rev = i.to_string();
        put(
            &storage,
            &node("a", "raisin:Folder", &[("slug", "same"), ("rev", &rev)]),
        )
        .await?;
    }

    let index = storage.property_index();
    let value = PropertyValue::String("same".to_string());
    let (found, counts) = count_async(index.find_by_property_with_limit(
        scope(),
        "slug",
        &value,
        false,
        None,
        Some(1),
    ))
    .await;
    assert_eq!(found?, vec!["a".to_string()]);
    assert!(
        counts.seek_on_memtable > 0,
        "perf counters recorded nothing: {counts:?}"
    );
    assert!(
        counts.next_on_memtable < 10,
        "LIMIT 1 stepped over {} entries — it walked the node's history\n{counts:?}",
        counts.next_on_memtable
    );
    Ok(())
}

#[tokio::test]
async fn order_by_updated_at_asc_includes_updated_nodes() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    put(&storage, &node("n1", "raisin:Folder", &[("v", "1")])).await?;
    put(&storage, &node("n2", "raisin:Folder", &[("v", "1")])).await?;
    // n1's updated_at moves past n2's: its OLD value is now tombstoned.
    put(&storage, &node("n1", "raisin:Folder", &[("v", "2")])).await?;

    for ascending in [true, false] {
        let entries = storage
            .property_index()
            .scan_property(scope(), "__updated_at", false, None, ascending, None)
            .await?;
        let ids: Vec<&str> = entries.iter().map(|e| e.node_id.as_str()).collect();
        let expected = if ascending {
            ["n2", "n1"]
        } else {
            ["n1", "n2"]
        };
        assert_eq!(ids, expected, "ascending={ascending}");
    }
    Ok(())
}

#[tokio::test]
async fn updated_at_range_lower_and_upper_bounds() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    for id in ["n1", "n2", "n3"] {
        put(&storage, &node(id, "raisin:Folder", &[])).await?;
    }
    let mut stamps = Vec::new();
    for id in ["n1", "n2", "n3"] {
        let n = storage.nodes().get(scope(), id, None).await?.unwrap();
        stamps.push(PropertyValue::Date(n.updated_at.expect("stamped").into()));
    }
    let range = |lower: Option<(usize, bool)>, upper: Option<(usize, bool)>| {
        let storage = &storage;
        let stamps = &stamps;
        async move {
            let entries = storage
                .property_index()
                .scan_property_range(
                    scope(),
                    "__updated_at",
                    lower.map(|(i, inc)| (&stamps[i], inc)),
                    upper.map(|(i, inc)| (&stamps[i], inc)),
                    false,
                    None,
                    true,
                    None,
                )
                .await
                .unwrap();
            entries.into_iter().map(|e| e.node_id).collect::<Vec<_>>()
        }
    };

    assert_eq!(range(Some((1, true)), None).await, vec!["n2", "n3"]);
    assert_eq!(range(Some((1, false)), None).await, vec!["n3"]);
    assert_eq!(range(None, Some((1, true))).await, vec!["n1", "n2"]);
    assert_eq!(range(None, Some((1, false))).await, vec!["n1"]);
    assert_eq!(range(Some((0, false)), Some((2, false))).await, vec!["n2"]);

    // Shadowing: once n1 is updated, its old timestamp no longer matches.
    let before = storage.branches().get_head(TENANT, REPO, BRANCH).await?;
    put(&storage, &node("n1", "raisin:Folder", &[("v", "2")])).await?;
    assert_eq!(
        range(Some((0, true)), Some((0, true))).await,
        Vec::<String>::new()
    );
    let historical = storage
        .property_index()
        .scan_property_range(
            scope(),
            "__updated_at",
            Some((&stamps[0], true)),
            Some((&stamps[0], true)),
            false,
            Some(&before),
            true,
            None,
        )
        .await?;
    assert_eq!(
        historical.len(),
        1,
        "as of before the update, n1 still matches"
    );
    Ok(())
}

/// Write a LIVE `__node_type` entry that contradicts the node — what an old
/// writer that forgot a tombstone leaves behind.
fn plant_orphan(storage: &RocksDBStorage, value: &str, node_id: &str, at: &HLC) {
    let key = keys::property_index_key_versioned(
        TENANT,
        REPO,
        BRANCH,
        WORKSPACE,
        "__node_type",
        value,
        at,
        node_id,
        false,
    );
    let db = storage.db();
    db.put_cf(
        db.cf_handle(cf::PROPERTY_INDEX).unwrap(),
        key,
        node_id.as_bytes(),
    )
    .unwrap();
}

#[tokio::test]
async fn list_by_type_after_type_change() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    let r1 = put(&storage, &node("a", "test:Old", &[])).await?;
    let r2 = put(&storage, &node("a", "test:New", &[])).await?;
    // Even with an orphan `test:Old` entry live at HEAD.
    plant_orphan(&storage, "test:Old", "a", &r2);

    let ids = |nodes: Vec<Node>| nodes.into_iter().map(|n| n.id).collect::<Vec<_>>();
    let nodes = storage.nodes();
    assert!(ids(nodes
        .list_by_type(scope(), "test:Old", ListOptions::for_sql())
        .await?)
    .is_empty());
    assert_eq!(
        ids(nodes
            .list_by_type(scope(), "test:New", ListOptions::for_sql())
            .await?),
        vec!["a"]
    );
    assert_eq!(
        ids(nodes
            .list_by_type(scope(), "test:Old", ListOptions::at_revision(r1))
            .await?),
        vec!["a"],
        "as of r1 the node was still test:Old"
    );
    Ok(())
}

#[tokio::test]
async fn orphan_detector_reports_index_entries_the_blobs_contradict() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    put(&storage, &node("a", "test:Real", &[])).await?;
    let head = put(&storage, &node("b", "test:Real", &[])).await?;
    assert!(
        detect_property_index_orphans(&storage, scope(), "__node_type", false, 10)
            .await?
            .is_empty(),
        "a clean index reports nothing"
    );

    plant_orphan(&storage, "test:Ghost", "a", &head);
    plant_orphan(&storage, "test:Real", "missing-node", &head);

    let mut orphans =
        detect_property_index_orphans(&storage, scope(), "__node_type", false, 10).await?;
    orphans.sort_by(|x, y| x.node_id.cmp(&y.node_id));
    assert_eq!(orphans.len(), 2, "{orphans:?}");
    assert_eq!(orphans[0].node_id, "a");
    assert_eq!(orphans[0].indexed_value, "test:Ghost");
    assert_eq!(
        orphans[0].reason,
        PropertyIndexOrphanReason::ValueMismatch {
            actual: vec!["test:Real".to_string()]
        }
    );
    assert_eq!(orphans[1].node_id, "missing-node");
    assert_eq!(orphans[1].reason, PropertyIndexOrphanReason::NodeMissing);

    let capped = detect_property_index_orphans(&storage, scope(), "__node_type", false, 1).await?;
    assert_eq!(capped.len(), 1, "the limit caps the report");
    Ok(())
}
