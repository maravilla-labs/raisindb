#[cfg(test)]
mod tests {
    use crate::compaction::{CompactionConfig, OperationLogCompactor};
    use crate::operation::{OpType, Operation};
    use crate::vector_clock::VectorClock;

    fn make_set_property_op(
        cluster_node_id: &str,
        op_seq: u64,
        timestamp_ms: u64,
        storage_node_id: &str,
        property: &str,
        value: &str,
    ) -> Operation {
        let mut vc = VectorClock::new();
        vc.increment(cluster_node_id);

        let mut op = Operation::new(
            op_seq,
            cluster_node_id.to_string(),
            vc,
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::SetProperty {
                node_id: storage_node_id.to_string(),
                property_name: property.to_string(),
                value: raisin_models::nodes::properties::PropertyValue::String(value.to_string()),
            },
            "test@example.com".to_string(),
        );

        op.timestamp_ms = timestamp_ms;
        op
    }

    fn make_delete_node_op(
        cluster_node_id: &str,
        op_seq: u64,
        timestamp_ms: u64,
        storage_node_id: &str,
    ) -> Operation {
        let mut vc = VectorClock::new();
        vc.increment(cluster_node_id);

        let mut op = Operation::new(
            op_seq,
            cluster_node_id.to_string(),
            vc,
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            OpType::DeleteNode {
                node_id: storage_node_id.to_string(),
            },
            "test@example.com".to_string(),
        );

        op.timestamp_ms = timestamp_ms;
        op
    }

    #[test]
    fn test_merge_consecutive_property_updates() {
        let compactor = OperationLogCompactor::default_config();

        let base_time = 1000000u64;
        let current_time = base_time + 7200 * 1000;

        let ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_set_property_op("cluster1", 2, base_time + 1000, "doc123", "title", "v2"),
            make_set_property_op("cluster1", 3, base_time + 2000, "doc123", "title", "v3"),
        ];

        let (compacted, result) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 1);
        assert_eq!(compacted[0].op_seq, 3);
        assert_eq!(result.original_count, 3);
        assert_eq!(result.compacted_count, 1);
        assert_eq!(result.merged_count, 2);
        assert!(result.bytes_saved > 0);
    }

    #[test]
    fn test_different_properties_not_merged() {
        let compactor = OperationLogCompactor::default_config();

        let base_time = 1000000u64;
        let current_time = base_time + 7200 * 1000;

        let ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_set_property_op(
                "cluster1",
                2,
                base_time + 1000,
                "doc123",
                "description",
                "v2",
            ),
            make_set_property_op("cluster1", 3, base_time + 2000, "doc123", "title", "v3"),
        ];

        let (compacted, result) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 2);
        assert_eq!(result.merged_count, 1);

        assert!(compacted.iter().any(|op| {
            matches!(&op.op_type, OpType::SetProperty { property_name, .. } if property_name == "title")
        }));
        assert!(compacted.iter().any(|op| {
            matches!(&op.op_type, OpType::SetProperty { property_name, .. } if property_name == "description")
        }));
    }

    #[test]
    fn test_different_storage_nodes_not_merged() {
        let compactor = OperationLogCompactor::default_config();

        let base_time = 1000000u64;
        let current_time = base_time + 7200 * 1000;

        let ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_set_property_op("cluster1", 2, base_time + 1000, "doc456", "title", "v2"),
        ];

        let (compacted, result) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 2);
        assert_eq!(result.merged_count, 0);
    }

    #[test]
    fn test_recent_operations_preserved() {
        let config = CompactionConfig {
            min_age_secs: 3600,
            merge_property_updates: true,
            batch_size: 100_000,
        };
        let compactor = OperationLogCompactor::new(config);

        let base_time = 1000000u64;
        let current_time = base_time + 1800 * 1000;

        let ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_set_property_op("cluster1", 2, base_time + 1000, "doc123", "title", "v2"),
            make_set_property_op("cluster1", 3, base_time + 2000, "doc123", "title", "v3"),
        ];

        let (compacted, result) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 3);
        assert_eq!(result.merged_count, 0);
    }

    #[test]
    fn test_delete_operations_not_compacted() {
        let compactor = OperationLogCompactor::default_config();

        let base_time = 1000000u64;
        let current_time = base_time + 7200 * 1000;

        let ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_delete_node_op("cluster1", 2, base_time + 1000, "doc123"),
            make_set_property_op("cluster1", 3, base_time + 2000, "doc456", "title", "v2"),
        ];

        let (compacted, result) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 3);
        assert_eq!(result.merged_count, 0);
    }

    #[test]
    fn test_mixed_old_and_recent_operations() {
        let config = CompactionConfig {
            min_age_secs: 3600,
            merge_property_updates: true,
            batch_size: 100_000,
        };
        let compactor = OperationLogCompactor::new(config);

        let base_time = 1000000u64;
        let current_time = base_time + 7200 * 1000;

        let ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_set_property_op("cluster1", 2, base_time + 1000, "doc123", "title", "v2"),
            make_set_property_op(
                "cluster1",
                3,
                current_time - 1800 * 1000,
                "doc123",
                "title",
                "v3",
            ),
        ];

        let (compacted, result) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 2);
        assert_eq!(result.merged_count, 1);
    }

    #[test]
    fn test_empty_operations() {
        let compactor = OperationLogCompactor::default_config();
        let (compacted, result) = compactor.compact_node_operations(vec![], 1000000);

        assert_eq!(compacted.len(), 0);
        assert_eq!(result.original_count, 0);
        assert_eq!(result.compacted_count, 0);
    }

    #[test]
    fn test_vector_clock_preserved_from_latest() {
        let compactor = OperationLogCompactor::default_config();

        let base_time = 1000000u64;
        let current_time = base_time + 7200 * 1000;

        let mut ops = vec![
            make_set_property_op("cluster1", 1, base_time, "doc123", "title", "v1"),
            make_set_property_op("cluster1", 2, base_time + 1000, "doc123", "title", "v2"),
            make_set_property_op("cluster1", 3, base_time + 2000, "doc123", "title", "v3"),
        ];

        ops[0].vector_clock.set("cluster1", 1);
        ops[1].vector_clock.set("cluster1", 2);
        ops[2].vector_clock.set("cluster1", 3);

        let (compacted, _) = compactor.compact_node_operations(ops, current_time);

        assert_eq!(compacted.len(), 1);
        assert_eq!(compacted[0].vector_clock.get("cluster1"), 3);
    }
}
