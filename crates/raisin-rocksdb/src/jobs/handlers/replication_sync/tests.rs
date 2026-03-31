//! Tests for replication sync handler.

use super::*;
use crate::repositories::OpLogRepository;
use crate::RocksDBConfig;
use raisin_replication::{Operation, VectorClock};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_replication_sync_handler_build_vector_clock() {
    // Create temporary database
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());

    // Create handler
    let handler = ReplicationSyncHandler::new(db.clone(), "node1".to_string());

    let oplog_repo = OpLogRepository::new(db);

    // Build vector clock from empty operations
    let vc = handler
        .build_vector_clock(&oplog_repo, "tenant1", "repo1")
        .unwrap();

    assert!(vc.is_empty());
}

#[test]
fn test_filter_new_operations() {
    // Create temporary database
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());

    let handler = ReplicationSyncHandler::new(db.clone(), "node1".to_string());
    let oplog_repo = OpLogRepository::new(db);

    // Create test operations
    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op1 = raisin_replication::Operation::new(
        1,
        "node1".to_string(),
        vc.clone(),
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        raisin_replication::OpType::CreateNode {
            node_id: "test123".to_string(),
            name: "Test Article".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "a".to_string(),
            properties: std::collections::HashMap::new(),
            owner_id: None,
            workspace: None,
            path: String::new(),
        },
        "test@example.com".to_string(),
    );

    // Store one operation
    oplog_repo.put_operation(&op1).unwrap();

    // Try to filter - should be empty since op1 already exists
    let filtered = handler
        .filter_new_operations(&oplog_repo, "tenant1", "repo1", vec![op1.clone()])
        .unwrap();

    assert_eq!(filtered.len(), 0);

    // Create a new operation
    vc.increment("node1");
    let op2 = raisin_replication::Operation::new(
        2,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        raisin_replication::OpType::CreateNode {
            node_id: "test456".to_string(),
            name: "Test Article 2".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "b".to_string(),
            properties: std::collections::HashMap::new(),
            owner_id: None,
            workspace: None,
            path: String::new(),
        },
        "test@example.com".to_string(),
    );

    // Filter with new operation - should include it
    let filtered = handler
        .filter_new_operations(&oplog_repo, "tenant1", "repo1", vec![op2.clone()])
        .unwrap();

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].op_id, op2.op_id);
}

#[test]
fn test_branch_filtering() {
    // Test the branch filtering logic directly with in-memory operations
    let mut vc = VectorClock::new();
    vc.increment("node1");

    // Create operations on different branches
    let main_op = Operation::new(
        1,
        "node1".to_string(),
        vc.clone(),
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        raisin_replication::OpType::CreateNode {
            node_id: "node_main".to_string(),
            name: "Node Main".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "a".to_string(),
            properties: std::collections::HashMap::new(),
            owner_id: None,
            workspace: None,
            path: String::new(),
        },
        "test@example.com".to_string(),
    );

    vc.increment("node1");
    let dev_op = Operation::new(
        2,
        "node1".to_string(),
        vc.clone(),
        "tenant1".to_string(),
        "repo1".to_string(),
        "develop".to_string(),
        raisin_replication::OpType::CreateNode {
            node_id: "node_dev".to_string(),
            name: "Node Dev".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "b".to_string(),
            properties: std::collections::HashMap::new(),
            owner_id: None,
            workspace: None,
            path: String::new(),
        },
        "test@example.com".to_string(),
    );

    vc.increment("node1");
    let staging_op = Operation::new(
        3,
        "node1".to_string(),
        vc.clone(),
        "tenant1".to_string(),
        "repo1".to_string(),
        "staging".to_string(),
        raisin_replication::OpType::CreateNode {
            node_id: "node_staging".to_string(),
            name: "Node Staging".to_string(),
            node_type: "article".to_string(),
            archetype: None,
            parent_id: None,
            order_key: "c".to_string(),
            properties: std::collections::HashMap::new(),
            owner_id: None,
            workspace: None,
            path: String::new(),
        },
        "test@example.com".to_string(),
    );

    let all_ops = vec![main_op.clone(), dev_op.clone(), staging_op.clone()];

    // Test filtering by single branch
    let main_filter = vec!["main".to_string()];
    let filtered: Vec<&Operation> = all_ops
        .iter()
        .filter(|op| main_filter.contains(&op.branch))
        .collect();

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].branch, "main");
    assert_eq!(filtered[0].op_seq, 1);

    // Test filtering by multiple branches
    let multi_filter = vec!["main".to_string(), "develop".to_string()];
    let filtered: Vec<&Operation> = all_ops
        .iter()
        .filter(|op| multi_filter.contains(&op.branch))
        .collect();

    assert_eq!(filtered.len(), 2);
    let branches: Vec<String> = filtered.iter().map(|op| op.branch.clone()).collect();
    assert!(branches.contains(&"main".to_string()));
    assert!(branches.contains(&"develop".to_string()));
    assert!(!branches.contains(&"staging".to_string()));

    // Test no filter (empty list means all branches)
    let no_filter: Vec<String> = vec![];
    let filtered: Vec<&Operation> = if no_filter.is_empty() {
        all_ops.iter().collect()
    } else {
        all_ops
            .iter()
            .filter(|op| no_filter.contains(&op.branch))
            .collect()
    };

    assert_eq!(filtered.len(), 3);
}
