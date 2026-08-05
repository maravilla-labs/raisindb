//! Transactional materialization of external items into nodes.
//!
//! Every write goes through the normal transactional write path under actor
//! [`SYNC_ACTOR`] with a system auth context, so `node_event` triggers,
//! fulltext, SQL indexes, audit, and replication all apply for free.
//!
//! # Why this is batched
//!
//! The engine used to materialize ONE item per transaction, and to locate that
//! item it re-listed the ENTIRE target workspace (`scan_nodes`) and scanned the
//! result linearly for `__external_id`. That is O(items × workspace): a 500-item
//! page against a 50k-node workspace materialized ~25M nodes just to find 500.
//! Importing a real mailbox was not merely slow, it got quadratically slower as
//! it went.
//!
//! Two changes fix it, and they are independent:
//!
//! 1. [`SyncIndex`] — the workspace under the mount path is read ONCE per sync
//!    run into two maps, and every lookup the upsert path needs is served from
//!    memory. The index is kept current as writes land, so it stays authoritative
//!    for the whole run.
//! 2. [`NodeMaterializer::apply_batch`] — N items share ONE transaction and ONE
//!    commit, which means one revision, one branch-HEAD bump, one RocksDB write,
//!    one snapshot job and one replication oplog record instead of N of each.
//!
//! Batches are bounded by BOTH an item count and a byte budget (see
//! `SyncConfig::batch_size` / `batch_max_bytes`), because the commit's
//! replication capture persists a single un-decomposed `ApplyRevision` holding
//! full snapshots of every node in the batch. No catch-up/replay path decomposes
//! a stored operation, so an oversized record would exceed the 10 MB transport
//! frame cap and permanently wedge a peer's sync. The byte budget is what makes
//! a large item count safe; do not remove it.

mod chunk;
mod index;
mod node_paths;
mod ops;
mod remap;
mod stage;

#[cfg(test)]
mod tests;

pub use index::{MountScope, PathEntry, SyncIndex, VirtualMeta, VirtualNodeRef};
pub use node_paths::is_item_level;
pub use ops::{dedup_ops, estimate_op_bytes, BatchOp, BatchStats};

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use std::sync::Arc;

use super::config::SYNC_ACTOR;
use crate::RocksDBStorage;

/// Materializes mapped external items into nodes. Deletes are scoped to the
/// mount — user-created nodes under the mount path are never touched.
#[async_trait]
pub trait NodeMaterializer: Send + Sync {
    /// Read the mount's slice of the target workspace once, for the whole run.
    async fn load_index(&self, scope: &MountScope) -> Result<SyncIndex>;

    /// Apply a batch of operations in ONE transaction and ONE commit, updating
    /// `index` to match what landed.
    ///
    /// Never fails for a single bad item: item-level rejections are counted in
    /// [`BatchStats::failed`] and logged. An error here means the whole batch,
    /// and its retry, could not be written.
    async fn apply_batch(
        &self,
        scope: &MountScope,
        index: &mut SyncIndex,
        ops: Vec<BatchOp>,
    ) -> Result<BatchStats>;
}

/// RocksDB-backed materializer.
pub struct RocksDbMaterializer {
    storage: Arc<RocksDBStorage>,
}

impl RocksDbMaterializer {
    /// Create a materializer bound to storage.
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }

    /// Open a transaction scoped to the mount with the sync actor + system auth.
    pub(super) async fn begin(
        &self,
        scope: &MountScope,
        message: &str,
    ) -> Result<Box<dyn TransactionalContext>> {
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&scope.tenant, &scope.repo)?;
        tx.set_branch(&scope.branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message(message)?;
        Ok(tx)
    }
}

#[async_trait]
impl NodeMaterializer for RocksDbMaterializer {
    async fn load_index(&self, scope: &MountScope) -> Result<SyncIndex> {
        let tx = self.begin(scope, "virtual mount sync: index").await?;
        let all = tx.scan_nodes(&scope.workspace).await?;
        Ok(SyncIndex::from_nodes(
            all,
            &scope.mount_id,
            &scope.mount_path,
        ))
    }

    async fn apply_batch(
        &self,
        scope: &MountScope,
        index: &mut SyncIndex,
        ops: Vec<BatchOp>,
    ) -> Result<BatchStats> {
        let ops = dedup_ops(ops, &scope.mount_path);
        if ops.is_empty() {
            return Ok(BatchStats::default());
        }
        if scope.force_rewrite {
            self.remap_moves(scope, index, &ops).await?;
        }
        let (mut stats, deferred) = self.apply_chunk(scope, index, &ops, true).await?;

        // Items held back by the in-chunk unique guard are written individually,
        // where the real constraint check runs against committed state and the
        // loser is rejected exactly as it would have been before batching.
        if !deferred.is_empty() {
            stats.merge(self.replay(scope, index, &deferred).await?);
        }
        Ok(stats)
    }
}
