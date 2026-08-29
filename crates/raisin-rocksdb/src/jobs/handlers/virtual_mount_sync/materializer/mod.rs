//! Transactional materialization of external items into nodes.
//!
//! Every write goes through the normal transactional write path as
//! [`SYNC_ACTOR`] — with system privileges, but with the sync actor as the
//! identity, not `"system"` — so `node_event` triggers, fulltext, SQL indexes,
//! audit, and replication all apply for free, and every synced node's
//! `created_by` / `updated_by` names the engine that wrote it.
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
mod command;
mod index;
mod node_paths;
mod ops;
mod remap;
mod stage;
mod stamp;
mod write_view;

#[cfg(test)]
mod tests;

pub use command::{
    command_seq, command_status, CasOutcome, CommandCas, STATUS_PROP, WRITE_SEQ_PROP,
};
pub use index::{MountScope, PathEntry, SyncIndex, VirtualMeta, VirtualNodeRef};
pub use node_paths::is_item_level;
// Re-exported so the reconcile walk reads the reserved provenance props through
// the SAME accessors the materializer writes them with. A second reader that
// disagreed about the property name or its type is how a delete gets pushed for
// a node this mount never owned.
// `under` travels with them for the same reason: the mount-boundary test that
// decides a node has been moved OUT of a mount must be the identical predicate
// the index applies when it decides what is IN one. Two spellings of "under the
// mount path" is how a node ends up detached by one half and still indexed by
// the other.
pub(crate) use node_paths::{
    ancestor_paths, node_external_id, node_mount_id, node_str_prop, under,
};
pub use ops::{dedup_ops, estimate_node_bytes, estimate_op_bytes, BatchOp, BatchStats};
pub use write_view::{
    content_identity, pushed_state_of, write_view_of, WriteView, PUSHED_CONTENT_KEY,
    PUSHED_STATE_PROP,
};

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

    /// Read one node as it stands NOW.
    ///
    /// The write drain needs this even though it already holds an index: the
    /// index is a snapshot taken at the top of the run, and a push must be
    /// decided against the node's current state or a drain that took a minute
    /// would send an edit the user has since reverted.
    async fn read_node(
        &self,
        scope: &MountScope,
        node_id: &str,
    ) -> Result<Option<raisin_models::nodes::Node>>;

    /// Every node at or under the mount path, as it stands NOW.
    ///
    /// The write drain's `submit` mode needs this and [`Self::load_index`]
    /// cannot serve it: a command node is authored by a USER, so it carries
    /// neither `__mount_id` nor `__external_id` until the engine stamps one on
    /// it — and `SyncIndex::by_external`, which is what `virtual_nodes` reads,
    /// deliberately holds only mount-owned nodes that already have both. An
    /// outbox drained through the index would find an empty outbox forever.
    async fn scan_mount_nodes(&self, scope: &MountScope)
        -> Result<Vec<raisin_models::nodes::Node>>;

    /// Compare-and-swap one `submit` command node's status, in its own
    /// immediately-committed transaction.
    ///
    /// Deliberately NOT routed through [`Self::apply_batch`]. The batcher exists
    /// to amortize writes by deferring them, and a deferred `queued -> sending`
    /// is exactly the bug the at-most-once protocol is built to prevent: the
    /// claim must be on disk before the provider call, or a crash cannot be told
    /// apart from a send that never happened. See [`command`].
    async fn cas_command(&self, scope: &MountScope, cas: &CommandCas) -> Result<CasOutcome>;

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

    /// Open a transaction scoped to the mount, acting as [`SYNC_ACTOR`] with
    /// system privileges.
    ///
    /// Both the raw actor and the auth context carry [`SYNC_ACTOR`] on purpose.
    /// The raw actor is what lands on `RevisionMeta.actor`; the auth context is
    /// what `put_node` / `add_node` stamp into `created_by` / `updated_by`, and
    /// the auth context WINS there. A plain `AuthContext::system()` therefore
    /// stamped every synced node `"system"` — provenance the console and the
    /// audit log both showed, and which made this module's own promise false.
    pub(super) async fn begin(
        &self,
        scope: &MountScope,
        message: &str,
    ) -> Result<Box<dyn TransactionalContext>> {
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&scope.tenant, &scope.repo)?;
        tx.set_branch(&scope.branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
        tx.set_message(message)?;
        Ok(tx)
    }

    /// Every node at or under the mount path — and, as nearly as a key prefix
    /// allows, nothing else.
    ///
    /// # Why this is a path scan and not `scan_nodes`
    ///
    /// This used to be `tx.scan_nodes(workspace)` with the mount filter applied
    /// in memory by [`SyncIndex::from_nodes`]. That reads the NODES CF prefix
    /// for the whole workspace, which materialises the value of EVERY revision
    /// of EVERY node (the tombstone check needs the value), deserialises the
    /// newest per id, then re-seeks NODE_PATH once per node to materialise its
    /// path, then walks ORDERED_CHILDREN once per node — including leaves — to
    /// put the result in tree order, matching siblings with an O(children²)
    /// `find` and cloning each node twice. All of it, once per sync run, and
    /// then all but the mount's own slice is thrown away by `under()`.
    ///
    /// A mount owning 5k of a 200k-node workspace paid for the other 195k on
    /// every tick, which also couples unrelated mounts: growing one mailbox
    /// slows every other mount in the workspace.
    ///
    /// `scan_by_path_prefix` reads the PATH_INDEX prefix instead — tiny keys
    /// whose value is a node id — keeps the newest live revision per node, and
    /// fetches exactly those with ONE `MultiGet`. No tree ordering (the index is
    /// two hash maps; order was never used), no per-node NODE_PATH seek beyond
    /// the ones the deserializer already does, no whole-workspace Vec<Node>.
    ///
    /// # Why the result is still filtered afterwards
    ///
    /// A key prefix is a byte prefix: scanning `/drive` also returns
    /// `/driveways/x`. That is bounded and harmless because
    /// [`SyncIndex::from_nodes`] re-applies `under()` to everything it is
    /// handed, exactly as it did to the whole-workspace listing — so the set
    /// that reaches the index is unchanged, including the deliberate inclusion
    /// of FOREIGN nodes under the mount path, which the never-clobber-user-
    /// content guard depends on. A narrower scan (by `__mount_id`, say) would
    /// silently drop that guard.
    ///
    /// `mount_path == "/"` needs no special case: the prefix is `/`, which
    /// matches every path in the workspace, and `under("/", _)` is always true.
    pub(super) async fn load_index_nodes(
        &self,
        scope: &MountScope,
    ) -> Result<Vec<raisin_models::nodes::Node>> {
        use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

        let prefix = if scope.mount_path.is_empty() {
            "/"
        } else {
            scope.mount_path.as_str()
        };
        self.storage
            .nodes()
            .scan_by_path_prefix(
                StorageScope::new(&scope.tenant, &scope.repo, &scope.branch, &scope.workspace),
                prefix,
                ListOptions::default(),
            )
            .await
    }
}

#[async_trait]
impl NodeMaterializer for RocksDbMaterializer {
    async fn load_index(&self, scope: &MountScope) -> Result<SyncIndex> {
        let under_mount = self.load_index_nodes(scope).await?;
        Ok(SyncIndex::from_nodes(
            under_mount,
            &scope.mount_id,
            &scope.mount_path,
            &scope.watched_fields,
            &scope.command_node_types,
        ))
    }

    async fn read_node(
        &self,
        scope: &MountScope,
        node_id: &str,
    ) -> Result<Option<raisin_models::nodes::Node>> {
        let tx = self.begin(scope, "virtual mount sync: read node").await?;
        tx.get_node(&scope.workspace, node_id).await
    }

    async fn scan_mount_nodes(
        &self,
        scope: &MountScope,
    ) -> Result<Vec<raisin_models::nodes::Node>> {
        // The SAME `under()` predicate the index applies, for the same reason:
        // `scan_by_path_prefix` is a BYTE prefix scan, so a `/mail/outbox` mount
        // also returns `/mail/outbox-archive/...`. Two spellings of "under the
        // mount path" is how a command gets submitted by a mount that does not
        // own it.
        let nodes = self.load_index_nodes(scope).await?;
        Ok(nodes
            .into_iter()
            .filter(|n| node_paths::under(&scope.mount_path, &n.path))
            .collect())
    }

    async fn cas_command(&self, scope: &MountScope, cas: &CommandCas) -> Result<CasOutcome> {
        self.cas_command_impl(scope, cas).await
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
