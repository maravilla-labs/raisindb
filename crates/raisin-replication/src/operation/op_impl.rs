use chrono::Utc;
use std::collections::HashSet;
use uuid::Uuid;

use super::{OpType, Operation, OperationTarget};
use crate::vector_clock::VectorClock;

impl Operation {
    /// Create a new operation with the current timestamp
    ///
    /// # Arguments
    /// * `op_seq` - Sequence number for this operation
    /// * `cluster_node_id` - Cluster node ID (server instance) that originated this operation
    pub fn new(
        op_seq: u64,
        cluster_node_id: String, // Cluster node ID
        vector_clock: VectorClock,
        tenant_id: String,
        repo_id: String,
        branch: String,
        op_type: OpType,
        actor: String,
    ) -> Self {
        Self {
            op_id: Uuid::new_v4(),
            op_seq,
            cluster_node_id,
            timestamp_ms: Utc::now().timestamp_millis() as u64,
            vector_clock,
            tenant_id,
            repo_id,
            branch,
            op_type,
            revision: None, // For this constructor, revision must be set separately
            actor,
            message: None,
            is_system: false,
            acknowledged_by: HashSet::new(),
        }
    }

    /// Get the target of this operation (what it modifies)
    pub fn target(&self) -> OperationTarget {
        match &self.op_type {
            OpType::CreateNode { node_id, .. }
            | OpType::DeleteNode { node_id }
            | OpType::SetProperty { node_id, .. }
            | OpType::DeleteProperty { node_id, .. }
            | OpType::RenameNode { node_id, .. }
            | OpType::SetArchetype { node_id, .. }
            | OpType::SetOrderKey { node_id, .. }
            | OpType::SetOwner { node_id, .. }
            | OpType::PublishNode { node_id, .. }
            | OpType::UnpublishNode { node_id }
            | OpType::SetTranslation { node_id, .. }
            | OpType::DeleteTranslation { node_id, .. }
            | OpType::MoveNode { node_id, .. }
            | OpType::ListInsertAfter { node_id, .. }
            | OpType::ListDelete { node_id, .. } => OperationTarget::Node(node_id.clone()),
            OpType::UpsertNodeSnapshot { node, .. } => OperationTarget::Node(node.id.clone()),
            OpType::DeleteNodeSnapshot { node_id, .. } => OperationTarget::Node(node_id.clone()),
            OpType::AddRelation { source_id, .. } | OpType::RemoveRelation { source_id, .. } => {
                OperationTarget::Node(source_id.clone())
            }
            OpType::UpdateNodeType { node_type_id, .. }
            | OpType::DeleteNodeType { node_type_id } => {
                OperationTarget::NodeType(node_type_id.clone())
            }
            OpType::UpdateArchetype { archetype_id, .. }
            | OpType::DeleteArchetype { archetype_id } => {
                OperationTarget::Archetype(archetype_id.clone())
            }
            OpType::UpdateElementType {
                element_type_id, ..
            }
            | OpType::DeleteElementType { element_type_id } => {
                OperationTarget::ElementType(element_type_id.clone())
            }
            OpType::UpdateWorkspace { workspace_id, .. }
            | OpType::DeleteWorkspace { workspace_id } => {
                OperationTarget::Workspace(workspace_id.clone())
            }
            OpType::UpdateBranch { branch } => OperationTarget::Branch(branch.name.clone()),
            OpType::CreateRevisionMeta { revision_meta } => {
                OperationTarget::Branch(revision_meta.branch.clone())
            }
            OpType::DeleteBranch { branch_id } => OperationTarget::Branch(branch_id.clone()),
            OpType::CreateTag { tag_name, .. } | OpType::DeleteTag { tag_name } => {
                OperationTarget::Tag(tag_name.clone())
            }
            OpType::UpdateUser { user_id, .. } | OpType::DeleteUser { user_id } => {
                OperationTarget::User(user_id.clone())
            }
            OpType::UpdateTenant { tenant_id, .. } | OpType::DeleteTenant { tenant_id } => {
                OperationTarget::Tenant(tenant_id.clone())
            }
            OpType::UpdateDeployment { deployment_id, .. }
            | OpType::DeleteDeployment { deployment_id } => {
                OperationTarget::Deployment(deployment_id.clone())
            }
            OpType::UpdateRepository { repo_id, .. } | OpType::DeleteRepository { repo_id, .. } => {
                OperationTarget::Repository(repo_id.clone())
            }
            OpType::ApplyRevision { .. } => OperationTarget::Branch(self.branch.clone()),
            OpType::GrantPermission {
                subject_id,
                resource_id,
                ..
            }
            | OpType::RevokePermission {
                subject_id,
                resource_id,
                ..
            } => OperationTarget::Permission(format!("{}:{}", subject_id, resource_id)),
            OpType::UpsertIdentity { identity_id, .. } | OpType::DeleteIdentity { identity_id } => {
                OperationTarget::Identity(identity_id.clone())
            }
            OpType::CreateSession { session_id, .. }
            | OpType::RevokeSession { session_id }
            | OpType::RotateRefreshToken { session_id, .. } => {
                OperationTarget::Session(session_id.clone())
            }
            OpType::RevokeAllIdentitySessions { identity_id } => {
                OperationTarget::Identity(identity_id.clone())
            }
        }
    }

    /// Check if this operation is a delete operation
    pub fn is_delete(&self) -> bool {
        matches!(
            self.op_type,
            OpType::DeleteNode { .. }
                | OpType::DeleteProperty { .. }
                | OpType::DeleteTranslation { .. }
                | OpType::RemoveRelation { .. }
                | OpType::ListDelete { .. }
                | OpType::DeleteNodeType { .. }
                | OpType::DeleteArchetype { .. }
                | OpType::DeleteElementType { .. }
                | OpType::DeleteWorkspace { .. }
                | OpType::DeleteBranch { .. }
                | OpType::DeleteTag { .. }
                | OpType::DeleteUser { .. }
                | OpType::DeleteTenant { .. }
                | OpType::DeleteDeployment { .. }
                | OpType::UnpublishNode { .. }
                | OpType::RevokePermission { .. }
                | OpType::DeleteIdentity { .. }
                | OpType::RevokeSession { .. }
                | OpType::RevokeAllIdentitySessions { .. }
        )
    }

    /// Mark this operation as acknowledged by a peer
    pub fn acknowledge(&mut self, peer_id: &str) {
        self.acknowledged_by.insert(peer_id.to_string());
    }

    /// Check if this operation has been acknowledged by all given peers
    pub fn acknowledged_by_all(&self, peer_ids: &[String]) -> bool {
        peer_ids.iter().all(|id| self.acknowledged_by.contains(id))
    }

    /// Get the age of this operation in days
    pub fn age_days(&self) -> u64 {
        let now_ms = Utc::now().timestamp_millis() as u64;
        if now_ms > self.timestamp_ms {
            (now_ms - self.timestamp_ms) / (1000 * 60 * 60 * 24)
        } else {
            0
        }
    }

    /// Create a timestamp from the current time
    pub fn current_timestamp_ms() -> u64 {
        Utc::now().timestamp_millis() as u64
    }
}
