use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::{OpType, Operation, VectorClock};
use raisin_rocksdb::{OpLogRepository, RocksDBStorage};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

#[test]
fn test_oplog_write_then_read_with_properties() {
    // Create a temp database
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
    let oplog = OpLogRepository::new(storage.db().clone());

    // Create an operation with properties
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
        OpType::UpsertNodeSnapshot {
            node: raisin_models::nodes::Node {
                id: "article-1".to_string(),
                name: "My First Article".to_string(),
                path: "/My First Article".to_string(),
                node_type: "Article".to_string(),
                workspace: Some("content".to_string()),
                properties: props.clone(),
                ..Default::default()
            },
            parent_id: Some("/".to_string()),
            revision: raisin_hlc::HLC::new(1, 0),
            cf_order_key: "a0::article-1".to_string(),
        },
        "test_actor".to_string(),
    );

    println!("Original operation: {:?}", op);

    // Write operation to OpLog
    oplog.put_operation(&op).unwrap();
    println!("✅ Wrote operation to OpLog");

    // Read it back
    let ops_by_node = oplog.get_all_operations("tenant1", "repo1").unwrap();

    println!(
        "✅ Read operations by node: {:?}",
        ops_by_node.keys().collect::<Vec<_>>()
    );

    let read_ops = ops_by_node.get("node1").expect("Expected node1 operations");
    println!("✅ Read {} operations from OpLog for node1", read_ops.len());

    assert_eq!(read_ops.len(), 1);
    let read_op = &read_ops[0];

    println!("Read operation: {:?}", read_op);

    assert_eq!(read_op.op_seq, 1);
    assert_eq!(read_op.cluster_node_id, "node1");

    if let OpType::UpsertNodeSnapshot { node, .. } = &read_op.op_type {
        let rt_props = &node.properties;
        assert_eq!(rt_props.len(), 1);
        assert_eq!(
            rt_props.get("content"),
            Some(&PropertyValue::String("Hello World".to_string()))
        );
    } else {
        panic!("Expected UpsertNodeSnapshot variant");
    }

    println!("✅ All assertions passed!");
}
