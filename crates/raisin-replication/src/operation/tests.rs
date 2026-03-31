use super::*;
use crate::vector_clock::VectorClock;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

#[test]
fn test_operation_creation() {
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        OpType::CreateNode {
            node_id: "test123".to_string(),
            name: "test-article".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "a".to_string(),
            properties: {
                let mut props = HashMap::new();
                props.insert(
                    "title".to_string(),
                    PropertyValue::String("Test".to_string()),
                );
                props
            },
            owner_id: None,
            path: "/test-article".to_string(),
            workspace: None,
        },
        "user@example.com".to_string(),
    );

    assert_eq!(op.op_seq, 1);
    assert_eq!(op.cluster_node_id, "node1");
    assert_eq!(op.target(), OperationTarget::Node("test123".to_string()));
}

#[test]
fn test_operation_target() {
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op = Operation::new(
        1,
        "node1".to_string(),
        vc.clone(),
        "t1".to_string(),
        "r1".to_string(),
        "main".to_string(),
        OpType::SetProperty {
            node_id: "node123".to_string(),
            property_name: "title".to_string(),
            value: PropertyValue::String("New Title".to_string()),
        },
        "user".to_string(),
    );

    assert_eq!(op.target(), OperationTarget::Node("node123".to_string()));
}

#[test]
fn test_is_delete() {
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let delete_op = Operation::new(
        1,
        "node1".to_string(),
        vc.clone(),
        "t1".to_string(),
        "r1".to_string(),
        "main".to_string(),
        OpType::DeleteNode {
            node_id: "node123".to_string(),
        },
        "user".to_string(),
    );

    assert!(delete_op.is_delete());

    let create_op = Operation::new(
        2,
        "node1".to_string(),
        vc,
        "t1".to_string(),
        "r1".to_string(),
        "main".to_string(),
        OpType::CreateNode {
            node_id: "node456".to_string(),
            name: "test-node".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "a".to_string(),
            properties: HashMap::new(),
            owner_id: None,
            workspace: None,
            path: "/test-node".to_string(),
        },
        "user".to_string(),
    );

    assert!(!create_op.is_delete());
}

#[test]
fn test_acknowledgment() {
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let mut op = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "t1".to_string(),
        "r1".to_string(),
        "main".to_string(),
        OpType::CreateNode {
            node_id: "test".to_string(),
            name: "test-article".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "a".to_string(),
            properties: HashMap::new(),
            owner_id: None,
            workspace: None,
            path: "/test-article".to_string(),
        },
        "user".to_string(),
    );

    assert!(!op.acknowledged_by_all(&["peer1".to_string(), "peer2".to_string()]));

    op.acknowledge("peer1");
    assert!(!op.acknowledged_by_all(&["peer1".to_string(), "peer2".to_string()]));

    op.acknowledge("peer2");
    assert!(op.acknowledged_by_all(&["peer1".to_string(), "peer2".to_string()]));
}

#[test]
fn test_optype_msgpack_debug() {
    use raisin_models::nodes::properties::PropertyValue;

    // Test CreateNode with properties
    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String("Test".to_string()),
    );

    let op_type = OpType::CreateNode {
        node_id: "test-1".to_string(),
        name: "Test Node".to_string(),
        node_type: "Article".to_string(),
        archetype: None,
        parent_id: None,
        order_key: "a".to_string(),
        properties: props.clone(),
        owner_id: None,
        workspace: None,
        path: "/Test Node".to_string(),
    };

    // Serialize to MessagePack
    let bytes = rmp_serde::to_vec(&op_type).unwrap();
    eprintln!(
        "
OpType::CreateNode MessagePack bytes (len={}):",
        bytes.len()
    );
    eprintln!("First 50 bytes: {:?}", &bytes[..bytes.len().min(50)]);

    // Try to deserialize
    let roundtrip: OpType = rmp_serde::from_slice(&bytes).unwrap();

    if let OpType::CreateNode {
        properties: rt_props,
        ..
    } = roundtrip
    {
        assert_eq!(rt_props.len(), 1);
    } else {
        panic!("Expected CreateNode variant");
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

    // Test 1: Serialize using unnamed format (default)
    eprintln!(
        "
--- Test 1: Unnamed serialization ---"
    );
    let bytes_unnamed = rmp_serde::to_vec(&op_type).unwrap();
    eprintln!("Serialized size (unnamed): {} bytes", bytes_unnamed.len());

    let roundtrip_unnamed: OpType = rmp_serde::from_slice(&bytes_unnamed).unwrap();
    if let OpType::UpdateNodeType {
        node_type: rt_nodetype,
        ..
    } = roundtrip_unnamed
    {
        assert_eq!(rt_nodetype.name, "media_asset");
        assert_eq!(
            rt_nodetype.description,
            Some("Media asset (image, video, document, etc.)".to_string())
        );
        eprintln!("Unnamed format: Deserialization successful");
    } else {
        panic!("Expected UpdateNodeType variant after unnamed deserialization");
    }

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
