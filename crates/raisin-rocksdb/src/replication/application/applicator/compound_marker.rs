//! Fail-closed compound index state after a replicated node write.
//!
//! The replication apply path writes no COMPOUND_INDEX entries (that is
//! Phase 8 step 3), so every replicated write that changes a node leaves this
//! node's compound keyspace behind its records. Each such write marks the
//! workspace's compound indexes `NotBuilt`, and the planner then scans instead
//! of serving stale rows — replica compound queries are scan-only until a local
//! rebuild, by design.
//!
//! The mark is written IN THE SAME BATCH as the node write, under the compound
//! transition lock (`CompoundStateStore::write_marking_stale`). Marking after
//! the commit left a crash window — node durable, mark lost, `Ready` served
//! over entries that no longer match — that nothing would ever repair. See
//! `crate::compound_state::marker` for the generation that keeps a concurrent
//! rebuild from re-stamping `Ready` over the mark.

use raisin_error::Result;
use rocksdb::WriteBatch;

use super::OperationApplicator;

impl OperationApplicator {
    /// Commit `batch` (a replicated node write to `workspace`) together with
    /// the stale mark for the workspace's compound indexes, atomically.
    ///
    /// A failure here fails the apply and nothing was written, so the op is
    /// redelivered — the only answer that cannot leave `Ready` over stale
    /// entries.
    pub(in crate::replication::application) fn write_marking_compound_stale(
        &self,
        batch: WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<()> {
        let store = crate::compound_state::CompoundStateStore::new(self.db.clone());
        let marked = store.write_marking_stale(batch, tenant_id, repo_id, branch, workspace)?;
        if marked > 0 {
            tracing::debug!(
                tenant_id,
                repo_id,
                branch,
                workspace,
                marked,
                "replicated write: compound indexes marked NotBuilt until a local rebuild"
            );
        }
        Ok(())
    }
}
