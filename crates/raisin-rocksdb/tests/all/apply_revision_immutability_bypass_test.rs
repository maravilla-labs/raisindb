//! Regression: the replication apply path deliberately does NOT re-check
//! `NodeType.immutable` — a peer already accepted the write under its own
//! policy before capturing it for replication, and there is no rollback
//! story for rejecting an already-committed revision mid-apply. This is the
//! immutability analog of the documented secrets invariant ("the replication
//! apply path must not re-vault... it does not go through `put_node`, so
//! this is free"). See `crate::immutability` and CLAUDE.md's `immutable`
//! section.
//!
//! This does not test that replication is *unsafe* — it documents a known,
//! deliberate trust boundary: a compromised or buggy peer can still
//! overwrite an immutable node via replication. If this test starts
//! failing because apply now rejects the second write, that is a real
//! behavior change and needs a conscious decision, not a silent regression.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, Storage,
};
use tempfile::TempDir;

const TENANT: &str = "tenant1";
const REPO: &str = "repo1";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";
const IMMUTABLE_TYPE: &str = "test:Ledger";

fn cf_key(node: &Node) -> String {
    format!("{}::{}", node.order_key, node.id)
}

fn apply_op(revision: HLC, node: &Node) -> Operation {
    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision.clone(),
            node_changes: vec![ReplicatedNodeChange {
                node: node.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: cf_key(node),
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
async fn replication_apply_bypasses_immutable_check() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;

    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());

    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await;

    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({
                "name": IMMUTABLE_TYPE,
                "immutable": true,
            }))
            .expect("nodetype json"),
            CommitMetadata {
                message: "seed immutable type".to_string(),
                actor: "system".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("create nodetype");

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String("original".to_string()),
    );

    let mut node = Node {
        id: "ledger-1".to_string(),
        name: "ledger-1".to_string(),
        path: "/ledger-1".to_string(),
        node_type: IMMUTABLE_TYPE.to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: "a".to_string(),
        has_children: None,
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("user1".to_string()),
        created_by: Some("user1".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    };

    // First apply: a create, at revision 10. Would also succeed through the
    // normal transaction path (immutable only blocks a SUBSEQUENT change).
    applicator
        .apply_operation(&apply_op(HLC::new(10, 0), &node))
        .await
        .expect("first apply (create) must succeed");

    // Second apply: same node id, DIFFERENT properties, at a later revision —
    // simulating a replicated property-changing update from a peer. Through
    // the normal write path (`put_node`/`update_impl`) this would be
    // rejected by `crate::immutability::reject_if_immutable`. On the apply
    // path it must NOT be — this is the behavior under test.
    node.version = 2;
    node.properties.insert(
        "title".to_string(),
        PropertyValue::String("changed-by-peer".to_string()),
    );
    applicator
        .apply_operation(&apply_op(HLC::new(20, 0), &node))
        .await
        .expect("second apply (property change) must NOT be rejected on the apply path");

    let stored = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &node.id,
            None,
        )
        .await
        .unwrap()
        .expect("node must exist");
    assert_eq!(
        stored.properties.get("title"),
        Some(&PropertyValue::String("changed-by-peer".to_string())),
        "the replicated property change must actually have landed, not been silently dropped"
    );
}
