/// Operation priority sorting for cluster replication
///
/// This module provides priority-based ordering of operations to ensure
/// critical operations (like admin user sync) are applied before others.
use crate::operation::{OpType, Operation};
use std::cmp::Ordering;

/// Priority levels for operation types
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperationPriority {
    /// Critical: Admin users, permissions, tenant config (Priority 1)
    Critical = 1,
    /// High: Workspaces, branches, tags, node types (Priority 2)
    High = 2,
    /// Medium: Whole revisions and revision metadata (Priority 3)
    Medium = 3,
    /// Low: Node snapshots, relations, translations (Priority 4)
    Low = 4,
}

impl OperationPriority {
    /// Determine priority level for an operation
    pub fn from_operation(op: &Operation) -> Self {
        match &op.op_type {
            // Critical: Admin user, tenant, deployment, identity, session operations
            OpType::UpdateUser { .. }
            | OpType::DeleteUser { .. }
            | OpType::UpdateTenant { .. }
            | OpType::DeleteTenant { .. }
            | OpType::UpdateDeployment { .. }
            | OpType::DeleteDeployment { .. }
            | OpType::UpdateRepository { .. }
            | OpType::DeleteRepository { .. }
            | OpType::GrantPermission { .. }
            | OpType::RevokePermission { .. }
            | OpType::UpsertIdentity { .. }
            | OpType::DeleteIdentity { .. }
            | OpType::CreateSession { .. }
            | OpType::RevokeSession { .. }
            | OpType::RevokeAllIdentitySessions { .. }
            | OpType::UpsertOAuthClient { .. }
            | OpType::DeleteOAuthClient { .. }
            | OpType::UpsertOAuthRefreshToken { .. }
            | OpType::RevokeOAuthRefreshFamily { .. }
            | OpType::UpsertApiKey { .. }
            // A secret must never be applied AFTER the node that references it:
            // the reference would resolve to nothing until the next delivery.
            // Critical is the only class that cannot be reordered behind the
            // Medium/Low node operations by `sort_operations_by_priority`.
            | OpType::UpsertSecret { .. }
            | OpType::RotateRefreshToken { .. } => OperationPriority::Critical,

            // High: Schema and workspace structure
            OpType::UpdateWorkspace { .. }
            | OpType::DeleteWorkspace { .. }
            | OpType::UpdateBranch { .. }
            | OpType::DeleteBranch { .. }
            | OpType::CreateTag { .. }
            | OpType::DeleteTag { .. }
            | OpType::UpdateNodeType { .. }
            | OpType::DeleteNodeType { .. }
            | OpType::UpdateArchetype { .. }
            | OpType::DeleteArchetype { .. }
            | OpType::UpdateElementType { .. }
            | OpType::DeleteElementType { .. } => OperationPriority::High,

            // Medium: whole revisions, revision metadata
            OpType::ApplyRevision { .. } | OpType::CreateRevisionMeta { .. } => {
                OperationPriority::Medium
            }

            // Low: node snapshots, relations, translations
            OpType::AddRelation { .. }
            | OpType::RemoveRelation { .. }
            | OpType::UpsertTranslationOverlay { .. }
            | OpType::Unknown { .. }
            | OpType::UpsertNodeSnapshot { .. }
            | OpType::DeleteNodeSnapshot { .. } => OperationPriority::Low,
        }
    }
}

/// Sort operations by priority (highest priority first)
///
/// Within the same priority level, operations maintain their original order
/// to preserve causality.
pub fn sort_operations_by_priority(operations: &mut [Operation]) {
    operations.sort_by(|a, b| {
        let priority_a = OperationPriority::from_operation(a);
        let priority_b = OperationPriority::from_operation(b);

        // Lower numeric value = higher priority (Critical=1, Low=4)
        match priority_a.cmp(&priority_b) {
            Ordering::Equal => {
                // Same priority: maintain order by op_seq (causality)
                a.op_seq.cmp(&b.op_seq)
            }
            other => other,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VectorClock;
    use raisin_models::nodes::RelationRef;
    use std::collections::HashSet;
    use uuid::Uuid;

    fn create_test_op(op_type: OpType, op_seq: u64) -> Operation {
        Operation {
            op_id: Uuid::new_v4(),
            op_seq,
            timestamp_ms: 0,
            tenant_id: "test".to_string(),
            repo_id: "test".to_string(),
            branch: "main".to_string(),
            actor: "test".to_string(),
            cluster_node_id: "node1".to_string(),
            vector_clock: VectorClock::new(),
            message: None,
            is_system: false,
            agent: None,
            acknowledged_by: HashSet::new(),
            op_type,
            revision: None,
        }
    }

    #[test]
    fn test_priority_ordering() {
        let admin_op = create_test_op(
            OpType::UpdateUser {
                user_id: "user1".to_string(),
                user: raisin_models::admin_user::DatabaseAdminUser {
                    user_id: "user1".to_string(),
                    username: "testuser".to_string(),
                    email: Some("test@example.com".to_string()),
                    password_hash: "hash".to_string(),
                    tenant_id: "test".to_string(),
                    access_flags: raisin_models::admin_user::AdminAccessFlags::default(),
                    must_change_password: false,
                    created_at: raisin_models::timestamp::StorageTimestamp::now(),
                    last_login: None,
                    is_active: true,
                },
            },
            100,
        );

        let workspace_op = create_test_op(
            OpType::UpdateWorkspace {
                workspace_id: "ws1".to_string(),
                workspace: raisin_models::workspace::Workspace {
                    name: "Workspace".to_string(),
                    description: Some("Test workspace".to_string()),
                    allowed_node_types: vec![],
                    allowed_root_node_types: vec![],
                    depends_on: vec![],
                    initial_structure: None,
                    created_at: raisin_models::timestamp::StorageTimestamp::now(),
                    updated_at: None,
                    config: raisin_models::workspace::WorkspaceConfig::default(),
                    compound_indexes: None,
                },
            },
            101,
        );

        let node_op = create_test_op(
            OpType::ApplyRevision {
                branch_head: raisin_hlc::HLC::new(1, 0),
                node_changes: vec![],
            },
            102,
        );

        let property_op = create_test_op(
            OpType::DeleteNodeSnapshot {
                node_id: "node1".to_string(),
                revision: raisin_hlc::HLC::new(1, 0),
                node: None,
                parent_id: None,
            },
            103,
        );

        // Check priority assignment
        assert_eq!(
            OperationPriority::from_operation(&admin_op),
            OperationPriority::Critical
        );
        assert_eq!(
            OperationPriority::from_operation(&workspace_op),
            OperationPriority::High
        );
        assert_eq!(
            OperationPriority::from_operation(&node_op),
            OperationPriority::Medium
        );
        assert_eq!(
            OperationPriority::from_operation(&property_op),
            OperationPriority::Low
        );

        // Test sorting
        let mut ops = vec![
            property_op.clone(),
            node_op.clone(),
            workspace_op.clone(),
            admin_op.clone(),
        ];

        sort_operations_by_priority(&mut ops);

        // Should be sorted: admin -> workspace -> revision -> node snapshot
        assert!(matches!(ops[0].op_type, OpType::UpdateUser { .. }));
        assert!(matches!(ops[1].op_type, OpType::UpdateWorkspace { .. }));
        assert!(matches!(ops[2].op_type, OpType::ApplyRevision { .. }));
        assert!(matches!(ops[3].op_type, OpType::DeleteNodeSnapshot { .. }));
    }

    #[test]
    fn test_same_priority_maintains_order() {
        let op1 = create_test_op(
            OpType::DeleteNodeSnapshot {
                node_id: "node-a".to_string(),
                revision: raisin_hlc::HLC::new(1, 0),
                node: None,
                parent_id: None,
            },
            100,
        );

        let op2 = create_test_op(
            OpType::DeleteNodeSnapshot {
                node_id: "node-b".to_string(),
                revision: raisin_hlc::HLC::new(1, 0),
                node: None,
                parent_id: None,
            },
            101,
        );

        let op3 = create_test_op(
            OpType::AddRelation {
                source_id: "node1".to_string(),
                source_workspace: "ws".to_string(),
                relation_type: "ref".to_string(),
                target_id: "node2".to_string(),
                target_workspace: "ws".to_string(),
                relation: RelationRef::new(
                    "node2".to_string(),
                    "ws".to_string(),
                    "".to_string(),
                    "ref".to_string(),
                    None,
                ),
            },
            102,
        );

        let mut ops = vec![op3.clone(), op1.clone(), op2.clone()];
        sort_operations_by_priority(&mut ops);

        // Same priority (Low), should maintain op_seq order: 100, 101, 102
        assert_eq!(ops[0].op_seq, 100);
        assert_eq!(ops[1].op_seq, 101);
        assert_eq!(ops[2].op_seq, 102);
    }
}
