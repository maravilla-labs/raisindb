//! A replica's compound indexes fail closed after a replicated write.
//!
//! The replication apply path writes no COMPOUND_INDEX entries (Phase 8 step
//! 3), so a replicated upsert leaves this node's compound keyspace behind its
//! node records. Before this, the state record still said `Ready`, and the
//! planner served the stale keyspace — a moved or edited node answered by its
//! OLD column values. Now the apply marks the workspace's indexes `NotBuilt`
//! (the planner scans) until a local build re-earns `Ready`.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::compound_state::CompoundStateStore;
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::compound::{CompoundAvailability, CompoundIndexState};
use raisin_storage::{
    BranchRepository, RegistryRepository, RepoScope, RepositoryManagementRepository, Storage,
    WorkspaceRepository,
};
use tempfile::TempDir;

const TENANT: &str = "rc-tenant";
const REPO: &str = "rc-repo";
const BRANCH: &str = "main";
const WS: &str = "content";

fn definition() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: "by_cat".to_string(),
        columns: vec![CompoundIndexColumn {
            property: "cat".to_string(),
            ascending: None,
            column_type: CompoundColumnType::String,
        }],
        has_order_column: false,
        owner_node_type: None,
    }
}

async fn setup() -> (Arc<RocksDBStorage>, TempDir) {
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
    (Arc::new(storage), temp_dir)
}

fn availability(storage: &RocksDBStorage) -> CompoundAvailability {
    storage
        .compound_state()
        .expect("compound state source")
        .compound_availability(TENANT, REPO, BRANCH, WS, &definition())
}

fn replicated_upsert(node: &Node, revision: HLC) -> Operation {
    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision,
            node_changes: vec![ReplicatedNodeChange {
                node: node.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: format!("a0::{}", node.id),
            }],
        },
        revision: Some(revision),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        agent: None,
        acknowledged_by: Default::default(),
    }
}

#[tokio::test]
async fn replica_compound_query_falls_back_until_rebuilt() {
    let (storage, _dir) = setup().await;
    let store = CompoundStateStore::new(storage.db().clone());
    let head = storage
        .branches()
        .get_head(TENANT, REPO, BRANCH)
        .await
        .unwrap();

    // This node built the index.
    store
        .put(
            TENANT,
            REPO,
            BRANCH,
            WS,
            &CompoundIndexState::ready(&definition(), head),
        )
        .unwrap();
    assert!(matches!(
        availability(&storage),
        CompoundAvailability::Ready { .. }
    ));

    // A peer's write arrives through replication.
    let node = Node {
        id: "n1".to_string(),
        name: "n1".to_string(),
        path: "/n1".to_string(),
        node_type: "test:Item".to_string(),
        properties: HashMap::from([("cat".to_string(), PropertyValue::String("b".to_string()))]),
        workspace: Some(WS.to_string()),
        order_key: "a0".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );
    applicator
        .apply_operation(&replicated_upsert(
            &node,
            HLC::new(head.timestamp_ms + 1, 0),
        ))
        .await
        .expect("apply");

    // The keyspace no longer matches the records: the planner must scan.
    assert!(
        matches!(availability(&storage), CompoundAvailability::NotBuilt),
        "a replicated write must fail the replica's compound index closed: {:?}",
        availability(&storage)
    );

    // Until a local build — started after the write — re-earns Ready.
    let started = store
        .begin_build(TENANT, REPO, BRANCH, WS, &definition(), head)
        .unwrap();
    assert!(store
        .complete_build(
            TENANT,
            REPO,
            BRANCH,
            WS,
            CompoundIndexState::ready(&definition(), head),
            started,
        )
        .unwrap());
    assert!(matches!(
        availability(&storage),
        CompoundAvailability::Ready { .. }
    ));
}
