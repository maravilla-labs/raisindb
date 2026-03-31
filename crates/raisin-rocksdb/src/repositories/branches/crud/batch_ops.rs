//! Batch-aware branch operations for atomic transactions
//!
//! These methods write to WriteBatch instead of directly to the DB,
//! enabling inclusion in larger atomic transactions.

use crate::{cf, cf_handle, keys};
use raisin_context::Branch;
use raisin_error::Result;
use raisin_hlc::HLC;

use super::super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Update branch HEAD within a WriteBatch (for atomic operations)
    ///
    /// This method writes to the provided batch instead of directly to the DB,
    /// allowing the caller to include it in a larger atomic transaction.
    ///
    /// **Note:** This method does NOT handle replication capture. The caller
    /// should call `capture_head_update_for_replication` after the batch is
    /// written successfully.
    pub async fn update_head_to_batch(
        &self,
        batch: &mut rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
    ) -> Result<Branch> {
        use raisin_storage::BranchRepository;

        let mut branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        tracing::debug!(
            "update_head_to_batch: branch={}, old_head={:?}, new_head={:?}",
            branch_name,
            branch.head,
            new_head
        );
        branch.head = new_head;

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let value = rmp_serde::to_vec(&branch)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        batch.put_cf(cf, key, value);

        Ok(branch)
    }

    /// Capture branch HEAD update for replication (call after batch is written)
    ///
    /// This should be called after a successful batch write that included
    /// `update_head_to_batch` to ensure replication captures the change.
    pub async fn capture_head_update_for_replication(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        branch: &Branch,
        new_head: HLC,
    ) {
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_operation_with_revision(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch_name.to_string(),
                        raisin_replication::OpType::UpdateBranch {
                            branch: branch.clone(),
                        },
                        "system".to_string(),
                        Some(format!("Branch '{}' head updated", branch_name)),
                        true,
                        Some(new_head),
                    )
                    .await;
            }
        }
    }
}
