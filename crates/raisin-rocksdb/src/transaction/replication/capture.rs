//! Operation capture and tracked change processing.

use super::super::RocksDBTransaction;
use raisin_error::Result;
use raisin_hlc::HLC;
use std::collections::HashMap;

impl RocksDBTransaction {
    /// Capture an operation, using async queue if available or synchronous capture otherwise
    ///
    /// This method automatically selects the best approach:
    /// - If operation_queue is available AND the operation is not critical metadata: enqueue operation (non-blocking)
    /// - Otherwise: call operation_capture directly (blocking) and collect the operation
    ///
    /// IMPORTANT: UpdateBranch and CreateRevisionMeta operations are ALWAYS captured synchronously
    /// because they must be pushed immediately to make replicated data visible on peers.
    pub(in super::super) async fn capture_operation_internal(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        op_type: raisin_replication::OpType,
        actor: String,
        message: Option<String>,
        is_system: bool,
        revision: Option<HLC>,
    ) {
        // Fall back to the transaction-wide revision if caller doesn't supply one
        let resolved_revision = revision.or_else(|| {
            self.metadata
                .lock()
                .ok()
                .and_then(|meta| meta.transaction_revision)
        });

        if resolved_revision.is_none() {
            tracing::warn!(
                "capture_operation_internal called without revision (tenant={}, repo={}, branch={}, op_type={:?})",
                tenant_id,
                repo_id,
                branch,
                op_type
            );
        }

        // Check if this is a critical metadata operation that must be captured synchronously
        let is_critical_metadata = matches!(
            op_type,
            raisin_replication::OpType::UpdateBranch { .. }
                | raisin_replication::OpType::CreateRevisionMeta { .. }
        );

        if let Some(ref queue) = self.operation_queue {
            if !is_critical_metadata {
                // Use async queue for non-blocking operation capture of non-critical operations
                // Note: When using queue, operations will be pushed by the queue worker,
                // not by the transaction directly
                let queued_op = crate::replication::QueuedOperation {
                    tenant_id,
                    repo_id,
                    branch,
                    op_type,
                    actor,
                    message,
                    is_system,
                    revision: resolved_revision,
                };

                if let Err(e) = queue.try_enqueue(queued_op) {
                    tracing::warn!(
                        error = %e,
                        "Failed to enqueue operation - queue may be full (backpressure active)"
                    );
                }
                return;
            }
            // Fall through to synchronous capture for critical metadata operations
        }

        // Synchronous capture for critical operations or when queue not available
        match self
            .operation_capture
            .capture_operation_with_revision(
                tenant_id,
                repo_id,
                branch,
                op_type,
                actor,
                message,
                is_system,
                resolved_revision,
            )
            .await
        {
            Ok(op) => {
                tracing::trace!("Captured operation synchronously");
                // Collect the operation for replication push
                if let Ok(mut ops) = self.captured_operations.lock() {
                    ops.push(op);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to capture operation synchronously");
            }
        }
    }

    /// PHASE 5.4: Capture detailed operations from ChangeTracker for replication
    pub(in super::super) async fn capture_tracked_changes(
        &self,
        tenant_id: String,
        repo_id: String,
        branch_name: String,
        max_revision: Option<HLC>,
        actor: String,
        message: String,
        is_system: bool,
    ) -> Result<()> {
        tracing::info!(
            operation_capture_enabled = self.operation_capture.is_enabled(),
            "💾 TRANSACTION COMMIT: Starting operation capture phase"
        );

        if !self.operation_capture.is_enabled() {
            return Ok(());
        }

        // Extract detailed changes from ChangeTracker
        let tracked_changes = {
            let tracker = self.change_tracker.lock().map_err(|e| {
                raisin_error::Error::storage(format!("Failed to lock change_tracker: {}", e))
            })?;
            tracker.get_changes().clone()
        };

        tracing::info!(
            num_tracked_changes = tracked_changes.len(),
            "📋 TRANSACTION COMMIT: Processing tracked changes for replication"
        );

        if tracked_changes.is_empty() {
            return Ok(());
        }

        // Capture ApplyRevision operation
        if let Some(branch_revision) = max_revision {
            if let Err(e) = self
                .capture_apply_revision_operation(
                    tenant_id.clone(),
                    repo_id.clone(),
                    branch_name.clone(),
                    branch_revision,
                    &tracked_changes,
                    actor.clone(),
                    message.clone(),
                    is_system,
                    branch_revision,
                )
                .await
            {
                tracing::warn!(
                    error = %e,
                    "Failed to capture ApplyRevision operation"
                );
            }
        }

        tracing::debug!(
            "ApplyRevision operation captured for {} node changes - granular operations disabled for transactions",
            tracked_changes.len()
        );

        // Capture relation operations (AddRelation/RemoveRelation)
        // Relations can be added/removed even when no nodes are modified, so we need
        // to capture them independently. Use the current clock for relation operations
        // if no node revision was created.
        let relation_revision = max_revision.unwrap_or_else(HLC::now);
        self.capture_relation_operations(
            tenant_id,
            repo_id,
            branch_name,
            &tracked_changes,
            actor,
            is_system,
            relation_revision,
        )
        .await?;

        Ok(())
    }

    /// Capture ApplyRevision operation with all node changes in the transaction
    async fn capture_apply_revision_operation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        branch_head: HLC,
        tracked_changes: &HashMap<String, crate::replication::NodeChanges>,
        actor: String,
        message: String,
        is_system: bool,
        revision: HLC,
    ) -> Result<()> {
        let node_changes = self
            .build_replicated_node_changes(&tenant_id, &repo_id, &branch, tracked_changes)
            .await?;

        if node_changes.is_empty() {
            return Ok(());
        }

        self.capture_operation_internal(
            tenant_id,
            repo_id,
            branch,
            raisin_replication::OpType::ApplyRevision {
                branch_head,
                node_changes,
            },
            actor,
            Some(message),
            is_system,
            Some(revision),
        )
        .await;

        Ok(())
    }
}
