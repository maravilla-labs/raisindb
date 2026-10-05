use crate::full_operation_msgpack_test::article_snapshot;
use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::OpType;

#[test]
fn test_node_snapshot_with_properties_msgpack_roundtrip() {
    let op_type = article_snapshot();

    let bytes = rmp_serde::to_vec_named(&op_type).unwrap();
    let roundtrip: OpType = rmp_serde::from_slice(&bytes).unwrap();

    if let OpType::UpsertNodeSnapshot { node, .. } = roundtrip {
        assert_eq!(node.properties.len(), 1);
        assert_eq!(
            node.properties.get("content"),
            Some(&PropertyValue::String("Hello World".to_string()))
        );
    } else {
        panic!("Expected UpsertNodeSnapshot variant");
    }
}
