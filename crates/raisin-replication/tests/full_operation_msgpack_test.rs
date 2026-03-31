use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::{OpType, Operation, VectorClock};
use std::collections::HashMap;

#[test]
fn test_full_operation_with_properties_msgpack_roundtrip() {
    let mut props = HashMap::new();
    props.insert(
        "content".to_string(),
        PropertyValue::String("Hello World".to_string()),
    );

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
            node_id: "article-1".to_string(),
            name: "My First Article".to_string(),
            node_type: "Article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "a".to_string(),
            properties: props.clone(),
            owner_id: None,
            workspace: None,
            path: "/My First Article".to_string(),
        },
        "test_actor".to_string(),
    );

    println!("Original operation: {:?}", op);

    // Serialize to MessagePack (this is what goes into RocksDB)
    let bytes = rmp_serde::to_vec(&op).unwrap();
    println!("Serialized {} bytes", bytes.len());
    println!("First 200 bytes: {:?}", &bytes[..bytes.len().min(200)]);

    // Deserialize (this is what happens when reading from RocksDB)
    let roundtrip: Operation = rmp_serde::from_slice(&bytes).unwrap();
    println!("Deserialized successfully!");
    println!("Roundtrip operation: {:?}", roundtrip);

    assert_eq!(roundtrip.op_seq, 1);
    assert_eq!(roundtrip.cluster_node_id, "node1");

    if let OpType::CreateNode {
        properties: rt_props,
        ..
    } = roundtrip.op_type
    {
        assert_eq!(rt_props.len(), 1);
        assert_eq!(
            rt_props.get("content"),
            Some(&PropertyValue::String("Hello World".to_string()))
        );
    } else {
        panic!("Expected CreateNode variant");
    }
}
