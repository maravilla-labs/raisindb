//! The transaction's half of the node commit step (plan Phase 7b): lock every
//! node the transaction writes (`indexing::node_lock`), then re-validate each
//! staged property-index write against what is stored NOW
//! (`indexing::StagedDeltaCheck`) — before the branch record lock, which is
//! the one lock order.

use super::super::RocksDBTransaction;
use raisin_error::Result;
use rocksdb::WriteBatch;

impl RocksDBTransaction {
    /// The node's stored versions, recorded BEFORE its first write in this
    /// transaction reads anything it derives from (`StagedDeltaCheck`);
    /// `None` when an earlier write of the node in this transaction already
    /// recorded them (its check covers the whole transaction).
    pub(crate) fn pending_delta_check(
        &self,
        ctx: &crate::indexing::IndexCtx<'_>,
        node_id: &str,
    ) -> Result<Option<crate::indexing::PendingDeltaCheck>> {
        let recorded = self
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?
            .delta_checks
            .contains_key(&(ctx.workspace.to_string(), node_id.to_string()));
        if recorded {
            return Ok(None);
        }
        crate::indexing::StagedDeltaCheck::before_read(&self.db, ctx, node_id).map(Some)
    }

    /// Keep `check` for the commit's re-validation, unless the node already
    /// has one.
    pub(crate) fn record_delta_check(
        &self,
        workspace: &str,
        check: crate::indexing::StagedDeltaCheck,
    ) -> Result<()> {
        self.read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?
            .delta_checks
            .entry((workspace.to_string(), check.node_id().to_string()))
            .or_insert(check);
        Ok(())
    }

    /// Lock every node this transaction wrote, moved or deleted on `branch`,
    /// in sorted order.
    pub(super) async fn lock_written_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<crate::indexing::NodeWriteGuard> {
        let ids: Vec<String> = {
            let cache = self
                .read_cache
                .lock()
                .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
            cache
                .nodes
                .keys()
                .chain(cache.moved_nodes.keys())
                .chain(cache.delta_checks.keys())
                .map(|(_, id)| id.clone())
                .collect()
        };
        Ok(crate::indexing::lock_nodes(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            ids.iter().map(String::as_str),
        )
        .await)
    }

    /// With [`Self::lock_written_nodes`] held: for every node whose staged
    /// write recorded a check, compare its stored versions with what the write
    /// was staged against (or re-derive unconditionally, for an `always`
    /// check), and append a corrective write of the node's FINAL state in this
    /// transaction when they changed — the version it stores, the moved
    /// record, or its deletion.
    pub(super) fn revalidate_staged_index_writes(&self, batch: &mut WriteBatch) -> Result<usize> {
        let cache = self
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let final_of = |key: &(String, String)| match cache.nodes.get(key) {
            Some(state) => Some(state.as_ref()),
            // Recorded, then nothing written (an aborted step): `None`.
            None => cache.moved_nodes.get(key).map(Some),
        };
        // Every node's UNIQUE claims first, so no correction (appended after
        // the whole batch) ends a value another node of this transaction
        // took at the same revision (plan Phase 13a; `NodeCommit::revalidate`).
        let mut held = crate::repositories::nodes::CommitClaims::default();
        for (key, check) in &cache.delta_checks {
            if let Some(final_version) = final_of(key) {
                check.hold_claims(&self.db, final_version, &mut held);
            }
        }
        let mut corrected = 0;
        for (key, check) in &cache.delta_checks {
            let Some(final_version) = final_of(key) else {
                continue;
            };
            if check.revalidate_final_held(&self.db, batch, final_version, Some(&held))? {
                corrected += 1;
            }
        }
        Ok(corrected)
    }
}
