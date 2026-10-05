//! Plan Phase 11b (owner decision 2026-10-04): the pre-Phase-11
//! `set_translation` / `delete_translation` ops are deleted from `OpType`
//! outright — no apply arm, no builder, no emission flag. `OpType` is
//! serialized by variant NAME, so no positional placeholder is needed; what
//! must still hold is that an entry of a removed op already sitting in a
//! saved oplog (or arriving in a checkpoint's `operation_log` CF) decodes as
//! `OpType::Unknown`, is skipped, and never stalls the ops after it. The
//! removed pre-v2 NODE ops have the sibling `legacy_node_op_test`.

use crate::translation_replication_test::{node, peer_op, receive, v2, NODE};
use crate::translation_substrate_test::{get, rev, title};
use raisin_replication::{
    OpType, Operation, ReplayEngine, ReplicatedOverlay, ReplicationMessage, VectorClock,
};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{cf, OpLogRepository};
use raisin_storage::Storage;
use std::sync::Arc;

use crate::translation_substrate_test::{B, R, T};

pub(crate) const OLD_PEER: &str = "old-peer";

/// The op exactly as a pre-Phase-11 binary wrote it into its oplog: the
/// named-msgpack encoding of an `Operation` whose op_type is the removed
/// variant.
pub(crate) fn legacy_bytes(seq: u64, tag: &str, content: serde_json::Value) -> Vec<u8> {
    let mut op = Operation::new(
        seq,
        OLD_PEER.to_string(),
        VectorClock::new(),
        T.to_string(),
        R.to_string(),
        B.to_string(),
        OpType::DeleteNodeSnapshot {
            node_id: "placeholder".into(),
            revision: rev(seq),
            node: None,
            parent_id: None,
        },
        "old-user".to_string(),
    );
    op.revision = Some(rev(seq));
    let mut value: rmpv::Value =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&op).unwrap()).unwrap();
    let legacy: rmpv::Value = rmp_serde::from_slice(
        &rmp_serde::to_vec_named(&serde_json::json!({ tag: content })).unwrap(),
    )
    .unwrap();
    let rmpv::Value::Map(entries) = &mut value else {
        panic!("an operation is a map");
    };
    for (key, entry) in entries.iter_mut() {
        if key.as_str() == Some("op_type") {
            *entry = legacy.clone();
        }
    }
    rmp_serde::to_vec_named(&value).unwrap()
}

/// Writes `bytes` into the oplog under the key the old binary used for that
/// op: the key of a placeholder op with the same identity, value replaced.
pub(crate) fn persist_raw(oplog: &OpLogRepository, db: &rocksdb::DB, seq: u64, bytes: &[u8]) {
    let placeholder: Operation = rmp_serde::from_slice(bytes).unwrap();
    oplog.put_operation(&placeholder).unwrap();
    let handle = db.cf_handle(cf::OPERATION_LOG).unwrap();
    let key = db
        .iterator_cf(handle, rocksdb::IteratorMode::Start)
        .map(|item| item.unwrap())
        .find(|(_, value)| {
            rmp_serde::from_slice::<Operation>(value)
                .map(|op| op.cluster_node_id == OLD_PEER && op.op_seq == seq)
                .unwrap_or(false)
        })
        .map(|(key, _)| key)
        .expect("placeholder persisted");
    db.put_cf(handle, &key, bytes).unwrap();
}

#[tokio::test]
async fn a_removed_legacy_translation_op_in_a_saved_oplog_is_skipped() {
    let replica = node("replica").await;
    let db = replica.storage.db().clone();
    let oplog = OpLogRepository::new(db.clone());

    let set = legacy_bytes(
        1,
        "set_translation",
        serde_json::json!({
            "node_id": NODE, "locale": "fr", "property_name": "properties",
            "value": { "/title": "Bonjour" }
        }),
    );
    let delete = legacy_bytes(
        2,
        "delete_translation",
        serde_json::json!({ "node_id": NODE, "locale": "de", "property_name": "properties" }),
    );
    persist_raw(&oplog, &db, 1, &set);
    persist_raw(&oplog, &db, 2, &delete);

    // Every oplog reader decodes them: the per-peer listing and catch-up.
    let saved = oplog.get_operations_from_node(T, R, OLD_PEER).unwrap();
    assert_eq!(saved.len(), 2);
    let missing = oplog
        .get_missing_operations(T, R, &VectorClock::new(), None)
        .unwrap();
    assert_eq!(
        missing
            .iter()
            .filter(|op| op.cluster_node_id == OLD_PEER)
            .count(),
        2
    );
    for (op, tag) in saved.iter().zip(["set_translation", "delete_translation"]) {
        assert!(
            matches!(&op.op_type, OpType::Unknown { tag: t, .. } if t == tag),
            "{tag}: {:?}",
            op.op_type
        );
        // Kept verbatim: re-persisting it loses nothing.
        let raw = if tag == "set_translation" {
            &set
        } else {
            &delete
        };
        let a: rmpv::Value = rmp_serde::from_slice(raw).unwrap();
        let b: rmpv::Value = rmp_serde::from_slice(&rmp_serde::to_vec_named(op).unwrap()).unwrap();
        assert_eq!(a, b);
    }

    // The applier skips them without an error and writes nothing. This loop is
    // the check that the `Unknown` arm returns `Ok`: the receive path below
    // only logs an apply error and moves on.
    let applicator = OperationApplicator::new(
        db.clone(),
        replica.storage.event_bus(),
        Arc::new(replica.storage.branches_impl().clone()),
    );
    for op in &saved {
        applicator.apply_operation(op).await.unwrap();
    }
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(3)).await.unwrap(),
        None
    );

    // Sent on as a peer would (the TCP message codec, the replay engine's
    // filter / causal sort / CRDT merge) and then through the production
    // receive path, with a current op after them: they stay `Unknown`
    // end to end and the current op still applies.
    let mut current = peer_op(
        3,
        v2(
            "fr",
            ReplicatedOverlay::from_stored(Some(&title("Salut"))),
            rev(3),
        ),
    );
    current.cluster_node_id = OLD_PEER.to_string();
    let current_id = current.op_id;
    let mut batch = saved.clone();
    batch.push(current);
    let bytes = ReplicationMessage::PushOperations { operations: batch }
        .to_bytes()
        .unwrap();
    let ReplicationMessage::PushOperations { operations: batch } =
        ReplicationMessage::from_bytes(&bytes).expect("a batch with removed ops must decode")
    else {
        panic!("wrong message");
    };
    let unknown = |ops: &[Operation]| {
        ops.iter()
            .filter(|op| matches!(op.op_type, OpType::Unknown { .. }))
            .count()
    };
    assert_eq!(unknown(&batch), 2, "the codec keeps them Unknown");
    let replayed = ReplayEngine::new().replay(batch.clone());
    assert!(
        replayed.applied.iter().any(|op| op.op_id == current_id),
        "the replay engine keeps the current op"
    );
    receive(&replica, &batch).await;
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(3)).await.unwrap(),
        Some(title("Salut"))
    );
    assert_eq!(
        oplog.get_vector_clock_snapshot(T, R).unwrap().get(OLD_PEER),
        3,
        "the clock moves past the removed ops"
    );
}
