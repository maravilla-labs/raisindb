#[cfg(test)]
mod tests {
    use crate::causal_delivery::{BufferStats, CausalDeliveryBuffer};
    use crate::{OpType, Operation, VectorClock};
    use raisin_models::nodes::properties::PropertyValue;
    use uuid::Uuid;

    fn make_test_op(cluster_node_id: &str, op_seq: u64, vc: VectorClock) -> Operation {
        Operation {
            op_id: Uuid::new_v4(),
            op_seq,
            cluster_node_id: cluster_node_id.to_string(),
            timestamp_ms: 1000 + op_seq * 1000,
            vector_clock: vc,
            tenant_id: "tenant1".to_string(),
            repo_id: "repo1".to_string(),
            branch: "main".to_string(),
            op_type: OpType::SetProperty {
                node_id: "test".to_string(),
                property_name: "value".to_string(),
                value: PropertyValue::Integer(op_seq as i64),
            },
            revision: None,
            actor: "test".to_string(),
            message: None,
            is_system: false,
            acknowledged_by: Default::default(),
        }
    }

    #[test]
    fn test_direct_delivery_when_dependencies_satisfied() {
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);

        let mut vc = VectorClock::new();
        vc.set("node1", 1);

        let op = make_test_op("node1", 1, vc);
        let delivered = buffer.deliver(op.clone());

        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].op_id, op.op_id);
        assert_eq!(buffer.buffer_size(), 0);
        assert_eq!(buffer.stats().direct_deliveries, 1);
    }

    #[test]
    fn test_buffer_when_dependencies_missing() {
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);

        // Create operation that depends on node1:1 (which we haven't seen)
        let mut vc = VectorClock::new();
        vc.set("node1", 2); // Depends on previous operation

        let op = make_test_op("node1", 2, vc);
        let delivered = buffer.deliver(op.clone());

        assert_eq!(delivered.len(), 0);
        assert_eq!(buffer.buffer_size(), 1);
        assert_eq!(buffer.stats().dependency_waits, 1);
    }

    #[test]
    fn test_cascading_delivery_from_buffer() {
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);

        // Create operations: op1 -> op2 -> op3
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 2);

        let mut vc3 = VectorClock::new();
        vc3.set("node1", 3);

        let op1 = make_test_op("node1", 1, vc1);
        let op2 = make_test_op("node1", 2, vc2);
        let op3 = make_test_op("node1", 3, vc3);

        // Deliver out of order: op3, op2, op1
        let delivered = buffer.deliver(op3.clone());
        assert_eq!(delivered.len(), 0); // Buffered (missing dependencies)

        let delivered = buffer.deliver(op2.clone());
        assert_eq!(delivered.len(), 0); // Buffered (missing dependencies)

        // Now deliver op1 - should trigger cascading delivery
        let delivered = buffer.deliver(op1.clone());
        assert_eq!(delivered.len(), 3); // All three delivered!

        // Check order: op1, op2, op3
        assert_eq!(delivered[0].op_seq, 1);
        assert_eq!(delivered[1].op_seq, 2);
        assert_eq!(delivered[2].op_seq, 3);

        assert_eq!(buffer.buffer_size(), 0); // Buffer empty
    }

    #[test]
    fn test_concurrent_operations_from_different_nodes() {
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);

        // Two concurrent operations from different nodes
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);

        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);

        let op1 = make_test_op("node1", 1, vc1);
        let op2 = make_test_op("node2", 1, vc2);

        // Both should be delivered immediately (no dependencies)
        let delivered = buffer.deliver(op1.clone());
        assert_eq!(delivered.len(), 1);

        let delivered = buffer.deliver(op2.clone());
        assert_eq!(delivered.len(), 1);

        assert_eq!(buffer.buffer_size(), 0);
    }

    #[test]
    fn test_operation_depending_on_multiple_nodes() {
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);

        // Create op1 from node1
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);
        let op1 = make_test_op("node1", 1, vc1);

        // Create op2 from node2
        let mut vc2 = VectorClock::new();
        vc2.set("node2", 1);
        let op2 = make_test_op("node2", 1, vc2);

        // Create op3 that depends on both
        let mut vc3 = VectorClock::new();
        vc3.set("node1", 1);
        vc3.set("node2", 1);
        vc3.set("node3", 1);
        let op3 = make_test_op("node3", 1, vc3);

        // Deliver op3 first - should buffer (missing dependencies)
        let delivered = buffer.deliver(op3.clone());
        assert_eq!(delivered.len(), 0);
        assert_eq!(buffer.buffer_size(), 1);

        // Deliver op1 - op3 still waiting for op2
        let delivered = buffer.deliver(op1.clone());
        assert_eq!(delivered.len(), 1); // Only op1
        assert_eq!(buffer.buffer_size(), 1); // op3 still buffered

        // Deliver op2 - now op3 can be delivered
        let delivered = buffer.deliver(op2.clone());
        assert_eq!(delivered.len(), 2); // op2 + op3

        assert_eq!(buffer.buffer_size(), 0);
    }

    #[test]
    fn test_buffer_stats() {
        let mut buffer = CausalDeliveryBuffer::new(VectorClock::new(), None);

        let mut vc1 = VectorClock::new();
        vc1.set("node1", 1);
        let op1 = make_test_op("node1", 1, vc1);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 2);
        let op2 = make_test_op("node1", 2, vc2);

        // Deliver op2 (buffered)
        buffer.deliver(op2);
        assert_eq!(buffer.stats().total_buffered, 1);
        assert_eq!(buffer.stats().dependency_waits, 1);
        assert_eq!(buffer.stats().current_buffered, 1);

        // Deliver op1 (triggers op2 delivery)
        buffer.deliver(op1);
        assert_eq!(buffer.stats().total_delivered, 2);
        assert_eq!(buffer.stats().current_buffered, 0);
        assert_eq!(buffer.stats().direct_deliveries, 1);
    }
}
