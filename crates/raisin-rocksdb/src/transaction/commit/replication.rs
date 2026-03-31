//! Replication-related commit operations

use super::super::RocksDBTransaction;
use raisin_error::Result;
use tracing::{debug, info, warn};

impl RocksDBTransaction {
    /// Capture RevisionMeta and branch update operations for replication
    pub(in crate::transaction) async fn capture_metadata_operations(
        &self,
        tenant_id: String,
        repo_id: String,
        revision_meta: Option<raisin_storage::RevisionMeta>,
        branch_updates: Vec<(String, String, raisin_context::Branch)>,
        actor: &Option<String>,
        message: &Option<String>,
        is_system: bool,
    ) {
        // PHASE 5.3: Capture RevisionMeta operation for replication
        // This must happen BEFORE UpdateBranch to maintain correct op_seq ordering
        if let Some(revision_meta) = revision_meta {
            self.capture_operation_internal(
                tenant_id.clone(),
                repo_id.clone(),
                revision_meta.branch.clone(),
                raisin_replication::OpType::CreateRevisionMeta {
                    revision_meta: revision_meta.clone(),
                },
                revision_meta.actor.clone(),
                Some(revision_meta.message.clone()),
                revision_meta.is_system,
                Some(revision_meta.revision),
            )
            .await;
        }

        // Capture branch update operations
        debug!(count = branch_updates.len(), "Capturing branch updates");
        for (tenant_id, repo_id, updated_branch) in branch_updates {
            let branch_name = updated_branch.name.clone();
            let branch_revision = updated_branch.head;
            debug!(
                branch = %branch_name,
                head = %branch_revision,
                "Capturing UpdateBranch operation"
            );
            self.capture_operation_internal(
                tenant_id,
                repo_id,
                branch_name,
                raisin_replication::OpType::UpdateBranch {
                    branch: updated_branch,
                },
                actor
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "system".to_string()),
                message.clone(),
                is_system,
                Some(branch_revision),
            )
            .await;
        }
    }

    /// PHASE 5.6: Push captured operations to replication peers
    pub(in crate::transaction) async fn push_to_replication_peers(&self) -> Result<()> {
        debug!("push_to_replication_peers called");

        if !self.operation_capture.is_enabled() {
            debug!("Operation capture is not enabled, skipping push");
            return Ok(());
        }

        debug!("Operation capture is enabled");

        // Collect captured operations to push
        let ops_to_push = {
            let mut ops = self.captured_operations.lock().map_err(|e| {
                raisin_error::Error::storage(format!("Failed to lock captured_operations: {}", e))
            })?;
            std::mem::take(&mut *ops)
        };

        debug!(count = ops_to_push.len(), "Collected operations to push");

        if ops_to_push.is_empty() {
            debug!("No operations to push, returning");
            return Ok(());
        }

        // Check if replication coordinator is available
        let coordinator_guard = self.replication_coordinator.read().await;
        if let Some(ref coordinator) = *coordinator_guard {
            debug!(
                count = ops_to_push.len(),
                "Replication coordinator available, pushing operations"
            );

            // Push to peers (async, fire-and-forget)
            match coordinator.push_to_all_peers(ops_to_push).await {
                Ok(_) => {
                    info!("Successfully triggered replication push");
                }
                Err(e) => {
                    warn!(error = %e, "Failed to push operations to peers");
                }
            }
        } else {
            warn!("No replication coordinator available");
        }

        Ok(())
    }
}
