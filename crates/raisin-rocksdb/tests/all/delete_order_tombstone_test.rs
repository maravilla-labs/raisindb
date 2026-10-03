//! A delete tombstones the child's ORDERED_CHILDREN entry under the parent's
//! ID and the label actually stored — locally and on a replica.
//!
//! The tombstoner keyed it by `Node.parent`, which is the parent's NAME, so
//! the entry under the real parent id stayed live: `has_children` reported a
//! parent of deleted children as still having children, and a cursor listing
//! kept the deleted child's slot. The replicated delete fed the same NAME
//! whenever the peer's node carried one, so a fix to the tombstoner alone
//! would never have reached replicas. Both are read RAW here, so the
//! `has_children` probe's own liveness check cannot mask a missing tombstone.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{cf, keys, RocksDBConfig, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, RegistryRepository,
    RepoScope, RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "del-tenant";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

/// Children of `parent_id` whose newest entry at or below `at` is live.
fn live_children(storage: &RocksDBStorage, parent_id: &str, at: Option<HLC>) -> HashSet<String> {
    let prefix = keys::ordered_children_prefix(TENANT, REPO, BRANCH, WORKSPACE, parent_id);
    let db = storage.db();
    let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
    let mut decided = HashSet::new();
    let mut live = HashSet::new();
    for item in db.prefix_iterator_cf(cf, &prefix) {
        let (key, value) = item.unwrap();
        if !key.starts_with(&prefix) {
            break;
        }
        let suffix = &key[prefix.len()..];
        let Some(label_end) = suffix.iter().position(|b| *b == 0) else {
            continue;
        };
        let child_start = label_end + 18;
        if suffix.len() <= child_start {
            continue;
        }
        let rev = HLC::decode_descending(&suffix[label_end + 1..label_end + 17]).unwrap();
        if at.is_some_and(|at| rev > at) {
            continue;
        }
        let pair = (suffix[..label_end].to_vec(), suffix[child_start..].to_vec());
        if decided.insert(pair.clone()) && !keys::is_tombstone_value(&value) {
            live.insert(String::from_utf8_lossy(&pair.1).into_owned());
        }
    }
    live
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

async fn create(storage: &RocksDBStorage, id: &str, path: &str) -> Result<()> {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = path
        .rsplitn(2, '/')
        .nth(1)
        .filter(|p| !p.is_empty())
        .map(|p| p.rsplit('/').next().unwrap().to_string());
    let node = Node {
        id: id.to_string(),
        path: path.to_string(),
        name,
        parent, // the parent's NAME, as the model defines it
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    storage.nodes().create(scope(), node, options).await
}

#[tokio::test]
async fn has_children_after_last_nested_child_deleted() -> Result<()> {
    let (storage, _tmp) = setup().await?;
    create(&storage, "a-id", "/a").await?;
    create(&storage, "b-id", "/a/b").await?;
    create(&storage, "c-id", "/a/b/c").await?;
    let before = storage.branches().get_head(TENANT, REPO, BRANCH).await?;

    storage
        .nodes()
        .delete(scope(), "c-id", DeleteNodeOptions::default())
        .await?;

    assert!(
        !live_children(&storage, "b-id", None).contains("c-id"),
        "the entry under the parent's ID is tombstoned"
    );
    assert!(live_children(&storage, "b-id", Some(before)).contains("c-id"));

    let nodes = storage.nodes();
    assert!(!nodes.has_children(scope(), "b-id", None).await?);
    assert!(nodes.has_children(scope(), "b-id", Some(&before)).await?);
    let page = nodes
        .list_ordered_children_page(scope(), "b-id", None, Some(10), false, None)
        .await?;
    assert!(
        page.is_empty(),
        "no slot left for the deleted child ({} entries)",
        page.len()
    );
    Ok(())
}

fn op(revision: HLC, seq: u64, changes: Vec<ReplicatedNodeChange>) -> Operation {
    Operation {
        op_id: Uuid::new_v4(),
        op_seq: seq,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision,
            node_changes: changes,
        },
        revision: Some(revision),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        agent: None,
        acknowledged_by: Default::default(),
    }
}

fn change(node: &Node, parent_id: &str, kind: ReplicatedNodeChangeKind) -> ReplicatedNodeChange {
    ReplicatedNodeChange {
        node: node.clone(),
        parent_id: Some(parent_id.to_string()),
        kind,
        cf_order_key: format!("{}::{}", node.order_key, node.id),
    }
}

#[tokio::test]
async fn replicated_delete_tombstones_by_parent_id() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;
    let storage = Arc::new(RocksDBStorage::with_config(config)?);
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await;
    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    // The parent's id and name differ, so a tombstone keyed by the name
    // misses the entry.
    let node = |id: &str, name: &str, path: &str, parent: &str| Node {
        id: id.to_string(),
        name: name.to_string(),
        path: path.to_string(),
        node_type: "Article".to_string(),
        order_key: "a".to_string(),
        parent: Some(parent.to_string()),
        created_at: Some(chrono::Utc::now()),
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        ..Node::default()
    };
    let parent = node("parent-id", "parent", "/parent", "/");
    let child = node("child-id", "child", "/parent/child", "parent");

    applicator
        .apply_operation(&op(
            HLC::new(5, 0),
            1,
            vec![
                change(&parent, "/", ReplicatedNodeChangeKind::Upsert),
                change(&child, "parent-id", ReplicatedNodeChangeKind::Upsert),
            ],
        ))
        .await
        .expect("replicated create");
    assert!(live_children(&storage, "parent-id", None).contains("child-id"));

    applicator
        .apply_operation(&op(
            HLC::new(20, 0),
            2,
            vec![change(
                &child,
                "parent-id",
                ReplicatedNodeChangeKind::Delete,
            )],
        ))
        .await
        .expect("replicated delete");

    assert!(
        !live_children(&storage, "parent-id", None).contains("child-id"),
        "the replica tombstoned the entry under the parent's ID"
    );
    assert!(live_children(&storage, "parent-id", Some(HLC::new(19, 0))).contains("child-id"));
    Ok(())
}
