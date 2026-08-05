//! The write buffer shared by the full and delta sync paths.
//!
//! Both paths used to call one write per provider item. [`SyncBatcher`] instead
//! stages operations and flushes them through
//! [`NodeMaterializer::apply_batch`](super::materializer::NodeMaterializer::apply_batch),
//! so a page of items becomes a handful of transactions instead of hundreds.
//!
//! It also owns the run's [`SyncIndex`], which is what removes the per-item full
//! workspace scan. Callers therefore hold ONE batcher for a whole sync run.
//!
//! # Ordering
//!
//! Upserts and deletes share one ordered queue. A delta page that creates an item
//! and then deletes it must apply in that order; buffering only the upserts and
//! applying deletes immediately would reorder them and leave the item alive.

use super::config::{ExternalItem, MappedNode};
use super::materializer::is_item_level;
use super::materializer::{
    estimate_op_bytes, BatchOp, BatchStats, MountScope, NodeMaterializer, SyncIndex, VirtualMeta,
    VirtualNodeRef,
};
use super::{AdapterError, SyncCtx};
use chrono::Utc;
use raisin_error::Error;

/// Buffers sync operations and flushes them in bounded batches.
pub struct SyncBatcher<'a> {
    materializer: &'a dyn NodeMaterializer,
    scope: MountScope,
    index: SyncIndex,
    pending: Vec<BatchOp>,
    pending_bytes: usize,
    max_items: usize,
    max_bytes: usize,
    max_failures: usize,
    stats: BatchStats,
}

impl<'a> SyncBatcher<'a> {
    /// Open a batcher for a sync run, reading the mount's slice of the workspace
    /// once.
    pub async fn new(ctx: &'a SyncCtx<'_>) -> std::result::Result<Self, AdapterError> {
        let index = ctx
            .materializer
            .load_index(&ctx.scope)
            .await
            .map_err(|e| classify_write(&e, format!("sync index load failed: {e}")))?;
        Ok(Self {
            materializer: ctx.materializer,
            scope: ctx.scope.clone(),
            index,
            pending: Vec::new(),
            pending_bytes: 0,
            max_items: ctx.mount.sync_config.batch_size.max(1),
            max_bytes: ctx.mount.sync_config.batch_max_bytes.max(1),
            max_failures: ctx.mount.sync_config.max_item_failures,
            stats: BatchStats::default(),
        })
    }

    /// Mount-owned virtual nodes as of the last flush. Used by reconcile and TTL
    /// cleanup, which previously each cost their own full workspace scan.
    pub fn virtual_nodes(&self) -> Vec<VirtualNodeRef> {
        self.index.virtual_nodes()
    }

    /// Outcome counts accumulated across every flush so far.
    pub fn stats(&self) -> BatchStats {
        self.stats.clone()
    }

    /// Whether this item can be skipped without running the mapper at all.
    ///
    /// The etag skip-write is decided entirely by `external_id` + `etag`, both of
    /// which the provider already gave us, so mapping first only to discard the
    /// result is pure waste — and on a re-sync of an unchanged mailbox that waste
    /// IS the run: one QuickJS invocation per item, for every item, to produce
    /// nothing. The mapper is a pure item→node projection, so not running it
    /// changes no state.
    pub fn can_skip_unmapped(&self, item: &ExternalItem) -> bool {
        if self.scope.force_rewrite || item.etag.is_none() {
            return false;
        }
        self.index
            .etag_for(&item.external_id)
            .is_some_and(|etag| Some(etag) == item.etag.as_deref())
    }

    /// Count an item skipped before mapping (see [`Self::can_skip_unmapped`]).
    pub fn note_unmapped_skip(&mut self) {
        self.stats.skipped += 1;
    }

    /// Stage an upsert, flushing first if it would overflow the batch.
    pub async fn stage_upsert(
        &mut self,
        item: &ExternalItem,
        rel_path: &str,
        mapped: MappedNode,
    ) -> std::result::Result<(), AdapterError> {
        let op = BatchOp::Upsert {
            rel_path: rel_path.to_string(),
            mapped,
            virt: VirtualMeta {
                mount_id: self.scope.mount_id.clone(),
                external_id: item.external_id.clone(),
                etag: item.etag.clone(),
                synced_at: Utc::now().to_rfc3339(),
            },
        };
        self.stage(op).await
    }

    /// Stage a delete, preserving its position relative to staged upserts.
    pub async fn stage_delete(
        &mut self,
        external_id: &str,
    ) -> std::result::Result<(), AdapterError> {
        self.stage(BatchOp::Delete {
            external_id: external_id.to_string(),
        })
        .await
    }

    async fn stage(&mut self, op: BatchOp) -> std::result::Result<(), AdapterError> {
        let bytes = estimate_op_bytes(&op);
        // Flush BEFORE pushing when this op would blow the byte budget, so a
        // single oversized item still gets a batch of its own rather than being
        // appended to a batch that is already at the limit.
        if !self.pending.is_empty() && self.pending_bytes + bytes > self.max_bytes {
            self.flush().await?;
        }
        self.pending.push(op);
        self.pending_bytes += bytes;
        if self.pending.len() >= self.max_items || self.pending_bytes >= self.max_bytes {
            self.flush().await?;
        }
        Ok(())
    }

    /// Write everything staged so far.
    ///
    /// Callers MUST flush before persisting a resume cursor or returning: the
    /// cursor claims those items were imported, and an unflushed buffer is
    /// dropped silently.
    pub async fn flush(&mut self) -> std::result::Result<(), AdapterError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let ops = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        let stats = self
            .materializer
            .apply_batch(&self.scope, &mut self.index, ops)
            .await
            .map_err(|e| classify_write(&e, format!("materialize failed: {e}")))?;
        self.stats.merge(stats);
        self.check_failure_budget()
    }

    /// Give up when items are being rejected wholesale.
    ///
    /// The condition is `failed >= budget AND written == 0`, and both halves
    /// matter. A mount that writes SOME items is working — a few rejections
    /// there are individual bad items and must not stop the import. A mount that
    /// has written NOTHING after this many attempts is not having bad luck: its
    /// target workspace forbids the node type its mapper produces, or RLS denies
    /// the sync actor, and every remaining item fails the same way.
    ///
    /// Reported as `Config`, which `finalize` turns into `misconfigured` and
    /// `is_retryable` reports as non-retryable. Before this the run counted
    /// failures to the end of `max_items_per_sync`, exhausted the 600s job
    /// timeout, was retried three times, and still recorded `OK` — with the
    /// provider's own explanation visible only as a per-item WARN in a log
    /// nobody reads at `RUST_LOG=error`.
    fn check_failure_budget(&self) -> std::result::Result<(), AdapterError> {
        if self.stats.written > 0 || self.stats.failed < self.max_failures {
            return Ok(());
        }
        let reason = self
            .stats
            .first_error
            .as_deref()
            .unwrap_or("no reason recorded");
        Err(AdapterError::Config(format!(
            "{} items were rejected and none could be written; giving up. First rejection: {}",
            self.stats.failed, reason
        )))
    }
}

/// Map a storage error onto the adapter error taxonomy.
///
/// Every write failure used to become [`AdapterError::Transient`], which
/// `is_retryable` reports as retryable — so a permanently-rejected write (a
/// workspace that does not allow the mapper's node type, an RLS denial) was
/// retried by the job layer until it exhausted `max_retries`, burning the full
/// wall-clock budget over for an outcome that could not change.
///
/// Item-level errors are exactly the ones `add_node`/`put_node` raise before
/// touching the batch, and none of them is fixed by trying again.
fn classify_write(error: &Error, message: String) -> AdapterError {
    if is_item_level(error) {
        AdapterError::Config(message)
    } else {
        AdapterError::Transient(message)
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::MappedNode;
    use super::super::materializer::{estimate_op_bytes, BatchOp, VirtualMeta};

    fn op(body_len: usize) -> BatchOp {
        let mut props = serde_json::Map::new();
        props.insert(
            "body".to_string(),
            serde_json::Value::String("x".repeat(body_len)),
        );
        BatchOp::Upsert {
            rel_path: "a.txt".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: None,
                properties: props,
            },
            virt: VirtualMeta {
                mount_id: "m1".to_string(),
                external_id: "a".to_string(),
                etag: None,
                synced_at: "2026-01-01T00:00:00Z".to_string(),
            },
        }
    }

    #[test]
    fn byte_budget_is_reached_well_before_a_thousand_mail_sized_items() {
        // The reason the byte budget exists: 1000 items with 20 KB bodies would
        // be a ~20 MB replication record, over the 10 MB transport frame cap.
        let per_item = estimate_op_bytes(&op(20_000));
        assert!(per_item > 20_000);
        assert!(per_item * 1000 > 10 * 1024 * 1024);
        // ...and the default 4 MiB budget flushes long before that.
        assert!(per_item * 1000 > 4 * 1024 * 1024);
    }
}
