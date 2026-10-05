use super::*;
use crate::vector_clock::VectorClock;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;

fn snapshot(node_id: &str) -> OpType {
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String("Test".to_string()),
    );
    OpType::UpsertNodeSnapshot {
        node: Node {
            id: node_id.to_string(),
            name: "test-article".to_string(),
            path: "/test-article".to_string(),
            node_type: "article".to_string(),
            workspace: Some("content".to_string()),
            properties,
            ..Default::default()
        },
        parent_id: Some("/".to_string()),
        revision: HLC::new(1_000, 0),
        cf_order_key: "a0::test".to_string(),
    }
}

fn op(seq: u64, op_type: OpType) -> Operation {
    let mut vc = VectorClock::new();
    vc.increment("node1");
    Operation::new(
        seq,
        "node1".to_string(),
        vc,
        "t1".to_string(),
        "r1".to_string(),
        "main".to_string(),
        op_type,
        "user@example.com".to_string(),
    )
}

#[test]
fn test_operation_creation() {
    let op = op(1, snapshot("test123"));

    assert_eq!(op.op_seq, 1);
    assert_eq!(op.cluster_node_id, "node1");
    assert_eq!(op.target(), OperationTarget::Node("test123".to_string()));
}

#[test]
fn test_operation_target() {
    let op = op(
        1,
        OpType::DeleteNodeSnapshot {
            node_id: "node123".to_string(),
            revision: HLC::new(1_000, 0),
            node: None,
            parent_id: None,
        },
    );

    assert_eq!(op.target(), OperationTarget::Node("node123".to_string()));
}

#[test]
fn test_is_delete() {
    let delete_op = op(
        1,
        OpType::DeleteNodeSnapshot {
            node_id: "node123".to_string(),
            revision: HLC::new(1_000, 0),
            node: None,
            parent_id: None,
        },
    );
    assert!(delete_op.is_delete());

    assert!(!op(2, snapshot("node456")).is_delete());
}

#[test]
fn test_acknowledgment() {
    let mut op = op(1, snapshot("test"));

    assert!(!op.acknowledged_by_all(&["peer1".to_string(), "peer2".to_string()]));

    op.acknowledge("peer1");
    assert!(!op.acknowledged_by_all(&["peer1".to_string(), "peer2".to_string()]));

    op.acknowledge("peer2");
    assert!(op.acknowledged_by_all(&["peer1".to_string(), "peer2".to_string()]));
}

#[test]
fn test_optype_msgpack_debug() {
    let op_type = snapshot("test-1");

    // Serialize to MessagePack (named, as the oplog and the wire encode it)
    let bytes = rmp_serde::to_vec_named(&op_type).unwrap();
    let roundtrip: OpType = rmp_serde::from_slice(&bytes).unwrap();

    if let OpType::UpsertNodeSnapshot { node, .. } = roundtrip {
        assert_eq!(node.properties.len(), 1);
    } else {
        panic!("Expected UpsertNodeSnapshot variant");
    }
}

#[test]
fn test_update_nodetype_serialization() {
    use raisin_models::nodes::types::node_type::NodeType;

    // Create a NodeType with various fields populated (mimicking raisin_asset.yaml)
    let node_type = NodeType {
        id: Some("media_asset_id".to_string()),
        strict: Some(false),
        name: "media_asset".to_string(),
        extends: None,
        mixins: vec![],
        overrides: None,
        description: Some("Media asset (image, video, document, etc.)".to_string()),
        icon: Some("file-image".to_string()),
        version: Some(1),
        properties: Some(vec![]),
        allowed_children: vec![],
        required_nodes: vec![],
        initial_structure: None,
        versionable: Some(true),
        immutable: None,
        publishable: Some(false),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };

    // Create an UpdateNodeType operation
    let op_type = OpType::UpdateNodeType {
        node_type_id: "media_asset".to_string(),
        node_type: node_type.clone(),
    };

    eprintln!(
        "
=== Testing UpdateNodeType Serialization ==="
    );
    eprintln!("NodeType name: {}", node_type.name);
    eprintln!("NodeType description: {:?}", node_type.description);

    // Test 1: Unnamed (compact) format CANNOT round-trip NodeType.
    // NodeType uses `skip_serializing_if`, so positional (unnamed) msgpack
    // shifts fields on deserialize. All persistence/network paths must use
    // to_vec_named - this assertion documents the constraint.
    eprintln!(
        "
--- Test 1: Unnamed serialization (expected to NOT round-trip) ---"
    );
    let bytes_unnamed = rmp_serde::to_vec(&op_type).unwrap();
    eprintln!("Serialized size (unnamed): {} bytes", bytes_unnamed.len());

    let roundtrip_unnamed: Result<OpType, _> = rmp_serde::from_slice(&bytes_unnamed);
    assert!(
        roundtrip_unnamed.is_err(),
        "compact msgpack unexpectedly round-tripped NodeType - if skip_serializing_if \
         was removed, compact encoding may be usable again"
    );

    // Test 2: Serialize using named format (used in network protocol)
    eprintln!(
        "
--- Test 2: Named serialization (network protocol) ---"
    );
    let bytes_named = rmp_serde::to_vec_named(&op_type).unwrap();
    eprintln!("Serialized size (named): {} bytes", bytes_named.len());

    let roundtrip_named: OpType = rmp_serde::from_slice(&bytes_named).unwrap();
    if let OpType::UpdateNodeType {
        node_type: rt_nodetype,
        ..
    } = roundtrip_named
    {
        assert_eq!(rt_nodetype.name, "media_asset");
        assert_eq!(
            rt_nodetype.description,
            Some("Media asset (image, video, document, etc.)".to_string())
        );
        eprintln!("Named format: Deserialization successful");
    } else {
        panic!("Expected UpdateNodeType variant after named deserialization");
    }

    // Test 3: Full Operation serialization with UpdateNodeType (mimicking real usage)
    eprintln!(
        "
--- Test 3: Full Operation with UpdateNodeType ---"
    );
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let operation = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        op_type,
        "system".to_string(),
    );

    // Serialize the full operation with named format (as done in oplog and network)
    let op_bytes = rmp_serde::to_vec_named(&operation).unwrap();
    eprintln!(
        "Full Operation serialized size (named): {} bytes",
        op_bytes.len()
    );

    // Deserialize the full operation
    let roundtrip_op: Operation = rmp_serde::from_slice(&op_bytes).unwrap();
    eprintln!("Full Operation deserialization successful");

    if let OpType::UpdateNodeType {
        node_type: rt_nodetype,
        node_type_id,
    } = &roundtrip_op.op_type
    {
        assert_eq!(node_type_id, "media_asset");
        assert_eq!(rt_nodetype.name, "media_asset");
        assert_eq!(
            rt_nodetype.description,
            Some("Media asset (image, video, document, etc.)".to_string())
        );
        eprintln!("NodeType fields correctly preserved in full Operation");
    } else {
        panic!("Expected UpdateNodeType variant in deserialized Operation");
    }

    // Test 4: Cross-format compatibility (serialize with named, deserialize with unnamed)
    eprintln!(
        "
--- Test 4: Cross-format compatibility ---"
    );
    let named_bytes = rmp_serde::to_vec_named(&operation).unwrap();
    let cross_roundtrip: Result<Operation, _> = rmp_serde::from_slice(&named_bytes);

    match cross_roundtrip {
        Ok(op) => {
            if let OpType::UpdateNodeType { node_type, .. } = &op.op_type {
                assert_eq!(
                    node_type.description,
                    Some("Media asset (image, video, document, etc.)".to_string())
                );
                eprintln!("Cross-format deserialization successful");
            } else {
                panic!("Expected UpdateNodeType variant in cross-format roundtrip");
            }
        }
        Err(e) => {
            panic!("Cross-format deserialization failed: {}", e);
        }
    }

    eprintln!(
        "
=== All UpdateNodeType serialization tests passed! ===
"
    );
}

/// The `agent` field must be additive in BOTH directions across a mixed-version
/// cluster. Every production path — the RocksDB oplog
/// (`repositories/oplog/helpers.rs`), the TCP protocol
/// (`tcp_protocol/message_impl.rs`) and the HTTP batch handler — encodes an
/// `Operation` as a NAME-KEYED map, which is what makes that possible.
mod agent_field_is_additive {
    use super::*;

    fn sample(agent: Option<&str>) -> Operation {
        let mut op = Operation::new(
            7,
            "node-a".to_string(),
            VectorClock::new(),
            "t".to_string(),
            "r".to_string(),
            "main".to_string(),
            OpType::DeleteNodeSnapshot {
                node_id: "n1".to_string(),
                revision: raisin_hlc::HLC::new(1_000, 0),
                node: None,
                parent_id: None,
            },
            "alice".to_string(),
        );
        op.agent = agent.map(|a| a.to_string());
        op
    }

    #[test]
    fn it_round_trips_through_the_named_msgpack_used_everywhere() {
        let op = sample(Some("mcp:studio-admin"));
        let bytes = rmp_serde::to_vec_named(&op).unwrap();
        let back: Operation = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(back.agent.as_deref(), Some("mcp:studio-admin"));
        assert_eq!(back, op);
    }

    /// OLD writer → NEW reader: the key is simply absent.
    #[test]
    fn a_record_written_before_the_field_existed_reads_as_none() {
        let op = sample(None);
        let mut map: serde_json::Value = serde_json::to_value(&op).unwrap();
        map.as_object_mut().unwrap().remove("agent");
        assert!(map.get("agent").is_none(), "simulating a pre-field record");

        let back: Operation = serde_json::from_value(map).unwrap();
        assert_eq!(back.agent, None);
        assert_eq!(back, op);
    }

    /// NEW writer → OLD reader: an older binary has no `agent` field in its
    /// derive, and serde routes unknown map keys to `IgnoredAny` (there is no
    /// `deny_unknown_fields` on `Operation`). Stand in for the older struct with
    /// one that lacks the field and assert the rest still decodes.
    #[test]
    fn an_older_peer_ignores_the_extra_key_instead_of_failing() {
        #[derive(serde::Deserialize)]
        struct OperationAsAnOlderPeerSeesIt {
            op_seq: u64,
            actor: String,
        }

        let bytes = rmp_serde::to_vec_named(&sample(Some("agent:/agents/bot"))).unwrap();
        let old: OperationAsAnOlderPeerSeesIt = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(old.op_seq, 7);
        assert_eq!(old.actor, "alice");
    }
}
