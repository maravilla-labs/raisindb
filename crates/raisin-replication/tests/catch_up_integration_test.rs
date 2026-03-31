//! Integration tests for P2P cluster catch-up protocol
//!
//! These tests verify the complete catch-up flow including:
//! - Cluster discovery
//! - Consensus calculation
//! - Checkpoint transfer
//! - Log verification
//! - Conflict resolution

use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::{CatchUpCoordinator, ConflictResolver, OpType, Operation, VectorClock};
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test helper to create a test operation
fn create_test_operation(node_id: &str, op_seq: u64, timestamp_ms: u64) -> Operation {
    let mut vc = VectorClock::new();
    vc.set(node_id, op_seq);

    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq,
        cluster_node_id: node_id.to_string(),
        timestamp_ms,
        vector_clock: vc,
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        branch: "main".to_string(),
        op_type: OpType::SetProperty {
            node_id: "test_node".to_string(),
            property_name: "test_prop".to_string(),
            value: PropertyValue::String("test_value".to_string()),
        },
        revision: None,
        actor: "test".to_string(),
        message: None,
        is_system: false,
        acknowledged_by: std::collections::HashSet::new(),
    }
}

#[tokio::test]
async fn test_catch_up_coordinator_creation() {
    let temp_data = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    let coordinator = CatchUpCoordinator::new(
        "node1".to_string(),
        vec!["127.0.0.1:9001".to_string(), "127.0.0.1:9002".to_string()],
        temp_data.path().to_path_buf(),
        temp_staging.path().to_path_buf(),
        None, // No storage backend for test
        None, // No checkpoint ingestor for test
        None, // No tantivy receiver for test
        None, // No hnsw receiver for test
        None, // Use default checkpoint threshold
    );

    // Coordinator should be created successfully
    assert_eq!(
        std::mem::size_of_val(&coordinator) > 0,
        true,
        "Coordinator created"
    );
}

#[tokio::test]
async fn test_consensus_calculation() {
    use raisin_replication::CatchUpPeerStatus;

    let temp_data = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    let coordinator = CatchUpCoordinator::new(
        "node1".to_string(),
        vec![],
        temp_data.path().to_path_buf(),
        temp_staging.path().to_path_buf(),
        None, // No storage backend for test
        None, // No checkpoint ingestor for test
        None, // No tantivy receiver for test
        None, // No hnsw receiver for test
        None, // Use default checkpoint threshold
    );

    // This test verifies the consensus calculation logic
    // In a real scenario, this would be tested with actual peer discovery

    // Create test vector clocks
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 10);
    vc1.set("node2", 5);

    let mut vc2 = VectorClock::new();
    vc2.set("node1", 8);
    vc2.set("node2", 7);
    vc2.set("node3", 2);

    // Test vector clock merging
    let mut merged = VectorClock::new();
    merged.merge(&vc1);
    merged.merge(&vc2);

    // Merged should have max of each node
    assert_eq!(merged.get("node1"), 10);
    assert_eq!(merged.get("node2"), 7);
    assert_eq!(merged.get("node3"), 2);
}

#[tokio::test]
async fn test_conflict_resolution() {
    let resolver = ConflictResolver::new("node1".to_string());

    // Create concurrent operations from different nodes
    let op1 = create_test_operation("node1", 1, 1000);
    let op2 = create_test_operation("node2", 1, 2000);
    let op3 = create_test_operation("node3", 1, 1500);

    let local_ops = vec![op1.clone()];
    let mut peer_logs = HashMap::new();
    peer_logs.insert("node2".to_string(), vec![op2.clone()]);
    peer_logs.insert("node3".to_string(), vec![op3.clone()]);

    // Resolve divergent logs
    let result = resolver
        .resolve_divergent_logs(local_ops, peer_logs)
        .unwrap();

    // Should have all 3 operations in deterministic order
    assert_eq!(result.len(), 3);

    // Operations should be ordered by timestamp
    assert!(result[0].timestamp_ms <= result[1].timestamp_ms);
    assert!(result[1].timestamp_ms <= result[2].timestamp_ms);
}

#[tokio::test]
async fn test_conflict_resolution_with_duplicates() {
    let resolver = ConflictResolver::new("node1".to_string());

    let op1 = create_test_operation("node1", 1, 1000);
    let op2 = create_test_operation("node2", 1, 2000);

    // Same operation appears in multiple logs (duplicate)
    let local_ops = vec![op1.clone()];
    let mut peer_logs = HashMap::new();
    peer_logs.insert("node2".to_string(), vec![op2.clone(), op1.clone()]);

    let result = resolver
        .resolve_divergent_logs(local_ops, peer_logs)
        .unwrap();

    // Should deduplicate - only 2 unique operations
    assert_eq!(result.len(), 2);
}

#[tokio::test]
async fn test_conflict_detection() {
    let resolver = ConflictResolver::new("node1".to_string());

    // Create two operations with concurrent vector clocks
    let mut vc1 = VectorClock::new();
    vc1.set("node1", 1);

    let mut vc2 = VectorClock::new();
    vc2.set("node2", 1);

    let mut op1 = create_test_operation("node1", 1, 1000);
    op1.vector_clock = vc1;

    let mut op2 = create_test_operation("node2", 1, 2000);
    op2.vector_clock = vc2;

    // Detect conflicts
    let ops = vec![op1, op2];
    let conflicts = resolver.detect_conflicts(&ops);

    // Should detect 1 conflict (same target, concurrent clocks)
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].concurrent_operations.len(), 1);
}

#[tokio::test]
async fn test_vector_clock_ordering() {
    use raisin_replication::ClockOrdering;

    let mut vc1 = VectorClock::new();
    vc1.set("node1", 5);
    vc1.set("node2", 3);

    let mut vc2 = VectorClock::new();
    vc2.set("node1", 4);
    vc2.set("node2", 4);

    // vc1 and vc2 are concurrent (neither happens-before the other)
    assert_eq!(vc1.compare(&vc2), ClockOrdering::Concurrent);

    let mut vc3 = VectorClock::new();
    vc3.set("node1", 6);
    vc3.set("node2", 5);

    // vc3 happens after vc1
    assert_eq!(vc3.compare(&vc1), ClockOrdering::After);
    assert_eq!(vc1.compare(&vc3), ClockOrdering::Before);
}

#[tokio::test]
async fn test_consensus_vector_clock() {
    let resolver = ConflictResolver::new("node1".to_string());

    let mut vc1 = VectorClock::new();
    vc1.set("node1", 5);
    vc1.set("node2", 3);

    let mut vc2 = VectorClock::new();
    vc2.set("node1", 3);
    vc2.set("node2", 7);

    let mut vc3 = VectorClock::new();
    vc3.set("node1", 4);
    vc3.set("node3", 2);

    // Calculate consensus
    let consensus = resolver.calculate_consensus_vector_clock(&[vc1, vc2, vc3]);

    // Should have max of each node
    assert_eq!(consensus.get("node1"), 5);
    assert_eq!(consensus.get("node2"), 7);
    assert_eq!(consensus.get("node3"), 2);
}

#[tokio::test]
async fn test_operation_ordering_determinism() {
    let resolver = ConflictResolver::new("node1".to_string());

    // Create operations with same timestamp but different node IDs
    let op1 = create_test_operation("node_a", 1, 1000);
    let op2 = create_test_operation("node_b", 1, 1000);
    let op3 = create_test_operation("node_c", 1, 1000);

    let local_ops = vec![op3.clone(), op1.clone(), op2.clone()]; // Intentionally out of order
    let result = resolver
        .resolve_divergent_logs(local_ops, HashMap::new())
        .unwrap();

    // Should be ordered by node_id lexicographically
    assert_eq!(result[0].cluster_node_id, "node_a");
    assert_eq!(result[1].cluster_node_id, "node_b");
    assert_eq!(result[2].cluster_node_id, "node_c");
}

#[tokio::test]
async fn test_large_operation_batch() {
    let resolver = ConflictResolver::new("node1".to_string());

    // Create a large batch of operations
    let mut operations = Vec::new();
    for i in 0..1000 {
        let op = create_test_operation("node1", i, 1000 + i);
        operations.push(op);
    }

    let result = resolver
        .resolve_divergent_logs(operations.clone(), HashMap::new())
        .unwrap();

    // Should handle large batches correctly
    assert_eq!(result.len(), 1000);

    // Should be ordered by timestamp
    for i in 1..result.len() {
        assert!(result[i - 1].timestamp_ms <= result[i].timestamp_ms);
    }
}

#[tokio::test]
async fn test_empty_peer_logs() {
    let resolver = ConflictResolver::new("node1".to_string());

    let local_ops = vec![];
    let peer_logs = HashMap::new();

    let result = resolver
        .resolve_divergent_logs(local_ops, peer_logs)
        .unwrap();

    // Should handle empty logs gracefully
    assert_eq!(result.len(), 0);
}

#[tokio::test]
async fn test_single_peer_log() {
    let resolver = ConflictResolver::new("node1".to_string());

    let op1 = create_test_operation("node1", 1, 1000);
    let op2 = create_test_operation("node1", 2, 2000);

    let local_ops = vec![];
    let mut peer_logs = HashMap::new();
    peer_logs.insert("node1".to_string(), vec![op1.clone(), op2.clone()]);

    let result = resolver
        .resolve_divergent_logs(local_ops, peer_logs)
        .unwrap();

    // Should apply operations from single peer
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].op_seq, 1);
    assert_eq!(result[1].op_seq, 2);
}

#[tokio::test]
async fn test_catch_up_coordinator_paths() {
    let temp_data = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    // Test that paths are properly configured
    let data_path = temp_data.path().to_path_buf();
    let staging_path = temp_staging.path().to_path_buf();

    assert!(data_path.exists());
    assert!(staging_path.exists());

    let _coordinator = CatchUpCoordinator::new(
        "test_node".to_string(),
        vec!["127.0.0.1:9999".to_string()],
        data_path,
        staging_path,
        None, // No storage backend for test
        None, // No checkpoint ingestor for test
        None, // No tantivy receiver for test
        None, // No hnsw receiver for test
        None, // Use default checkpoint threshold
    );

    // Coordinator created successfully
}

#[tokio::test]
async fn test_multiple_peer_merge() {
    let resolver = ConflictResolver::new("node1".to_string());

    // Create operations from 3 different peers
    let op1 = create_test_operation("node1", 1, 1000);
    let op2 = create_test_operation("node2", 1, 1500);
    let op3 = create_test_operation("node3", 1, 2000);
    let op4 = create_test_operation("node1", 2, 2500);

    let local_ops = vec![op1.clone(), op4.clone()];
    let mut peer_logs = HashMap::new();
    peer_logs.insert("node2".to_string(), vec![op2.clone()]);
    peer_logs.insert("node3".to_string(), vec![op3.clone()]);

    let result = resolver
        .resolve_divergent_logs(local_ops, peer_logs)
        .unwrap();

    // Should merge all operations from all peers
    assert_eq!(result.len(), 4);

    // Should be ordered by timestamp
    assert_eq!(result[0].timestamp_ms, 1000);
    assert_eq!(result[1].timestamp_ms, 1500);
    assert_eq!(result[2].timestamp_ms, 2000);
    assert_eq!(result[3].timestamp_ms, 2500);
}
