use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{OpType, Operation, VectorClock};
use std::collections::HashMap;

/// A node snapshot op as a commit replicates it: an article with properties.
pub(crate) fn article_snapshot() -> OpType {
    let mut props = HashMap::new();
    props.insert(
        "content".to_string(),
        PropertyValue::String("Hello World".to_string()),
    );
    OpType::UpsertNodeSnapshot {
        node: Node {
            id: "article-1".to_string(),
            name: "My First Article".to_string(),
            path: "/My First Article".to_string(),
            node_type: "Article".to_string(),
            workspace: Some("content".to_string()),
            properties: props,
            ..Default::default()
        },
        parent_id: Some("/".to_string()),
        revision: HLC::new(1_000, 0),
        cf_order_key: "a0::article-1".to_string(),
    }
}

#[test]
fn test_full_operation_with_properties_msgpack_roundtrip() {
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        article_snapshot(),
        "test_actor".to_string(),
    );

    // Serialize to MessagePack as the oplog stores it (name-keyed maps)
    let bytes = rmp_serde::to_vec_named(&op).unwrap();

    // Deserialize (this is what happens when reading from RocksDB)
    let roundtrip: Operation = rmp_serde::from_slice(&bytes).unwrap();

    assert_eq!(roundtrip.op_seq, 1);
    assert_eq!(roundtrip.cluster_node_id, "node1");

    if let OpType::UpsertNodeSnapshot { node, .. } = roundtrip.op_type {
        assert_eq!(node.properties.len(), 1);
        assert_eq!(
            node.properties.get("content"),
            Some(&PropertyValue::String("Hello World".to_string()))
        );
    } else {
        panic!("Expected UpsertNodeSnapshot variant");
    }
}
