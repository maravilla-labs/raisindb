//! Tests for operation log functionality

#[cfg(test)]
mod tests {
    use super::super::OpLogRepository;
    use raisin_replication::{OpType, Operation, VectorClock};
    use rocksdb::DB;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup_test_db() -> (Arc<DB>, TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = crate::open_db(temp_dir.path()).unwrap();
        (Arc::new(db), temp_dir)
    }

    fn make_test_operation(node_id: &str, op_seq: u64, timestamp_ms: u64) -> Operation {
        let mut vc = VectorClock::new();
        vc.increment(node_id);

        let mut op = Operation::new(
            op_seq,
            node_id.to_string(),
            vc,
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::CreateNode {
                node_id: format!("node{}", op_seq),
                name: format!("Node {}", op_seq),
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

        // Override timestamp with the provided value for testing
        op.timestamp_ms = timestamp_ms;
        op
    }

    #[test]
    fn test_put_and_get_operation() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        let op = make_test_operation("node1", 1, 1000);
        repo.put_operation(&op).unwrap();

        let retrieved = repo
            .get_operations_from_node("tenant1", "repo1", "node1")
            .unwrap();

        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].op_seq, 1);
        assert_eq!(retrieved[0].cluster_node_id, "node1");
    }

    #[test]
    fn test_get_operations_from_seq() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add multiple operations
        for i in 1..=5 {
            let op = make_test_operation("node1", i, i * 1000);
            repo.put_operation(&op).unwrap();
        }

        // Get operations from sequence 3
        let ops = repo
            .get_operations_from_seq("tenant1", "repo1", "node1", 3)
            .unwrap();

        assert_eq!(ops.len(), 3); // Should get 3, 4, 5
        assert_eq!(ops[0].op_seq, 3);
        assert_eq!(ops[2].op_seq, 5);
    }

    #[test]
    fn test_get_missing_operations() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add operations from two nodes
        for i in 1..=3 {
            let op1 = make_test_operation("node1", i, i * 1000);
            let op2 = make_test_operation("node2", i, i * 1000);
            repo.put_operation(&op1).unwrap();
            repo.put_operation(&op2).unwrap();
        }

        // Create a vector clock that has seen up to seq 2 from both nodes
        let mut vc = VectorClock::new();
        vc.set("node1", 2);
        vc.set("node2", 1);

        let missing = repo
            .get_missing_operations("tenant1", "repo1", &vc, None)
            .unwrap();

        // Should get: node1:3 and node2:2,3
        assert_eq!(missing.len(), 3);
    }

    #[test]
    fn test_get_all_operations_skips_vector_clock_snapshot() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        let op1 = make_test_operation("node1", 1, 1_000);
        let op2 = make_test_operation("node2", 1, 2_000);
        let ops = vec![op1.clone(), op2.clone()];

        // put_operations_batch updates the vector clock snapshot entry
        repo.put_operations_batch(&ops).unwrap();

        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(snapshot.get("node1"), 1);
        assert_eq!(snapshot.get("node2"), 1);

        let all = repo
            .get_all_operations("tenant1", "repo1")
            .expect("should deserialize operations");

        assert_eq!(all.len(), 2);
        assert_eq!(all.get("node1").unwrap()[0].op_id, op1.op_id);
        assert_eq!(all.get("node2").unwrap()[0].op_id, op2.op_id);
    }

    #[test]
    fn test_highest_seq() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add operations
        for i in 1..=5 {
            let op = make_test_operation("node1", i, i * 1000);
            repo.put_operation(&op).unwrap();
        }

        let highest = repo.get_highest_seq("tenant1", "repo1", "node1").unwrap();

        assert_eq!(highest, 5);
    }

    #[test]
    fn test_delete_old_operations() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add operations with different timestamps
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let old_ms = now_ms - (40 * 24 * 60 * 60 * 1000); // 40 days ago

        let old_op = make_test_operation("node1", 1, old_ms);
        let new_op = make_test_operation("node1", 2, now_ms);

        repo.put_operation(&old_op).unwrap();
        repo.put_operation(&new_op).unwrap();

        // Debug: Check what operations exist before deletion
        let all_before = repo.get_all_operations("tenant1", "repo1").unwrap();
        eprintln!("Operations before deletion: {:?}", all_before.len());
        for (node, ops) in &all_before {
            eprintln!("  Node {}: {} operations", node, ops.len());
            for op in ops {
                eprintln!("    seq={}, timestamp_ms={}", op.op_seq, op.timestamp_ms);
            }
        }

        // Delete operations older than 30 days
        let deleted = repo
            .delete_operations_older_than("tenant1", "repo1", 30)
            .unwrap();

        eprintln!("Deleted: {}", deleted);
        assert_eq!(deleted, 1);

        // Verify only new operation remains
        let remaining = repo
            .get_operations_from_node("tenant1", "repo1", "node1")
            .unwrap();

        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].op_seq, 2);
    }

    #[test]
    fn test_stats() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add operations from multiple nodes
        for i in 1..=3 {
            let op1 = make_test_operation("node1", i, i * 1000);
            let op2 = make_test_operation("node2", i, i * 1000);
            repo.put_operation(&op1).unwrap();
            repo.put_operation(&op2).unwrap();
        }

        let stats = repo.get_stats("tenant1", "repo1").unwrap();

        assert_eq!(stats.total_operations, 6);
        assert_eq!(stats.operations_per_node.get("node1"), Some(&3));
        assert_eq!(stats.operations_per_node.get("node2"), Some(&3));
        assert!(stats.oldest_operation_timestamp.is_some());
        assert!(stats.newest_operation_timestamp.is_some());
    }

    // ========================================================================
    // Vector Clock Snapshot Tests
    // ========================================================================

    #[test]
    fn test_vector_clock_snapshot_empty() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Get snapshot for repo with no operations
        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();

        assert!(snapshot.is_empty());
    }

    #[test]
    fn test_vector_clock_snapshot_update() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Create a vector clock
        let mut vc = VectorClock::new();
        vc.set("node1", 5);
        vc.set("node2", 10);

        // Update snapshot
        repo.update_vector_clock_snapshot("tenant1", "repo1", &vc)
            .unwrap();

        // Retrieve and verify
        let retrieved = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();

        assert_eq!(retrieved.get("node1"), 5);
        assert_eq!(retrieved.get("node2"), 10);
    }

    #[test]
    fn test_vector_clock_snapshot_increment() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Increment for node1
        repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", 5)
            .unwrap();

        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(snapshot.get("node1"), 5);

        // Increment again with higher value
        repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", 10)
            .unwrap();

        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(snapshot.get("node1"), 10);

        // Try to increment with lower value (should not change)
        repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", 3)
            .unwrap();

        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(snapshot.get("node1"), 10); // Still 10, not 3
    }

    #[test]
    fn test_vector_clock_snapshot_rebuild() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add operations from multiple nodes
        for i in 1..=5 {
            let op1 = make_test_operation("node1", i, i * 1000);
            repo.put_operation(&op1).unwrap();
        }

        for i in 1..=3 {
            let op2 = make_test_operation("node2", i, i * 1000);
            repo.put_operation(&op2).unwrap();
        }

        // Rebuild snapshot
        let rebuilt = repo
            .rebuild_vector_clock_snapshot("tenant1", "repo1")
            .unwrap();

        assert_eq!(rebuilt.get("node1"), 5);
        assert_eq!(rebuilt.get("node2"), 3);

        // Verify it was persisted
        let retrieved = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();

        assert_eq!(retrieved.get("node1"), 5);
        assert_eq!(retrieved.get("node2"), 3);
    }

    #[test]
    fn test_vector_clock_snapshot_verification_consistent() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Manually create and update snapshot
        for i in 1..=3 {
            repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", i)
                .unwrap();
        }

        // Get snapshot
        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();

        assert_eq!(snapshot.get("node1"), 3);

        // Verify it matches itself
        let snapshot2 = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();

        assert_eq!(snapshot, snapshot2);
    }

    #[test]
    fn test_vector_clock_snapshot_verification_inconsistent() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Create correct snapshot
        repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", 3)
            .unwrap();

        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(snapshot.get("node1"), 3);

        // Manually override with incorrect snapshot
        let mut wrong_vc = VectorClock::new();
        wrong_vc.set("node1", 1); // Wrong value
        repo.update_vector_clock_snapshot("tenant1", "repo1", &wrong_vc)
            .unwrap();

        // Verify it was overridden
        let wrong_snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(wrong_snapshot.get("node1"), 1);

        // Restore correct snapshot
        repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", 3)
            .unwrap();

        let corrected = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        assert_eq!(corrected.get("node1"), 3);
    }

    #[test]
    fn test_put_operations_batch_updates_snapshot() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Create a batch of operations
        let ops = vec![
            make_test_operation("node1", 1, 1000),
            make_test_operation("node1", 2, 2000),
            make_test_operation("node2", 1, 1000),
            make_test_operation("node2", 2, 2000),
            make_test_operation("node2", 3, 3000),
        ];

        // Store batch
        repo.put_operations_batch(&ops).unwrap();

        // Check that snapshot was updated
        let snapshot = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();

        assert_eq!(snapshot.get("node1"), 2);
        assert_eq!(snapshot.get("node2"), 3);
    }

    #[test]
    fn test_vector_clock_snapshot_multi_tenant_isolation() {
        let (db, _temp) = setup_test_db();
        let repo = OpLogRepository::new(db);

        // Add operations for tenant1/repo1
        let op1 = make_test_operation_for_tenant("node1", 1, 1000, "tenant1", "repo1");
        repo.put_operation(&op1).unwrap();
        repo.increment_vector_clock_for_node("tenant1", "repo1", "node1", 1)
            .unwrap();

        // Add operations for tenant2/repo1
        let op2 = make_test_operation_for_tenant("node1", 5, 1000, "tenant2", "repo1");
        repo.put_operation(&op2).unwrap();
        repo.increment_vector_clock_for_node("tenant2", "repo1", "node1", 5)
            .unwrap();

        // Verify isolation
        let snapshot1 = repo.get_vector_clock_snapshot("tenant1", "repo1").unwrap();
        let snapshot2 = repo.get_vector_clock_snapshot("tenant2", "repo1").unwrap();

        assert_eq!(snapshot1.get("node1"), 1);
        assert_eq!(snapshot2.get("node1"), 5);
    }

    // Helper for multi-tenant tests
    fn make_test_operation_for_tenant(
        node_id: &str,
        op_seq: u64,
        timestamp_ms: u64,
        tenant_id: &str,
        repo_id: &str,
    ) -> Operation {
        let mut vc = VectorClock::new();
        vc.increment(node_id);

        let mut op = Operation::new(
            op_seq,
            node_id.to_string(),
            vc,
            tenant_id.to_string(),
            repo_id.to_string(),
            "main".to_string(),
            OpType::CreateNode {
                node_id: format!("node{}", op_seq),
                name: format!("Node {}", op_seq),
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

        // Override timestamp with the provided value for testing
        op.timestamp_ms = timestamp_ms;
        op
    }
}
