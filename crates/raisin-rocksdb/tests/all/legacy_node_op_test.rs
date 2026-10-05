//! Plan "Phase 11d" (owner decision 2026-10-04: no cluster runs a pre-v2
//! binary): the pre-v2 granular node ops — `create_node`, `delete_node`,
//! `set_property`, `delete_property`, `rename_node`, `set_archetype`,
//! `set_order_key`, `set_owner`, `publish_node`, `unpublish_node`,
//! `move_node`, `list_insert_after`, `list_delete` — are deleted from
//! `OpType` with their apply arms and builders. `OpType` is serialized by
//! variant NAME, so an entry of one already in a saved oplog (or a
//! checkpoint's `operation_log` CF) decodes as `OpType::Unknown`, is skipped,
//! and never stalls the ops after it.
//!
//! Every op below would CHANGE the target node if it were applied (delete
//! it, retitle it, move it, rename it, ...), so the node's stored state is
//! the check: the receive path (`put_operations_batch`) only logs an apply
//! error, so "nothing stalled" is asserted on the state, not on a result.

use crate::legacy_translation_op_test::{legacy_bytes, persist_raw, OLD_PEER};
use crate::translation_replication_test::{node, peer_op, receive};
use crate::translation_substrate_test::{rev, B, R, T, WS};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind};
use raisin_replication::{OpType, Operation, ReplayEngine, ReplicationMessage, VectorClock};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::OpLogRepository;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde_json::json;
use std::sync::Arc;

const TARGET: &str = "legacy-target";

/// `(tag, content)` of every removed node op, field for field as the last
/// binary that had them serialized each one, all aimed at [`TARGET`].
fn removed_node_ops() -> Vec<(&'static str, serde_json::Value)> {
    let element = "6f1c1d2e-0000-4000-8000-000000000001";
    vec![
        (
            "create_node",
            json!({
                "node_id": "legacy-created", "name": "legacy-created",
                "node_type": "raisin:Folder", "archetype": null, "parent_id": null,
                "order_key": "z0", "properties": {}, "owner_id": null,
                "workspace": WS, "path": "/legacy-created"
            }),
        ),
        ("delete_node", json!({ "node_id": TARGET })),
        (
            "set_property",
            json!({ "node_id": TARGET, "property_name": "title", "value": "stale" }),
        ),
        (
            "delete_property",
            json!({ "node_id": TARGET, "property_name": "title" }),
        ),
        (
            "rename_node",
            json!({ "node_id": TARGET, "old_name": TARGET, "new_name": "renamed" }),
        ),
        (
            "set_archetype",
            json!({ "node_id": TARGET, "old_archetype": null, "new_archetype": "hero" }),
        ),
        (
            "set_order_key",
            json!({ "node_id": TARGET, "old_order_key": "a0", "new_order_key": "z9" }),
        ),
        (
            "set_owner",
            json!({ "node_id": TARGET, "old_owner_id": null, "new_owner_id": "mallory" }),
        ),
        (
            "publish_node",
            json!({ "node_id": TARGET, "published_by": "mallory", "published_at": 1000 }),
        ),
        ("unpublish_node", json!({ "node_id": TARGET })),
        (
            "move_node",
            json!({
                "node_id": TARGET, "old_parent_id": null,
                "new_parent_id": "elsewhere", "position": "a0"
            }),
        ),
        (
            "list_insert_after",
            json!({
                "node_id": TARGET, "list_property": "items", "after_id": null,
                "value": "x", "element_id": element
            }),
        ),
        (
            "list_delete",
            json!({ "node_id": TARGET, "list_property": "items", "element_id": element }),
        ),
    ]
}

fn folder(id: &str, title: &str) -> Node {
    let mut node = Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        parent: Some("/".to_string()),
        node_type: "raisin:Folder".to_string(),
        order_key: format!("a0::{id}"),
        workspace: Some(WS.to_string()),
        tenant_id: Some(T.to_string()),
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    node.properties
        .insert("title".to_string(), PropertyValue::String(title.into()));
    node
}

/// A current node write at `at`, as every commit replicates it.
fn upsert(node: &Node, at: raisin_hlc::HLC) -> OpType {
    OpType::ApplyRevision {
        branch_head: at,
        node_changes: vec![ReplicatedNodeChange {
            node: node.clone(),
            parent_id: Some("/".to_string()),
            kind: ReplicatedNodeChangeKind::Upsert,
            cf_order_key: node.order_key.clone(),
        }],
    }
}

async fn stored(storage: &raisin_rocksdb::RocksDBStorage, id: &str) -> Option<Node> {
    storage
        .nodes()
        .get(StorageScope::new(T, R, B, WS), id, None)
        .await
        .unwrap()
}

/// The target exactly as created: none of the removed ops touched it.
async fn assert_untouched(storage: &raisin_rocksdb::RocksDBStorage) {
    let target = stored(storage, TARGET)
        .await
        .expect("a removed delete_node deleted the target");
    assert_eq!(target.path, format!("/{TARGET}"), "moved or renamed");
    assert_eq!(target.name, TARGET, "renamed");
    assert_eq!(
        target.properties.get("title"),
        Some(&PropertyValue::String("original".into())),
        "retitled or property deleted"
    );
    assert_eq!(target.archetype, None, "archetype set");
    assert_eq!(target.published_at, None, "published");
    assert!(target.properties.get("items").is_none(), "list op applied");
    assert!(
        stored(storage, "legacy-created").await.is_none(),
        "a removed create_node created a node"
    );
}

#[tokio::test]
async fn every_removed_legacy_node_op_in_a_saved_oplog_is_skipped() {
    let replica = node("replica").await;
    let db = replica.storage.db().clone();
    let oplog = OpLogRepository::new(db.clone());
    let applicator = OperationApplicator::new(
        db.clone(),
        replica.storage.event_bus(),
        Arc::new(replica.storage.branches_impl().clone()),
    );

    // The target, written the current way (not through the oplog).
    let mut create = peer_op(0, upsert(&folder(TARGET, "original"), rev(0)));
    create.cluster_node_id = "origin".to_string();
    applicator.apply_operation(&create).await.unwrap();
    assert_untouched(&replica.storage).await;

    // Every removed op, byte for byte as a pre-removal binary persisted it.
    let removed = removed_node_ops();
    let raw: Vec<Vec<u8>> = removed
        .iter()
        .enumerate()
        .map(|(i, (tag, content))| legacy_bytes(i as u64 + 1, tag, content.clone()))
        .collect();
    for (i, bytes) in raw.iter().enumerate() {
        persist_raw(&oplog, &db, i as u64 + 1, bytes);
    }

    // Both oplog readers decode them, as Unknown, verbatim.
    let mut saved = oplog.get_operations_from_node(T, R, OLD_PEER).unwrap();
    saved.sort_by_key(|op| op.op_seq);
    assert_eq!(saved.len(), removed.len());
    let missing = oplog
        .get_missing_operations(T, R, &VectorClock::new(), None)
        .unwrap();
    assert_eq!(
        missing
            .iter()
            .filter(|op| op.cluster_node_id == OLD_PEER)
            .count(),
        removed.len()
    );
    for ((op, (tag, _)), bytes) in saved.iter().zip(&removed).zip(&raw) {
        assert!(
            matches!(&op.op_type, OpType::Unknown { tag: t, .. } if t == tag),
            "{tag}: {:?}",
            op.op_type
        );
        let a: rmpv::Value = rmp_serde::from_slice(bytes).unwrap();
        let b: rmpv::Value = rmp_serde::from_slice(&rmp_serde::to_vec_named(op).unwrap()).unwrap();
        assert_eq!(a, b, "{tag}: re-persisting it loses nothing");
    }

    // The applier skips each one without an error and changes nothing.
    for op in &saved {
        applicator.apply_operation(op).await.unwrap();
    }
    assert_untouched(&replica.storage).await;

    // Through the codec, the replay engine and the production receive path,
    // with a current op from the same peer after them: they stay Unknown,
    // still change nothing, and the current op lands.
    let next = removed.len() as u64 + 1;
    let mut current = peer_op(next, upsert(&folder("after", "current"), rev(next)));
    current.cluster_node_id = OLD_PEER.to_string();
    let current_id = current.op_id;
    let mut batch: Vec<Operation> = saved.clone();
    batch.push(current);
    let bytes = ReplicationMessage::PushOperations { operations: batch }
        .to_bytes()
        .unwrap();
    let ReplicationMessage::PushOperations { operations: batch } =
        ReplicationMessage::from_bytes(&bytes).expect("a batch with removed ops must decode")
    else {
        panic!("wrong message");
    };
    assert_eq!(
        batch
            .iter()
            .filter(|op| matches!(op.op_type, OpType::Unknown { .. }))
            .count(),
        removed.len(),
        "the codec keeps them Unknown"
    );
    let replayed = ReplayEngine::new().replay(batch.clone());
    assert!(
        replayed.applied.iter().any(|op| op.op_id == current_id),
        "the replay engine keeps the current op"
    );
    receive(&replica, &batch).await;

    assert_untouched(&replica.storage).await;
    let after = stored(&replica.storage, "after")
        .await
        .expect("the op after the removed ones was applied");
    assert_eq!(
        after.properties.get("title"),
        Some(&PropertyValue::String("current".into()))
    );
    assert_eq!(
        oplog.get_vector_clock_snapshot(T, R).unwrap().get(OLD_PEER),
        next,
        "the clock moves past the removed ops"
    );
}
