use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::{OpType, Operation, VectorClock};
use raisin_rocksdb::{OpLogRepository, RocksDBStorage};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

#[test]
fn test_debug_serialization_issue() {
    // Create a temp database
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
    let oplog = OpLogRepository::new(storage.db().clone());

    // Create an operation with properties - exactly like the failing test
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

    // Serialize directly to see what bytes we get
    let direct_bytes = rmp_serde::to_vec(&op).unwrap();
    println!("✅ Direct serialization: {} bytes", direct_bytes.len());
    println!(
        "First 200 bytes: {:?}",
        &direct_bytes[..direct_bytes.len().min(200)]
    );

    // Try deserializing directly
    let direct_deser: Operation = rmp_serde::from_slice(&direct_bytes).unwrap();
    println!("✅ Direct deserialization succeeded");

    // Now write to OpLog
    oplog.put_operation(&op).unwrap();
    println!("✅ Wrote to OpLog");

    // Read back using get_missing_operations (like replication does)
    let missing_ops = oplog
        .get_missing_operations("tenant1", "repo1", &VectorClock::new(), None)
        .unwrap();

    println!(
        "✅ Read {} operations via get_missing_operations",
        missing_ops.len()
    );
    assert_eq!(missing_ops.len(), 1);

    // Also test get_operations_from_seq
    let seq_ops = oplog
        .get_operations_from_seq("tenant1", "repo1", "node1", 0)
        .unwrap();

    println!(
        "✅ Read {} operations via get_operations_from_seq",
        seq_ops.len()
    );
    assert_eq!(seq_ops.len(), 1);

    println!("✅ All methods work!");
}
