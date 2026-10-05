//! Tests for operation capture

use super::core::OperationCapture;
use raisin_replication::OpType;
use std::sync::Arc;

use crate::RocksDBConfig;
use tempfile::tempdir;

#[tokio::test]
async fn test_operation_capture() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());

    let capture = OperationCapture::new(db, "node1".to_string());

    let op = capture
        .capture_operation(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::DeleteNodeSnapshot {
                node_id: "test_node".to_string(),
                revision: raisin_hlc::HLC::new(1, 0),
                node: None,
                parent_id: None,
            },
            "test_user".to_string(),
            Some("Create test node".to_string()),
            false,
        )
        .await
        .unwrap();

    assert_eq!(op.op_seq, 1);
    assert_eq!(op.cluster_node_id, "node1");
    assert_eq!(op.tenant_id, "tenant1");

    // Verify it was written to oplog
    let ops = capture
        .oplog_repo()
        .get_operations_from_node("tenant1", "repo1", "node1")
        .unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_id, op.op_id);
}

#[tokio::test]
async fn test_vector_clock_increment() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());

    let capture = OperationCapture::new(db, "node1".to_string());

    // Initial vector clock should be empty
    let vc1 = capture.get_vector_clock("tenant1", "repo1").await;
    assert_eq!(vc1.get("node1"), 0);

    // Capture an operation (should increment VC)
    capture
        .capture_operation(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::DeleteNodeSnapshot {
                node_id: "node1".to_string(),
                revision: raisin_hlc::HLC::new(1, 0),
                node: None,
                parent_id: None,
            },
            "test".to_string(),
            None,
            false,
        )
        .await
        .unwrap();

    let vc2 = capture.get_vector_clock("tenant1", "repo1").await;
    assert_eq!(vc2.get("node1"), 1);
}

#[tokio::test]
async fn test_restore_from_oplog() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());

    // Create first capture instance and log some operations
    {
        let capture = OperationCapture::new(db.clone(), "node1".to_string());

        for i in 1..=5 {
            capture
                .capture_operation(
                    "tenant1".to_string(),
                    "repo1".to_string(),
                    "main".to_string(),
                    OpType::DeleteNodeSnapshot {
                        node_id: format!("node{}", i),
                        revision: raisin_hlc::HLC::new(1, 0),
                        node: None,
                        parent_id: None,
                    },
                    "test".to_string(),
                    None,
                    false,
                )
                .await
                .unwrap();
        }

        let vc = capture.get_vector_clock("tenant1", "repo1").await;
        assert_eq!(vc.get("node1"), 5);
    }

    // Create new capture instance and restore
    {
        let capture = OperationCapture::new(db.clone(), "node1".to_string());

        // Before restore, op_seq should be 0
        assert_eq!(capture.get_op_seq("tenant1", "repo1").await, 0);

        // Restore from oplog
        capture
            .restore_from_oplog("tenant1", "repo1")
            .await
            .unwrap();

        // After restore, should resume from where we left off
        assert_eq!(capture.get_op_seq("tenant1", "repo1").await, 5);

        let vc = capture.get_vector_clock("tenant1", "repo1").await;
        assert_eq!(vc.get("node1"), 5);

        // Next operation should get seq 6
        let op = capture
            .capture_operation(
                "tenant1".to_string(),
                "repo1".to_string(),
                "main".to_string(),
                OpType::DeleteNodeSnapshot {
                    node_id: "node6".to_string(),
                    revision: raisin_hlc::HLC::new(1, 0),
                    node: None,
                    parent_id: None,
                },
                "test".to_string(),
                None,
                false,
            )
            .await
            .unwrap();

        assert_eq!(op.op_seq, 6);
    }
}

#[tokio::test]
async fn test_disabled_capture() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());

    let capture = OperationCapture::disabled(db);

    assert!(!capture.is_enabled());

    // Should return dummy operation
    let op = capture
        .capture_operation(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::DeleteNodeSnapshot {
                node_id: "node1".to_string(),
                revision: raisin_hlc::HLC::new(1, 0),
                node: None,
                parent_id: None,
            },
            "test".to_string(),
            None,
            false,
        )
        .await
        .unwrap();

    assert_eq!(op.op_seq, 0); // Dummy operation

    // Verify nothing was written
    let ops = capture
        .oplog_repo()
        .get_operations_from_node("tenant1", "repo1", "node1")
        .unwrap();
    assert_eq!(ops.len(), 0);
}
