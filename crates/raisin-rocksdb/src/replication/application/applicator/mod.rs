//! Operation application layer for replication
//!
//! This module handles applying operations received from peer nodes to the local database.
//! It implements a last-write-wins (LWW) conflict resolution strategy.
//!
//! ## Key Design Decisions
//!
//! 1. **Direct CF Writes**: Operations are applied by writing directly to RocksDB column families,
//!    bypassing the repository layer. This prevents recursive operation capture.
//!
//! 2. **Event Emission**: Even though we bypass repositories, we still emit events so that
//!    application-layer handlers (NodeType initialization, admin user creation, etc.) work correctly.
//!
//! 3. **LWW Conflict Resolution**: When multiple nodes create the same tenant/repository,
//!    the one with the most recent timestamp wins. This is simple but effective for metadata.
//!
//! 4. **Idempotency**: Operations are applied idempotently - applying the same operation
//!    multiple times has the same effect as applying it once.

mod crdt_ops;
mod db_lookups;
mod legacy_node_ops;
mod move_node_ops;
mod registry_ops;
mod relation_ops;
mod schema_ops;
mod user_ops;
mod workspace_branch_ops;

use crate::repositories::BranchRepositoryImpl;
use raisin_error::Result;
use raisin_events::EventBus;
use raisin_hlc::HLC;
use raisin_replication::{OpType, Operation};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

const TOMBSTONE: &[u8] = b"T";

fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

fn node_workspace(node: &raisin_models::nodes::Node) -> &str {
    node.workspace.as_deref().unwrap_or("default")
}

/// Applies operations to the local database
///
/// This is the core of the replication application layer. It receives operations
/// from peer nodes and applies them to the local RocksDB instance.
pub struct OperationApplicator {
    pub(super) db: Arc<DB>,
    pub(super) event_bus: Arc<dyn EventBus>,
    pub(super) branch_repo: Arc<BranchRepositoryImpl>,
}

impl OperationApplicator {
    /// Create a new operation applicator
    pub fn new(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        branch_repo: Arc<BranchRepositoryImpl>,
    ) -> Self {
        Self {
            db,
            event_bus,
            branch_repo,
        }
    }

    /// Extract the revision HLC from an operation
    pub(super) fn op_revision(op: &Operation) -> Result<HLC> {
        if let Some(rev) = op.revision {
            return Ok(rev);
        }

        match &op.op_type {
            OpType::UpsertNodeSnapshot { revision, .. } => Ok(*revision),
            OpType::DeleteNodeSnapshot { revision, .. } => Ok(*revision),
            OpType::ApplyRevision { branch_head, .. } => Ok(*branch_head),
            _ => Err(raisin_error::Error::storage(format!(
                "Operation {} missing revision in both Operation.revision and OpType - cannot apply",
                op.op_id
            ))),
        }
    }

    /// Emit a schema event for a replicated schema operation
    pub(super) fn emit_schema_event(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        schema_id: &str,
        schema_type: &str,
        kind: raisin_events::SchemaEventKind,
    ) {
        use raisin_events::{Event, SchemaEvent};

        let mut metadata = HashMap::new();
        metadata.insert(
            "source".to_string(),
            serde_json::Value::String("replication".to_string()),
        );

        tracing::debug!(
            schema_id = %schema_id,
            schema_type = %schema_type,
            kind = ?kind,
            source = "replication",
            "Emitting schema event for replicated operation"
        );

        let event = SchemaEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            branch: branch.to_string(),
            schema_id: schema_id.to_string(),
            schema_type: schema_type.to_string(),
            kind,
            metadata: Some(metadata),
        };

        self.event_bus.publish(Event::Schema(event));
    }

    /// Apply an operation to the local database
    ///
    /// This is the main entry point. It matches on the operation type and
    /// calls the appropriate handler.
    pub async fn apply_operation(&self, op: &Operation) -> Result<()> {
        tracing::debug!(
            "Applying operation: {} from node {}",
            op.op_id,
            op.cluster_node_id
        );

        if matches!(op.op_type, OpType::UpdateNodeType { .. }) {
            tracing::debug!(
                op_id = %op.op_id,
                tenant_id = %op.tenant_id,
                repo_id = %op.repo_id,
                branch = %op.branch,
                op_seq = op.op_seq,
                cluster_node_id = %op.cluster_node_id,
                "Starting to apply UpdateNodeType operation"
            );
        }

        match &op.op_type {
            // ========== Tenant/Deployment/Repository Operations ==========
            OpType::UpdateTenant { tenant_id, tenant } => {
                self.apply_update_tenant(tenant_id, tenant, op).await
            }
            OpType::UpdateDeployment {
                deployment_id,
                deployment,
            } => {
                self.apply_update_deployment(&deployment.tenant_id, deployment_id, deployment, op)
                    .await
            }
            OpType::UpdateRepository {
                tenant_id,
                repo_id,
                repository,
            } => {
                self.apply_update_repository(tenant_id, repo_id, repository, op)
                    .await
            }

            // ========== Schema Operations ==========
            OpType::UpdateNodeType {
                node_type_id,
                node_type,
            } => {
                self.apply_update_nodetype(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_type_id,
                    node_type,
                    op,
                )
                .await
            }
            OpType::UpdateArchetype {
                archetype_id,
                archetype,
            } => {
                self.apply_update_archetype(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    archetype_id,
                    archetype,
                    op,
                )
                .await
            }
            OpType::UpdateElementType {
                element_type_id,
                element_type,
            } => {
                self.apply_update_element_type(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    element_type_id,
                    element_type,
                    op,
                )
                .await
            }
            OpType::DeleteNodeType { node_type_id } => {
                self.apply_delete_nodetype(&op.tenant_id, &op.repo_id, &op.branch, node_type_id, op)
                    .await
            }
            OpType::DeleteArchetype { archetype_id } => {
                self.apply_delete_archetype(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    archetype_id,
                    op,
                )
                .await
            }
            OpType::DeleteElementType { element_type_id } => {
                self.apply_delete_element_type(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    element_type_id,
                    op,
                )
                .await
            }

            // ========== Node Operations ==========
            OpType::ApplyRevision {
                branch_head,
                node_changes,
            } => {
                self.apply_replicated_revision(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    branch_head,
                    node_changes,
                    op,
                )
                .await
            }
            OpType::CreateNode {
                node_id,
                name,
                node_type,
                archetype,
                parent_id,
                order_key,
                properties,
                owner_id,
                workspace,
                path,
            } => {
                self.apply_create_node(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    workspace.as_ref().map(|s| s.as_str()).unwrap_or("default"),
                    node_id,
                    name,
                    node_type,
                    archetype.as_deref(),
                    parent_id.as_deref(),
                    order_key,
                    properties,
                    owner_id.as_deref(),
                    path,
                    op,
                )
                .await
            }
            OpType::DeleteNode { node_id } => {
                self.apply_delete_node(&op.tenant_id, &op.repo_id, &op.branch, node_id, op)
                    .await
            }
            OpType::SetProperty {
                node_id,
                property_name,
                value,
            } => {
                self.apply_set_property(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_id,
                    property_name,
                    value,
                    op,
                )
                .await
            }
            OpType::RenameNode {
                node_id,
                old_name: _,
                new_name,
            } => {
                self.apply_rename_node(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_id,
                    new_name,
                    op,
                )
                .await
            }
            OpType::MoveNode {
                node_id,
                old_parent_id: _,
                new_parent_id,
                position,
            } => {
                self.apply_move_node(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_id,
                    new_parent_id.as_deref(),
                    position.as_deref(),
                    op,
                )
                .await
            }
            OpType::SetArchetype {
                node_id,
                old_archetype: _,
                new_archetype,
            } => {
                self.apply_set_archetype(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_id,
                    new_archetype.as_deref(),
                    op,
                )
                .await
            }
            OpType::AddRelation {
                source_id,
                source_workspace,
                relation_type,
                target_id,
                target_workspace,
                relation,
            } => {
                self.apply_add_relation(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    source_id,
                    source_workspace,
                    relation_type,
                    target_id,
                    target_workspace,
                    relation.clone(),
                    op,
                )
                .await
            }
            OpType::RemoveRelation {
                source_id,
                source_workspace,
                relation_type,
                target_id,
                target_workspace,
            } => {
                self.apply_remove_relation(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    source_id,
                    source_workspace,
                    relation_type,
                    target_id,
                    target_workspace,
                    op,
                )
                .await
            }

            // ========== User Operations ==========
            OpType::UpdateUser { user_id, user } => {
                self.apply_update_user(&op.tenant_id, user_id, user, op)
                    .await
            }
            OpType::DeleteUser { user_id } => {
                self.apply_delete_user(&op.tenant_id, user_id, op).await
            }

            // ========== Workspace/Branch/Tag Operations ==========
            OpType::UpdateWorkspace {
                workspace_id,
                workspace,
            } => {
                self.apply_update_workspace(&op.tenant_id, &op.repo_id, workspace_id, workspace, op)
                    .await
            }
            OpType::DeleteWorkspace { workspace_id } => {
                self.apply_delete_workspace(&op.tenant_id, &op.repo_id, workspace_id, op)
                    .await
            }
            OpType::UpdateBranch { branch } => {
                self.apply_update_branch(&op.tenant_id, &op.repo_id, branch, op)
                    .await
            }
            OpType::CreateRevisionMeta { revision_meta } => {
                self.apply_create_revision_meta(&op.tenant_id, &op.repo_id, revision_meta, op)
                    .await
            }
            OpType::DeleteBranch { branch_id } => {
                self.apply_delete_branch(&op.tenant_id, &op.repo_id, branch_id, op)
                    .await
            }
            OpType::CreateTag { tag_name, revision } => {
                self.apply_create_tag(&op.tenant_id, &op.repo_id, tag_name, revision, op)
                    .await
            }
            OpType::DeleteTag { tag_name } => {
                self.apply_delete_tag(&op.tenant_id, &op.repo_id, tag_name, op)
                    .await
            }

            // ========== CRDT Snapshot Operations ==========
            OpType::UpsertNodeSnapshot {
                node,
                parent_id,
                revision,
                cf_order_key,
            } => {
                self.apply_upsert_node_snapshot(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node,
                    parent_id.as_deref(),
                    revision,
                    cf_order_key,
                    op,
                )
                .await
            }
            OpType::DeleteNodeSnapshot { node_id, revision } => {
                self.apply_delete_node_snapshot(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_id,
                    revision,
                    op,
                )
                .await
            }

            // ========== Identity & Session Operations ==========
            OpType::UpsertIdentity {
                identity_id,
                identity,
            } => {
                super::identity_operations::apply_upsert_identity(
                    self,
                    &op.tenant_id,
                    identity_id,
                    identity,
                    op,
                )
                .await
            }
            OpType::DeleteIdentity { identity_id } => {
                super::identity_operations::apply_delete_identity(
                    self,
                    &op.tenant_id,
                    identity_id,
                    op,
                )
                .await
            }
            OpType::CreateSession {
                session_id,
                session,
            } => {
                super::identity_operations::apply_create_session(
                    self,
                    &op.tenant_id,
                    session_id,
                    session,
                    op,
                )
                .await
            }
            OpType::RevokeSession { session_id } => {
                super::identity_operations::apply_revoke_session(
                    self,
                    &op.tenant_id,
                    session_id,
                    op,
                )
                .await
            }
            OpType::RevokeAllIdentitySessions { identity_id } => {
                super::identity_operations::apply_revoke_all_identity_sessions(
                    self,
                    &op.tenant_id,
                    identity_id,
                    op,
                )
                .await
            }
            OpType::RotateRefreshToken {
                session_id,
                new_generation,
            } => {
                super::identity_operations::apply_rotate_refresh_token(
                    self,
                    &op.tenant_id,
                    session_id,
                    *new_generation,
                    op,
                )
                .await
            }

            // ========== Not Yet Implemented ==========
            _ => {
                tracing::debug!("Operation type not handled by applicator: {:?}", op.op_type);
                Ok(())
            }
        }
    }
}
