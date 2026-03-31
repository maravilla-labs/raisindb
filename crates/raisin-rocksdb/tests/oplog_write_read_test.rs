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

    if let OpType::CreateNode {
        properties: rt_props,
        ..
    } = &read_op.op_type
    {
        assert_eq!(rt_props.len(), 1);
        assert_eq!(
            rt_props.get("content"),
            Some(&PropertyValue::String("Hello World".to_string()))
        );
    } else {
        panic!("Expected CreateNode variant");
    }

    println!("✅ All assertions passed!");
}
