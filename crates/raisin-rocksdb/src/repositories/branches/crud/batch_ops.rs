//! Batch-aware branch operations for atomic transactions
//!
//! These methods carry a branch HEAD advance inside the caller's WriteBatch,
//! so it lands atomically with the changes it makes visible.

use crate::{cf, cf_handle, keys};
use raisin_context::Branch;
use raisin_error::Result;
use raisin_hlc::HLC;

use super::super::{lock_branch_record, BranchRepositoryImpl};

impl BranchRepositoryImpl {
    /// Add a branch HEAD advance to `batch` and write the batch, atomically
    /// with respect to every other writer of the branch record.
    ///
    /// The HEAD update rides in the caller's batch so the nodes and the HEAD
    /// that makes them visible land together. The branch record is read,
    /// checked and written under [`lock_branch_record`]: the monotonic guard
    /// is only meaningful if no other writer can land between the read and
    /// the write (see `branches/head.rs` for the lost-update this prevents).
    ///
    /// **Note:** This method does NOT handle replication capture. The caller
    /// should call `capture_head_update_for_replication` afterwards.
    pub async fn write_batch_with_head(
        &self,
        mut batch: rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
    ) -> Result<Branch> {
        use raisin_storage::BranchRepository;

        let _branch_lock = lock_branch_record(tenant_id, repo_id, branch_name).await;

        let mut branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        tracing::debug!(
            "write_batch_with_head: branch={}, old_head={:?}, new_head={:?}",
            branch_name,
            branch.head,
            new_head
        );
        // Monotonic advance (see update_head): a racing earlier commit must
        // never move the head back below an already-visible later revision.
        if new_head <= branch.head {
            tracing::debug!(
                "write_batch_with_head: skipping non-advancing head update branch={} current={:?} candidate={:?}",
                branch_name,
                branch.head,
                new_head
            );
        } else {
            branch.head = new_head;

            let key = keys::branch_key(tenant_id, repo_id, branch_name);
            let value = rmp_serde::to_vec(&branch)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::BRANCHES)?;
            batch.put_cf(cf, key, value);
        }

        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic write failed: {}", e)))?;

        Ok(branch)
    }

    /// Capture branch HEAD update for replication (call after batch is written)
    ///
    /// This should be called after a successful batch write that included
    /// `write_batch_with_head` to ensure replication captures the change.
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
