//! Tests for operation queue

use super::*;
use crate::RocksDBConfig;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn test_queue_enqueue_and_process() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());
    let operation_capture = Arc::new(OperationCapture::new(db, "test_node".to_string()));

    let queue = OperationQueue::new(
        operation_capture.clone(),
        100,
        10,
        Duration::from_millis(50),
    );

    // Enqueue an operation
    let op = QueuedOperation {
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "node1".to_string(),
            property_name: "title".to_string(),
            value: raisin_models::nodes::properties::PropertyValue::String("Test".to_string()),
        },
        actor: "test_user".to_string(),
        message: Some("Test operation".to_string()),
        is_system: false,
        revision: None,
    };

    queue.try_enqueue(op.clone()).unwrap();

    // Wait for processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stats = queue.stats();
    assert_eq!(stats.enqueued_count, 1);
    assert_eq!(stats.processed_count, 1);
    assert_eq!(stats.failed_count, 0);

    queue.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_batch_processing() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());
    let operation_capture = Arc::new(OperationCapture::new(db, "test_node".to_string()));

    let queue = OperationQueue::new(
        operation_capture.clone(),
        1000,
        10,
        Duration::from_millis(100),
    );

    // Enqueue multiple operations
    for i in 0..25 {
        let op = QueuedOperation {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: format!("node{}", i),
                property_name: "value".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::Integer(i as i64),
            },
            actor: "test_user".to_string(),
            message: Some(format!("Operation {}", i)),
            is_system: false,
            revision: None,
        };

        queue.try_enqueue(op).unwrap();
    }

    // Wait for processing
    tokio::time::sleep(Duration::from_millis(300)).await;

    let stats = queue.stats();
    assert_eq!(stats.enqueued_count, 25);
    assert_eq!(stats.processed_count, 25);
    assert_eq!(stats.failed_count, 0);

    queue.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_queue_full_backpressure() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());
    let operation_capture = Arc::new(OperationCapture::new(db, "test_node".to_string()));

    // Very small queue to test backpressure
    let queue = OperationQueue::new(
        operation_capture.clone(),
        5,
        10,
        Duration::from_millis(1000), // Long timeout to keep queue full
    );

    // Fill the queue
    for i in 0..5 {
        let op = QueuedOperation {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: format!("node{}", i),
                property_name: "value".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::Integer(i as i64),
            },
            actor: "test_user".to_string(),
            message: None,
            is_system: false,
            revision: None,
        };

        queue.try_enqueue(op).unwrap();
    }

    // Next enqueue should fail (queue full)
    let op = QueuedOperation {
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "overflow".to_string(),
            property_name: "value".to_string(),
            value: raisin_models::nodes::properties::PropertyValue::Integer(999),
        },
        actor: "test_user".to_string(),
        message: None,
        is_system: false,
        revision: None,
    };

    let result = queue.try_enqueue(op);
    assert!(result.is_err());

    queue.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_graceful_shutdown() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());
    let operation_capture = Arc::new(OperationCapture::new(db, "test_node".to_string()));

    let queue = OperationQueue::new(
        operation_capture.clone(),
        100,
        10,
        Duration::from_millis(50),
    );

    // Enqueue operations
    for i in 0..10 {
        let op = QueuedOperation {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: format!("node{}", i),
                property_name: "value".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::Integer(i as i64),
            },
            actor: "test_user".to_string(),
            message: None,
            is_system: false,
            revision: None,
        };

        queue.try_enqueue(op).unwrap();
    }

    // Shutdown should wait for all operations to process
    queue.shutdown().await.unwrap();

    // After shutdown, all operations should be processed
    let stats = operation_capture
        .oplog_repo()
        .get_all_operations("tenant1", "repo1")
        .unwrap();
    assert!(!stats.is_empty());
}

#[tokio::test]
async fn test_timeout_based_batching() {
    let dir = tempdir().unwrap();
    let config = RocksDBConfig {
        path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let db = Arc::new(crate::open_db_with_config(&config).unwrap());
    let operation_capture = Arc::new(OperationCapture::new(db, "test_node".to_string()));

    // Large batch size but short timeout
    let queue = OperationQueue::new(
        operation_capture.clone(),
        100,
        100,                        // Won't reach this
        Duration::from_millis(100), // Will trigger on timeout
    );

    // Enqueue just a few operations (less than batch size)
    for i in 0..5 {
        let op = QueuedOperation {
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: format!("node{}", i),
                property_name: "value".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::Integer(i as i64),
            },
            actor: "test_user".to_string(),
            message: None,
            is_system: false,
            revision: None,
        };

        queue.try_enqueue(op).unwrap();
    }

    // Wait for timeout to trigger batch processing
    tokio::time::sleep(Duration::from_millis(200)).await;

    let stats = queue.stats();
    assert_eq!(stats.enqueued_count, 5);
    assert_eq!(stats.processed_count, 5);

    queue.shutdown().await.unwrap();
}
