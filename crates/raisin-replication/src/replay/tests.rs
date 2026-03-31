#[cfg(test)]
mod tests {
    use crate::crdt::ConflictType;
    use crate::operation::OperationTarget;
    use crate::replay::ReplayEngine;
    use crate::{OpType, Operation, VectorClock};
    use raisin_models::nodes::properties::PropertyValue;
    use std::collections::HashSet;
    use uuid::Uuid;

    fn make_test_op(
        node_id: &str,
        op_seq: u64,
        vc: VectorClock,
        timestamp_ms: u64,
        target_node_id: &str,
    ) -> Operation {
        Operation {
            op_id: Uuid::new_v4(),
            op_seq,
            cluster_node_id: node_id.to_string(),
            timestamp_ms,
            vector_clock: vc,
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: target_node_id.to_string(),
                property_name: "title".to_string(),
                value: PropertyValue::String(format!("Value from {}", node_id).to_string()),
            },
            revision: None,
            actor: "test".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: HashSet::new(),
        }
    }

    #[test]
    fn test_causal_sort_simple() {
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 2);

        let mut vc3 = VectorClock::new();
        vc3.set("node1", 3);

        let op1 = make_test_op("node1", 1, vc1, 1000, "target");
        let op2 = make_test_op("node1", 2, vc2, 2000, "target");
        let op3 = make_test_op("node1", 3, vc3, 3000, "target");

        let ops = vec![op3.clone(), op1.clone(), op2.clone()];

        let sorted = ReplayEngine::causal_sort(ops);

        assert_eq!(sorted[0].op_seq, 1);
        assert_eq!(sorted[1].op_seq, 2);
        assert_eq!(sorted[2].op_seq, 3);
    }

    #[test]
    fn test_causal_sort_concurrent() {
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);

        let op1 = make_test_op("node1", 1, vc1, 1000, "target");
        let op2 = make_test_op("node2", 1, vc2, 2000, "target");

        let ops = vec![op2.clone(), op1.clone()];
        let sorted = ReplayEngine::causal_sort(ops);

        assert_eq!(sorted[0].timestamp_ms, 1000);
        assert_eq!(sorted[1].timestamp_ms, 2000);
    }

    #[test]
    fn test_group_by_target() {
        let mut vc = VectorClock::new();
        vc.increment("node1");

        let op1 = make_test_op("node1", 1, vc.clone(), 1000, "target1");
        let op2 = make_test_op("node1", 2, vc.clone(), 2000, "target1");
        let op3 = make_test_op("node1", 3, vc.clone(), 3000, "target2");

        let ops = vec![op1, op2, op3];
        let grouped = ReplayEngine::group_by_target(ops);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped
                .get(&OperationTarget::Node("target1".to_string()))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            grouped
                .get(&OperationTarget::Node("target2".to_string()))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_idempotency() {
        let mut engine = ReplayEngine::new();

        let mut vc = VectorClock::new();
        vc.increment("node1");

        let op = make_test_op("node1", 1, vc, 1000, "target");
        let op_id = op.op_id;

        let result = engine.replay(vec![op.clone()]);
        assert_eq!(result.applied.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let result = engine.replay(vec![op]);
        assert_eq!(result.applied.len(), 0);
        assert_eq!(result.skipped.len(), 1);

        assert!(engine.is_applied(&op_id).unwrap());
    }

    #[test]
    fn test_replay_with_conflicts() {
        let mut engine = ReplayEngine::new();

        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);

        let op1 = make_test_op("node1", 1, vc1, 1000, "target");
        let op2 = make_test_op("node2", 1, vc2, 2000, "target");

        let result = engine.replay(vec![op1, op2]);

        assert_eq!(result.applied.len(), 1);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(
            result.conflicts[0].conflict_type,
            ConflictType::ConcurrentPropertyUpdate
        );
    }

    #[test]
    fn test_complex_causal_chain() {
        let mut vc1_1 = VectorClock::new();
        vc1_1.set("node1", 1);

        let mut vc1_2 = VectorClock::new();
        vc1_2.set("node1", 2);

        let mut vc1_3 = VectorClock::new();
        vc1_3.set("node1", 3);

        let mut vc2_1 = VectorClock::new();
        vc2_1.set("node2", 1);

        let mut vc3_1 = VectorClock::new();
        vc3_1.set("node1", 3);
        vc3_1.set("node2", 1);
        vc3_1.set("node3", 1);

        let op1_1 = make_test_op("node1", 1, vc1_1, 1000, "target");
        let op1_2 = make_test_op("node1", 2, vc1_2, 2000, "target");
        let op1_3 = make_test_op("node1", 3, vc1_3, 3000, "target");
        let op2_1 = make_test_op("node2", 1, vc2_1, 2500, "target");
        let op3_1 = make_test_op("node3", 1, vc3_1, 4000, "target");

        let ops = vec![
            op3_1.clone(),
            op2_1.clone(),
            op1_2.clone(),
            op1_1.clone(),
            op1_3.clone(),
        ];

        let sorted = ReplayEngine::causal_sort(ops);

        assert_eq!(sorted[0].op_id, op1_1.op_id);

        let idx_1_2 = sorted
            .iter()
            .position(|op| op.op_id == op1_2.op_id)
            .unwrap();
        let idx_2_1 = sorted
            .iter()
            .position(|op| op.op_id == op2_1.op_id)
            .unwrap();
        assert!(idx_1_2 < idx_2_1);

        assert_eq!(sorted[4].op_id, op3_1.op_id);
    }
}
