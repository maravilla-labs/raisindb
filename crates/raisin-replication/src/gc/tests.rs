#[cfg(test)]
mod tests {
    use crate::gc::{GarbageCollector, GcConfig, GcStrategy, PeerWatermarks};
    use crate::operation::Operation;
    use crate::{OpType, VectorClock};

    fn make_test_op(
        node_id: &str,
        op_seq: u64,
        timestamp_ms: u64,
        acknowledged_by: Vec<&str>,
    ) -> Operation {
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
                node_id: "target".to_string(),
                property_name: "title".to_string(),
                value: raisin_models::nodes::properties::PropertyValue::String("value".to_string()),
            },
            revision: None,
            actor: "test".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: acknowledged_by.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_watermark_update() {
        let mut watermarks = PeerWatermarks::new();

        watermarks.update("node1".to_string(), 5);
        watermarks.update("node2".to_string(), 3);
        watermarks.update("node1".to_string(), 10); // Should update to 10

        assert_eq!(watermarks.get_watermark("node1"), 10);
        assert_eq!(watermarks.get_watermark("node2"), 3);
        assert_eq!(watermarks.min_watermark(), 3);
    }

    #[test]
    fn test_watermark_min() {
        let mut watermarks = PeerWatermarks::new();

        watermarks.update("node1".to_string(), 100);
        watermarks.update("node2".to_string(), 50);
        watermarks.update("node3".to_string(), 75);

        assert_eq!(watermarks.min_watermark(), 50);
    }

    #[test]
    fn test_watermark_remove_peer() {
        let mut watermarks = PeerWatermarks::new();

        watermarks.update("node1".to_string(), 100);
        watermarks.update("node2".to_string(), 50);

        assert_eq!(watermarks.min_watermark(), 50);

        watermarks.remove_peer("node2");
        assert_eq!(watermarks.min_watermark(), 100);
    }

    #[test]
    fn test_acknowledgment_based_gc() {
        let mut gc = GarbageCollector::new();

        // Setup watermarks
        gc.watermarks_mut().update("peer1".to_string(), 10);
        gc.watermarks_mut().update("peer2".to_string(), 5);

        // Create operations
        let ops = vec![
            make_test_op("node1", 3, 1000, vec!["peer1", "peer2"]), // Can delete (seq <= 5)
            make_test_op("node1", 5, 2000, vec!["peer1", "peer2"]), // Can delete (seq <= 5)
            make_test_op("node1", 8, 3000, vec!["peer1", "peer2"]), // Cannot delete (seq > 5)
        ];

        let (to_delete, strategy) = gc.collect(&ops, 0);

        assert_eq!(strategy, GcStrategy::AcknowledgmentBased);
        assert_eq!(to_delete.len(), 2);
        assert!(to_delete.contains(&ops[0].op_id));
        assert!(to_delete.contains(&ops[1].op_id));
    }

    #[test]
    fn test_time_based_gc() {
        let gc = GarbageCollector::with_config(GcConfig {
            max_age_days: 30,
            ..Default::default()
        });

        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let old_ms = now_ms - (31 * 24 * 60 * 60 * 1000); // 31 days ago
        let recent_ms = now_ms - (10 * 24 * 60 * 60 * 1000); // 10 days ago

        let ops = vec![
            make_test_op("node1", 1, old_ms, vec![]), // Should delete (too old)
            make_test_op("node1", 2, recent_ms, vec![]), // Should keep (recent)
        ];

        let (to_delete, strategy) = gc.collect(&ops, 0);

        assert_eq!(strategy, GcStrategy::TimeBasedFailsafe);
        assert_eq!(to_delete.len(), 1);
        assert!(to_delete.contains(&ops[0].op_id));
    }

    #[test]
    fn test_emergency_gc() {
        let gc = GarbageCollector::with_config(GcConfig {
            max_log_size_bytes: 10_000,
            target_log_size_bytes: 5_000, // Need to reclaim 5KB
            emergency_gc_enabled: true,
            ..Default::default()
        });

        // Create 10 operations
        let ops: Vec<_> = (1..=10)
            .map(|i| make_test_op("node1", i, i * 1000, vec![]))
            .collect();

        // Trigger emergency GC by passing size over limit
        // With 10 ops, avg size = 10000/10 = 1000 bytes
        // Need to reclaim 5000 bytes = 5 operations
        let (to_delete, strategy) = gc.collect(&ops, 10_001);

        assert_eq!(strategy, GcStrategy::Emergency);
        assert_eq!(to_delete.len(), 5); // Should delete oldest 5 operations

        // Verify it deletes oldest operations (sorted by timestamp)
        for i in 0..5 {
            assert!(to_delete.contains(&ops[i].op_id));
        }
    }

    #[test]
    fn test_min_peer_acknowledgments() {
        let mut gc = GarbageCollector::with_config(GcConfig {
            min_peer_acknowledgments: 2, // Only need 2 peers
            ..Default::default()
        });

        // Setup 3 peers, but min_watermark requires all 3
        gc.watermarks_mut().update("peer1".to_string(), 10);
        gc.watermarks_mut().update("peer2".to_string(), 10);
        gc.watermarks_mut().update("peer3".to_string(), 5);

        let ops = vec![
            make_test_op("node1", 3, 1000, vec!["peer1", "peer2"]), // 2 acks, can delete
            make_test_op("node1", 6, 2000, vec!["peer1"]),          // 1 ack, cannot delete
        ];

        let (to_delete, _) = gc.collect(&ops, 0);

        assert_eq!(to_delete.len(), 1);
        assert!(to_delete.contains(&ops[0].op_id));
    }

    #[test]
    fn test_update_watermarks_from_operations() {
        let mut gc = GarbageCollector::new();

        let ops = vec![
            make_test_op("node1", 5, 1000, vec!["peer1", "peer2"]),
            make_test_op("node1", 10, 2000, vec!["peer1"]),
            make_test_op("node1", 8, 3000, vec!["peer2", "peer3"]),
        ];

        gc.update_watermarks_from_operations(&ops);

        assert_eq!(gc.watermarks().get_watermark("peer1"), 10);
        assert_eq!(gc.watermarks().get_watermark("peer2"), 8);
        assert_eq!(gc.watermarks().get_watermark("peer3"), 8);
    }

    #[test]
    fn test_strategy_determination() {
        let gc = GarbageCollector::with_config(GcConfig {
            max_log_size_bytes: 1000,
            emergency_gc_enabled: true,
            ..Default::default()
        });

        // Normal size - should use acknowledgment-based
        // (strategy determination is tested through collect method behavior)
        let ops: Vec<Operation> = vec![];
        let (_, strategy) = gc.collect(&ops, 500);
        assert_eq!(strategy, GcStrategy::TimeBasedFailsafe); // No ops, falls through to time-based

        // Over limit - should use emergency
        let (_, strategy) = gc.collect(&ops, 1001);
        assert_eq!(strategy, GcStrategy::Emergency);
    }
}
