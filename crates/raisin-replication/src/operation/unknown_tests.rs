//! An operation type this binary does not know decodes as `OpType::Unknown`
//! (plan Phase 11 / Operational test gates: `unknown_optype_is_skipped_not_stalled`
//! — the applier half lives in raisin-rocksdb).

use super::*;
use crate::tcp_protocol::ReplicationMessage;
use raisin_models::translations::JsonPointer;
use std::collections::HashMap;

fn sample(op_type: OpType) -> Operation {
    let mut op = Operation::new(
        3,
        "newer-peer".to_string(),
        VectorClock::new(),
        "t".to_string(),
        "r".to_string(),
        "main".to_string(),
        op_type,
        "alice".to_string(),
    );
    op.revision = Some(HLC::new(1_705_843_009_213, 2));
    op
}

/// What a NEWER peer sends: a real operation whose op_type tag this binary
/// has never seen, with a struct payload.
fn future_op_value() -> serde_json::Value {
    let mut value = serde_json::to_value(sample(OpType::DeleteNodeSnapshot {
        node_id: "n1".to_string(),
        revision: HLC::new(1_000, 0),
        node: None,
        parent_id: None,
    }))
    .unwrap();
    value["op_type"] = serde_json::json!({
        "rebalance_shards_v9": { "shard": 7, "nodes": ["a", "b"], "nested": { "x": null } }
    });
    value
}

/// The same op as the newer peer's msgpack: a real encoding (binary uuid,
/// as the oplog and the TCP protocol write it) with the op_type replaced.
fn future_op_msgpack() -> Vec<u8> {
    msgpack_with_op_type(&future_op_value()["op_type"])
}

/// A real msgpack-encoded operation whose op_type is replaced by `op_type`.
fn msgpack_with_op_type(op_type: &serde_json::Value) -> Vec<u8> {
    let op = sample(OpType::DeleteNodeSnapshot {
        node_id: "n1".to_string(),
        revision: HLC::new(1_000, 0),
        node: None,
        parent_id: None,
    });
    let mut value: rmpv::Value =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&op).unwrap()).unwrap();
    let future: rmpv::Value =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(op_type).unwrap()).unwrap();
    let rmpv::Value::Map(entries) = &mut value else {
        panic!("an operation is a map");
    };
    for (key, entry) in entries.iter_mut() {
        if key.as_str() == Some("op_type") {
            *entry = future.clone();
        }
    }
    rmp_serde::to_vec_named(&value).unwrap()
}

#[test]
fn an_unknown_op_type_decodes_from_msgpack_and_round_trips() {
    let bytes = future_op_msgpack();
    let op: Operation = rmp_serde::from_slice(&bytes).expect("unknown op must decode");
    let OpType::Unknown { tag, .. } = &op.op_type else {
        panic!("expected Unknown, got {:?}", op.op_type);
    };
    assert_eq!(tag, "rebalance_shards_v9");
    assert_eq!(op.op_seq, 3);
    assert_eq!(
        op.target(),
        OperationTarget::Unknown(format!("{tag}:{}", op.op_id))
    );

    // Persisted / forwarded by this binary, it is still the newer peer's op.
    let again = rmp_serde::to_vec_named(&op).unwrap();
    let a: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
    let b: rmpv::Value = rmp_serde::from_slice(&again).unwrap();
    assert_eq!(a, b);
}

#[test]
fn an_unknown_op_type_decodes_from_json() {
    let value = future_op_value();
    let op: Operation = serde_json::from_value(value.clone()).expect("unknown op must decode");
    assert!(matches!(&op.op_type, OpType::Unknown { tag, .. } if tag == "rebalance_shards_v9"));
    assert_eq!(
        serde_json::to_value(&op).unwrap()["op_type"],
        value["op_type"]
    );
}

/// One unknown op must not make the whole message undecodable — that was the
/// stall: the batch failed to decode and was retried forever.
#[test]
fn a_batch_with_an_unknown_op_still_decodes() {
    let known = sample(OpType::DeleteNodeSnapshot {
        node_id: "n2".to_string(),
        revision: HLC::new(1_000, 0),
        node: None,
        parent_id: None,
    });
    let unknown: Operation = serde_json::from_value(future_op_value()).unwrap();
    let message = ReplicationMessage::PushOperations {
        operations: vec![unknown, known.clone()],
    };
    let bytes = message.to_bytes().unwrap();
    let ReplicationMessage::PushOperations { operations } =
        ReplicationMessage::from_bytes(&bytes).unwrap()
    else {
        panic!("wrong message");
    };
    assert!(matches!(operations[0].op_type, OpType::Unknown { .. }));
    assert_eq!(operations[1], known);
}

#[test]
fn a_known_variant_with_malformed_content_is_still_an_error() {
    let mut value = future_op_value();
    value["op_type"] = serde_json::json!({ "delete_node_snapshot": { "node_id": 42 } });
    assert!(serde_json::from_value::<Operation>(value).is_err());
}

#[test]
fn every_translation_op_round_trips() {
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        raisin_models::nodes::properties::PropertyValue::String("Bonjour".into()),
    );
    for op_type in [
        OpType::UpsertTranslationOverlay {
            workspace: "ws".into(),
            node_id: "n1".into(),
            locale: "fr".into(),
            block_uuid: None,
            overlay: ReplicatedOverlay::Properties { data },
            revision: HLC::new(10, 1),
            history_complete_from: None,
        },
        OpType::UpsertTranslationOverlay {
            workspace: "ws".into(),
            node_id: "n1".into(),
            locale: "fr".into(),
            block_uuid: Some("b-1".into()),
            overlay: ReplicatedOverlay::Hidden,
            revision: HLC::new(11, 0),
            history_complete_from: Some(HLC::new(5, 0)),
        },
        OpType::UpsertTranslationOverlay {
            workspace: "ws".into(),
            node_id: "n1".into(),
            locale: "fr".into(),
            block_uuid: None,
            overlay: ReplicatedOverlay::Deleted,
            revision: HLC::new(12, 0),
            history_complete_from: None,
        },
    ] {
        let op = sample(op_type);
        let bytes = rmp_serde::to_vec_named(&op).unwrap();
        assert_eq!(rmp_serde::from_slice::<Operation>(&bytes).unwrap(), op);
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(serde_json::from_value::<Operation>(json).unwrap(), op);
    }
}

/// The pre-Phase-11 translation ops are gone from `OpType`; one still sitting
/// in a saved oplog decodes as `Unknown` (and is skipped), never as an error
/// that stalls the log.
#[test]
fn a_removed_legacy_translation_op_decodes_as_unknown() {
    for (tag, content) in [
        (
            "delete_translation",
            serde_json::json!({ "node_id": "n1", "locale": "fr", "property_name": "properties" }),
        ),
        (
            "set_translation",
            serde_json::json!({
                "node_id": "n1", "locale": "fr", "property_name": "properties",
                "value": { "/title": "Bonjour" }
            }),
        ),
    ] {
        let mut value = future_op_value();
        value["op_type"] = serde_json::json!({ tag: content });
        let op: Operation = serde_json::from_value(value).unwrap();
        assert!(
            matches!(&op.op_type, OpType::Unknown { tag: t, .. } if t == tag),
            "{tag}: {:?}",
            op.op_type
        );
    }
}

/// The pre-v2 granular node ops are gone from `OpType` too (plan "Phase
/// 11d"): every one of them, as a pre-removal binary encoded it, decodes as
/// `Unknown` — from JSON and from the named msgpack the oplog and the wire
/// use — and survives a re-encode unchanged.
#[test]
fn every_removed_legacy_node_op_decodes_as_unknown() {
    for (tag, content) in removed_node_ops() {
        let mut value = future_op_value();
        value["op_type"] = serde_json::json!({ tag: content });
        let op: Operation = serde_json::from_value(value.clone()).unwrap();
        assert!(
            matches!(&op.op_type, OpType::Unknown { tag: t, .. } if t == tag),
            "{tag} (json): {:?}",
            op.op_type
        );
        assert_eq!(
            serde_json::to_value(&op).unwrap()["op_type"],
            value["op_type"]
        );

        let bytes = msgpack_with_op_type(&value["op_type"]);
        let op: Operation = rmp_serde::from_slice(&bytes).unwrap();
        assert!(
            matches!(&op.op_type, OpType::Unknown { tag: t, .. } if t == tag),
            "{tag} (msgpack): {:?}",
            op.op_type
        );
    }
}

/// `(tag, content)` of every removed node op, field for field as the last
/// binary that had them serialized each one.
fn removed_node_ops() -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    vec![
        (
            "create_node",
            json!({
                "node_id": "n1", "name": "page", "node_type": "Page", "archetype": null,
                "parent_id": null, "order_key": "a0", "properties": { "title": "Hi" },
                "owner_id": null, "workspace": "content", "path": "/page"
            }),
        ),
        ("delete_node", json!({ "node_id": "n1" })),
        (
            "set_property",
            json!({ "node_id": "n1", "property_name": "title", "value": "Hi" }),
        ),
        (
            "delete_property",
            json!({ "node_id": "n1", "property_name": "title" }),
        ),
        (
            "rename_node",
            json!({ "node_id": "n1", "old_name": "a", "new_name": "b" }),
        ),
        (
            "set_archetype",
            json!({ "node_id": "n1", "old_archetype": null, "new_archetype": "hero" }),
        ),
        (
            "set_order_key",
            json!({ "node_id": "n1", "old_order_key": "a0", "new_order_key": "a1" }),
        ),
        (
            "set_owner",
            json!({ "node_id": "n1", "old_owner_id": null, "new_owner_id": "u1" }),
        ),
        (
            "publish_node",
            json!({ "node_id": "n1", "published_by": "u1", "published_at": 1000 }),
        ),
        ("unpublish_node", json!({ "node_id": "n1" })),
        (
            "move_node",
            json!({
                "node_id": "n1", "old_parent_id": null, "new_parent_id": "p2", "position": "a0"
            }),
        ),
        (
            "list_insert_after",
            json!({
                "node_id": "n1", "list_property": "items", "after_id": null, "value": "x",
                "element_id": "6f1c1d2e-0000-4000-8000-000000000001"
            }),
        ),
        (
            "list_delete",
            json!({
                "node_id": "n1", "list_property": "items",
                "element_id": "6f1c1d2e-0000-4000-8000-000000000001"
            }),
        ),
    ]
}
