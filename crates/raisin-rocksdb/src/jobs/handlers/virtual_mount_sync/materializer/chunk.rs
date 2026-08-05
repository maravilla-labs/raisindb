//! One transaction, many items: the batch write and its single-item replay.

use raisin_error::Result;
use std::collections::{HashMap, HashSet};

use super::index::{MountScope, SyncIndex};
use super::node_paths::is_item_level;
use super::ops::{BatchOp, BatchStats};
use super::stage::{IndexMutation, Staged};
use super::RocksDbMaterializer;

impl RocksDbMaterializer {
    /// Apply `ops` in one transaction. `allow_replay` guards the single-item
    /// retry so a failing replay cannot recurse.
    ///
    /// Returns the outcome counts plus any operations held back by the in-chunk
    /// unique guard, which the caller must write individually.
    pub(super) async fn apply_chunk(
        &self,
        scope: &MountScope,
        index: &mut SyncIndex,
        ops: &[BatchOp],
        allow_replay: bool,
    ) -> Result<(BatchStats, Vec<BatchOp>)> {
        let tx = self.begin(scope, "virtual mount sync: upsert").await?;
        let mut stats = BatchStats::default();
        // Index updates are applied only after the commit succeeds: a rolled-back
        // transaction wrote nothing, and a replay must resolve against exactly the
        // state the aborted attempt started from.
        let mut pending: Vec<IndexMutation> = Vec::new();
        // In-chunk unique-constraint guard. `check_unique_constraints` reads the
        // COMMITTED unique index and is not read-cache aware, so two items in one
        // transaction sharing a `unique: true` value would both pass and both be
        // indexed. Items that collide inside the chunk are pushed to the
        // single-item replay, where the real check runs against committed state.
        let mut unique_seen: HashSet<(String, String, String)> = HashSet::new();
        let mut unique_props: HashMap<String, Vec<String>> = HashMap::new();
        let mut deferred: Vec<BatchOp> = Vec::new();

        for op in ops {
            let staged = self
                .stage_op(
                    tx.as_ref(),
                    scope,
                    index,
                    op,
                    &mut pending,
                    &mut unique_seen,
                    &mut unique_props,
                    allow_replay,
                )
                .await;
            match staged {
                Ok(Staged::Written) => stats.written += 1,
                Ok(Staged::Deleted) => stats.deleted += 1,
                Ok(Staged::Skipped) => stats.skipped += 1,
                Ok(Staged::Deferred) => deferred.push(op.clone()),
                // Item-level rejection. Every such check in `add_node` /
                // `put_node` runs BEFORE the first batch write, so the item
                // contributed nothing and the transaction stays usable — skip it
                // and keep going rather than losing the whole batch to one bad
                // item.
                Err(e) if is_item_level(&e) => {
                    tracing::warn!(
                        mount_id = %scope.mount_id,
                        external_id = %op.external_id(),
                        error = %e,
                        "virtual mount item rejected; continuing with the rest of the batch"
                    );
                    stats.failed += 1;
                }
                // Infrastructural: the transaction itself is suspect. Abandon it.
                Err(e) => {
                    let _ = tx.rollback().await;
                    if allow_replay {
                        tracing::warn!(
                            mount_id = %scope.mount_id,
                            items = ops.len(),
                            error = %e,
                            "virtual mount batch aborted; retrying item by item"
                        );
                        return Ok((self.replay(scope, index, ops).await?, Vec::new()));
                    }
                    return Err(e);
                }
            }
        }

        match tx.commit().await {
            Ok(()) => {
                for mutation in pending {
                    mutation.apply(index);
                }
                Ok((stats, deferred))
            }
            // A commit failure names no culprit — `commit_impl` does no per-node
            // validation — so the only way to isolate it is to replay the chunk
            // one item at a time. Nothing was written, so this cannot double-write.
            Err(e) if allow_replay => {
                tracing::warn!(
                    mount_id = %scope.mount_id,
                    items = ops.len(),
                    error = %e,
                    "virtual mount batch commit failed; retrying item by item"
                );
                Ok((self.replay(scope, index, ops).await?, Vec::new()))
            }
            Err(e) => Err(e),
        }
    }

    /// Re-run a failed chunk one item per transaction. A poison item is counted
    /// and logged, never propagated — one bad item must not stall a mount
    /// forever.
    // Boxed because `apply_chunk` calls this and this calls `apply_chunk` back
    // (with `allow_replay: false`, which is what bounds the recursion at one
    // level).
    pub(super) fn replay<'s>(
        &'s self,
        scope: &'s MountScope,
        index: &'s mut SyncIndex,
        ops: &'s [BatchOp],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<BatchStats>> + Send + 's>> {
        Box::pin(async move {
            let mut stats = BatchStats::default();
            for op in ops {
                match self
                    .apply_chunk(scope, index, std::slice::from_ref(op), false)
                    .await
                {
                    Ok((one, _)) => stats.merge(one),
                    Err(e) => {
                        tracing::warn!(
                            mount_id = %scope.mount_id,
                            external_id = %op.external_id(),
                            error = %e,
                            "virtual mount item failed on replay; skipping it"
                        );
                        stats.failed += 1;
                    }
                }
            }
            Ok(stats)
        })
    }
}
