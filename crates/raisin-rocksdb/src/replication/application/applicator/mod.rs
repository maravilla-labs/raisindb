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

mod compound_marker;
mod crdt_ops;
mod db_lookups;
mod delete_ops;
mod newer_version;
mod node_baseline;

pub use node_baseline::{unknown_workspace_scans, WorkspaceHint};
mod registry_ops;
mod relation_ops;
mod schema_ops;
mod user_ops;
mod workspace_branch_ops;
mod workspace_lww;
mod workspace_ops;

use crate::repositories::BranchRepositoryImpl;
use raisin_error::Result;
use raisin_events::EventBus;
use raisin_hlc::HLC;
use raisin_replication::{OpType, Operation};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

const TOMBSTONE: &[u8] = b"T";

/// The ONE tombstone predicate (`b"T"`, and the one-byte `b"\x00"` merge used
/// to write) — see `crate::keys::is_tombstone_value`.
fn is_tombstone(value: &[u8]) -> bool {
    crate::keys::is_tombstone_value(value)
}

/// The nodes an op writes on its branch — what [`OperationApplicator::
/// apply_operation`] locks (`indexing::node_lock`). `None` for ops that write
/// no node record or node index.
fn node_ids_of(op_type: &OpType) -> Option<Vec<&str>> {
    Some(match op_type {
        OpType::ApplyRevision { node_changes, .. } => node_changes
            .iter()
            .map(|change| change.node.id.as_str())
            .collect(),
        OpType::UpsertNodeSnapshot { node, .. } => vec![node.id.as_str()],
        OpType::DeleteNodeSnapshot { node_id, .. } => vec![node_id.as_str()],
        _ => return None,
    })
}

/// The workspace a replicated node is applied in. Every emitter stamps
/// `node.workspace` (see `node_baseline`), so the `"default"` here only names
/// where a malformed, workspace-less node lands — explicitly, with no scan.
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

    /// Borrow the database handle.
    ///
    /// Lets an apply handler construct the same store the live write path uses,
    /// instead of re-deriving that store's key layout (see
    /// `application/oauth_operations.rs`).
    pub fn db(&self) -> &Arc<DB> {
        &self.db
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

    /// After a replicated NodeType write: re-resolve the branch's cached
    /// index definitions (compound declarations, unique names) NOW, before
    /// any later op is applied against them — the `Event::Schema` re-warm is
    /// asynchronous. A read between ops, never inside a node batch.
    pub(super) async fn refresh_index_definitions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        written: &str,
    ) {
        let node_types = crate::repositories::NodeTypeRepositoryImpl::new(
            self.db.clone(),
            Arc::new(crate::repositories::RevisionRepositoryImpl::new(
                self.db.clone(),
                "replication-index-defs".to_string(),
            )),
            self.branch_repo.clone(),
        );
        if let Err(e) = crate::indexing::compound::defs::refresh_branch(
            &self.db,
            &node_types,
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &[written],
        )
        .await
        {
            // Dropped, not stale: the next write finds them cold and fails
            // the compound index closed.
            tracing::warn!(error = %e, "could not refresh index definitions after a schema op");
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

        // The node commit step (plan Phase 7b): every node this op writes is
        // locked — the SAME per-node mutex every local write funnel takes —
        // for the whole apply, so its baseline read and its write see no
        // local commit of those nodes land in between (and a local commit's
        // re-validation sees no replicated write land in between either).
        let _node_guard = match node_ids_of(&op.op_type) {
            Some(ids) => Some(
                crate::indexing::lock_nodes(&self.db, &op.tenant_id, &op.repo_id, &op.branch, ids)
                    .await,
            ),
            None => None,
        };

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

            OpType::DeleteRepository { tenant_id, repo_id } => {
                // A peer deleted the repository: remove ours, every key of it,
                // exactly as the local delete does. Our APPLIED_OPS record of
                // this operation is what stops a replay from re-applying the
                // repository's older operations afterwards.
                let report =
                    crate::storage::repo_purge::purge_repository_keys(&self.db, tenant_id, repo_id);
                tracing::warn!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    from = %op.cluster_node_id,
                    failed = ?report.failed,
                    "Applied a replicated repository delete"
                );
                self.event_bus.publish(raisin_events::Event::Repository(
                    raisin_events::RepositoryEvent {
                        tenant_id: tenant_id.clone(),
                        repository_id: repo_id.clone(),
                        kind: raisin_events::RepositoryEventKind::Deleted,
                        workspace: None,
                        revision_id: None,
                        branch_name: None,
                        tag_name: None,
                        message: None,
                        actor: None,
                        metadata: None,
                    },
                ));
                Ok(())
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
            OpType::DeleteNodeSnapshot {
                node_id,
                revision,
                node,
                parent_id,
            } => {
                self.apply_delete_node_snapshot(
                    &op.tenant_id,
                    &op.repo_id,
                    &op.branch,
                    node_id,
                    node.as_ref(),
                    parent_id.as_deref(),
                    revision,
                    op,
                )
                .await
            }

            // ========== OAuth authorization server & API keys ==========
            OpType::UpsertOAuthClient { client_id, client } => {
                super::oauth_operations::apply_upsert_oauth_client(
                    self,
                    &op.tenant_id,
                    client_id,
                    client,
                    op,
                )
                .await
            }
            OpType::DeleteOAuthClient { client_id } => {
                super::oauth_operations::apply_delete_oauth_client(
                    self,
                    &op.tenant_id,
                    client_id,
                    op,
                )
                .await
            }
            OpType::UpsertOAuthRefreshToken { token_hash, token } => {
                super::oauth_operations::apply_upsert_oauth_refresh_token(
                    self,
                    &op.tenant_id,
                    token_hash,
                    token,
                    op,
                )
                .await
            }
            OpType::RevokeOAuthRefreshFamily { family_id } => {
                super::oauth_operations::apply_revoke_oauth_refresh_family(
                    self,
                    &op.tenant_id,
                    family_id,
                    op,
                )
                .await
            }
            // ========== Secret Store ==========
            OpType::UpsertSecret { name, secret } => {
                super::secret_operations::apply_upsert_secret(self, name, secret, op).await
            }

            OpType::UpsertApiKey { key_id, api_key } => {
                super::oauth_operations::apply_upsert_api_key(
                    self,
                    &op.tenant_id,
                    key_id,
                    api_key,
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

            // ========== Translations (plan Phase 11) ==========
            OpType::UpsertTranslationOverlay { .. } => {
                super::translation_operations::apply_translation_version(self, op).await
            }

            // ========== From a newer peer ==========
            // Skipped, never an error: an error here is retried forever and
            // wedges every later op from that peer behind it. It is marked
            // applied like any other op and NOT re-dispatched after this node
            // upgrades — see `OpType::Unknown` for the rule that follows.
            OpType::Unknown { tag, .. } => {
                tracing::warn!(
                    op_id = %op.op_id,
                    from = %op.cluster_node_id,
                    op_type = %tag,
                    "skipping an operation type this binary does not know"
                );
                Ok(())
            }

            // ========== No apply arm ==========
            // Tenant/deployment deletes and permission grants are captured
            // but not applied by replication (yet).
            _ => {
                tracing::debug!("Operation type not handled by applicator: {:?}", op.op_type);
                Ok(())
            }
        }
    }
}
