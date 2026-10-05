//! Batch-aware branch operations for atomic transactions
//!
//! These methods carry a branch HEAD advance inside the caller's WriteBatch,
//! so it lands atomically with the changes it makes visible.

use crate::{cf, cf_handle, keys};
use raisin_context::Branch;
use raisin_error::Result;
use raisin_hlc::HLC;

use super::super::{lock_branch_record, BranchRepositoryImpl};

/// What [`BranchRepositoryImpl::write_batch_with_head_as`] did.
#[derive(Debug)]
pub(crate) enum HeadWrite {
    /// The batch was written. `meta`: the revision record stored with it —
    /// the commit carried one and its HEAD advanced (`NodeCommit::record_revision`).
    Written {
        branch: Branch,
        meta: Option<Box<raisin_storage::RevisionMeta>>,
    },
    /// A CONDITIONAL commit found a checked node written since its read and
    /// wrote nothing (`NodeCommit::only_if_unchanged`).
    Superseded,
}

impl HeadWrite {
    /// The branch, for a commit that cannot be superseded (not conditional).
    fn written(self) -> Result<Branch> {
        match self {
            Self::Written { branch, .. } => Ok(branch),
            Self::Superseded => Err(raisin_error::Error::internal(
                "an unconditional commit reported itself superseded",
            )),
        }
    }
}

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
        batch: rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
    ) -> Result<Branch> {
        self.write_batch_with_head_as(
            batch,
            tenant_id,
            repo_id,
            branch_name,
            new_head,
            false,
            None,
        )
        .await?
        .written()
    }

    /// [`Self::write_batch_with_head`] for a batch that writes nodes: their
    /// commit step (`indexing::NodeCommit`) — locked, re-validated, written.
    pub(crate) async fn write_nodes_with_head(
        &self,
        batch: rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
        commit: &crate::indexing::NodeCommit,
    ) -> Result<Branch> {
        self.write_batch_with_head_as(
            batch,
            tenant_id,
            repo_id,
            branch_name,
            new_head,
            false,
            Some(commit),
        )
        .await?
        .written()
    }

    /// [`Self::write_batch_with_head`], for a batch that rewrites a node IN
    /// PLACE when `in_place` is set: the write then holds the in-place guard
    /// (`repositories/nodes/crud/indexing/in_place_guard.rs`), so the
    /// `node_path` backfill cannot land a stale entry on the same revision.
    ///
    /// A `commit` that carries a revision record stores it in the batch when
    /// the HEAD advances, its `parent` the HEAD replaced (read under the
    /// branch record lock). A CONDITIONAL commit whose nodes were written
    /// since their read writes nothing and returns [`HeadWrite::Superseded`].
    pub(crate) async fn write_batch_with_head_as(
        &self,
        mut batch: rocksdb::WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
        in_place: bool,
        commit: Option<&crate::indexing::NodeCommit>,
    ) -> Result<HeadWrite> {
        use raisin_storage::BranchRepository;

        // The commit step of the nodes this batch writes (plan Phase 7b):
        // locked BEFORE the branch record (the one lock order), their staged
        // index writes re-validated against what is stored now, then held
        // through the write so no other writer of them lands in between.
        let node_guard = match commit {
            Some(commit) => {
                let guard = commit.lock(&self.db).await;
                if commit.superseded(&self.db)? {
                    return Ok(HeadWrite::Superseded);
                }
                commit.revalidate(&self.db, &mut batch)?;
                guard.before_write().await;
                Some(guard)
            }
            None => None,
        };

        let branch_lock = lock_branch_record(tenant_id, repo_id, branch_name).await;
        // Localized name uniqueness against what is stored NOW, under the
        // lock every commit of the branch checks under
        // (`localized_name::unique::deferred`).
        if let Some(commit) = commit {
            commit.check_names(&self.db)?;
        }

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
        let mut meta = None;
        if new_head <= branch.head {
            tracing::debug!(
                "write_batch_with_head: skipping non-advancing head update branch={} current={:?} candidate={:?}",
                branch_name,
                branch.head,
                new_head
            );
        } else {
            // The revision record of the new HEAD, its parent the HEAD it
            // replaces — in the same batch, so no commit can land between.
            if let Some(record) = commit.and_then(|c| c.revision_record()) {
                let mut record = record.clone();
                record.revision = new_head;
                record.parent = Some(branch.head);
                let value = rmp_serde::to_vec(&record).map_err(|e| {
                    raisin_error::Error::storage(format!("RevisionMeta serialization error: {e}"))
                })?;
                batch.put_cf(
                    cf_handle(&self.db, cf::REVISIONS)?,
                    keys::revision_meta_key(tenant_id, repo_id, &new_head),
                    value,
                );
                meta = Some(Box::new(record));
            }
            branch.head = new_head;

            let key = keys::branch_key(tenant_id, repo_id, branch_name);
            let value = rmp_serde::to_vec(&branch)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::BRANCHES)?;
            batch.put_cf(cf, key, value);
        }

        // On the blocking pool (a RocksDB write can park on write stalls).
        // The in-place guard is a std lock: taken inside the blocking task,
        // never held across an `.await`. The node and branch record guards
        // move INTO the task and drop only after the write returns: a caller
        // dropping this future mid-write (a client disconnect, a timeout)
        // must not release them with the batch still in flight.
        let db = self.db.clone();
        let scope = (
            tenant_id.to_string(),
            repo_id.to_string(),
            branch_name.to_string(),
        );
        tokio::task::spawn_blocking(move || {
            let _in_place = in_place.then(|| {
                crate::repositories::nodes::in_place_write_guard(&scope.0, &scope.1, &scope.2)
            });
            if let Some(guard) = &node_guard {
                guard.in_write();
            }
            let written = db.write(batch);
            drop(branch_lock);
            drop(node_guard);
            written
        })
        .await
        .map_err(|e| raisin_error::Error::storage(format!("Write task failed to join: {}", e)))?
        .map_err(|e| raisin_error::Error::storage(format!("Atomic write failed: {}", e)))?;

        Ok(HeadWrite::Written { branch, meta })
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
