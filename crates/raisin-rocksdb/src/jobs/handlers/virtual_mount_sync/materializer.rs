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

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::config::{build_properties, MappedNode, SYNC_ACTOR};
use crate::RocksDBStorage;

/// Identifies one mount within a repo/branch/workspace.
#[derive(Debug, Clone)]
pub struct MountScope {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
    /// Target workspace the mount materializes into.
    pub workspace: String,
    pub mount_id: String,
    /// Path prefix inside the target workspace (e.g. `/documents/shared`).
    pub mount_path: String,
    /// Re-materialize every item even when its etag is unchanged, and move it if
    /// the mapper/template now resolves a different path ("remap").
    ///
    /// Ordinary syncs must NOT do this: the etag skip-write is what stops a
    /// re-sync creating a revision per unchanged item and re-firing every
    /// downstream trigger. But that same skip returns before the mapper's output
    /// is applied, so a changed mapper — a new node type, a renamed property, a
    /// new folder hierarchy — is invisible to everything already synced. Remap
    /// is the deliberate, operator-triggered exception.
    pub force_rewrite: bool,
}

/// Reserved virtual metadata stamped on every synced node.
#[derive(Debug, Clone)]
pub struct VirtualMeta {
    pub mount_id: String,
    pub external_id: String,
    pub etag: Option<String>,
    /// ISO 8601 timestamp of this sync write.
    pub synced_at: String,
}

/// Lightweight reference to an existing mount-owned node.
#[derive(Debug, Clone)]
pub struct VirtualNodeRef {
    pub id: String,
    pub path: String,
    pub external_id: String,
    pub etag: Option<String>,
    /// `__synced_at` as unix epoch seconds (used by ephemeral TTL cleanup).
    /// Tolerates both `String` (ISO 8601) and `Date` stored representations.
    pub synced_secs: Option<i64>,
}

/// One node occupying a path under the mount.
#[derive(Debug, Clone)]
pub struct PathEntry {
    /// `None` when we know the path is occupied but never read the node's id —
    /// an ancestor folder auto-created by `upsert_deep_node` during this run.
    pub id: Option<String>,
    /// Whether the node carries THIS mount's `__mount_id`. Nodes that do not are
    /// user content and must never be clobbered.
    pub mount_owned: bool,
    pub etag: Option<String>,
}

/// Everything the upsert path needs to locate an item, read ONCE per sync run.
///
/// Two views, because the single filtered list the engine used to derive is not
/// enough:
///
/// * `by_external` — mount-owned nodes that carry an `__external_id`. Replaces
///   the linear scan that matched an item to its node, and is what makes a
///   provider-side rename update the existing node instead of duplicating it.
/// * `by_path` — EVERY node under the mount path, mount-owned or not. The
///   foreign-node guard (never overwrite user content sitting at a target path)
///   and the path-fallback etag check both operate on nodes that `by_external`
///   deliberately excludes, so filtering this map by `__mount_id` would silently
///   drop both protections.
#[derive(Debug, Clone, Default)]
pub struct SyncIndex {
    by_external: HashMap<String, VirtualNodeRef>,
    by_path: HashMap<String, PathEntry>,
}

impl SyncIndex {
    /// Build both views from a listing of the target workspace.
    pub fn from_nodes(nodes: Vec<Node>, mount_id: &str, mount_path: &str) -> Self {
        let mut idx = Self::default();
        for node in nodes {
            if !under(mount_path, &node.path) {
                continue;
            }
            let mount_owned = node_mount_id(&node) == Some(mount_id);
            let etag = node_str_prop(&node, "__etag");
            idx.by_path.insert(
                node.path.clone(),
                PathEntry {
                    id: Some(node.id.clone()),
                    mount_owned,
                    etag: etag.clone(),
                },
            );
            if !mount_owned {
                continue;
            }
            if let Some(ext) = node_external_id(&node) {
                idx.by_external.insert(
                    ext.to_string(),
                    VirtualNodeRef {
                        id: node.id.clone(),
                        path: node.path.clone(),
                        external_id: ext.to_string(),
                        etag,
                        synced_secs: node_synced_secs(&node),
                    },
                );
            }
        }
        idx
    }

    /// Every mount-owned virtual node, for reconcile and TTL cleanup.
    pub fn virtual_nodes(&self) -> Vec<VirtualNodeRef> {
        self.by_external.values().cloned().collect()
    }

    /// Number of mount-owned virtual nodes.
    pub fn virtual_len(&self) -> usize {
        self.by_external.len()
    }

    fn by_external(&self, external_id: &str) -> Option<&VirtualNodeRef> {
        self.by_external.get(external_id)
    }

    /// The stored etag of an already-synced item, for the pre-mapping skip check.
    pub fn etag_for(&self, external_id: &str) -> Option<&str> {
        self.by_external.get(external_id)?.etag.as_deref()
    }

    fn at_path(&self, path: &str) -> Option<&PathEntry> {
        self.by_path.get(path)
    }

    /// Record a node this run just wrote, plus the ancestor folders
    /// `upsert_deep_node` guarantees now exist.
    ///
    /// Both must be recorded or a later item in the SAME run re-derives a stale
    /// answer: without the node, a duplicate `external_id` creates a second node;
    /// without the ancestors, an item resolving exactly onto an auto-created
    /// folder path would treat that path as free.
    fn record_upsert(&mut self, node_ref: VirtualNodeRef) {
        for ancestor in ancestor_paths(&node_ref.path) {
            self.by_path.entry(ancestor).or_insert(PathEntry {
                id: None,
                mount_owned: false,
                etag: None,
            });
        }
        self.by_path.insert(
            node_ref.path.clone(),
            PathEntry {
                id: Some(node_ref.id.clone()),
                mount_owned: true,
                etag: node_ref.etag.clone(),
            },
        );
        self.by_external
            .insert(node_ref.external_id.clone(), node_ref);
    }

    /// Record a relocation performed by the remap pre-pass.
    fn record_move(&mut self, external_id: &str, new_path: &str) {
        if let Some(existing) = self.by_external.get_mut(external_id) {
            let old_path = std::mem::replace(&mut existing.path, new_path.to_string());
            let entry = self.by_path.remove(&old_path);
            self.by_path.insert(
                new_path.to_string(),
                entry.unwrap_or(PathEntry {
                    id: Some(existing.id.clone()),
                    mount_owned: true,
                    etag: existing.etag.clone(),
                }),
            );
        }
    }

    /// Drop a node this run just deleted.
    fn record_delete(&mut self, external_id: &str) {
        if let Some(existing) = self.by_external.remove(external_id) {
            self.by_path.remove(&existing.path);
        }
    }
}

/// One staged operation in a batch. Deletes and upserts share the queue so a
/// delta page that creates then deletes the same item applies in the order the
/// provider reported it.
// Deletes are much smaller than upserts, so the enum is sized by `Upsert`. Not
// worth boxing: a batch is overwhelmingly upserts (deletes only arrive from a
// delta tombstone or a reconcile pass), so the padding is rarely paid, and an
// allocation per operation is exactly the per-item cost this batching exists to
// remove.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum BatchOp {
    Upsert {
        rel_path: String,
        mapped: MappedNode,
        virt: VirtualMeta,
    },
    Delete {
        external_id: String,
    },
}

impl BatchOp {
    /// The external id this op targets, for order-preserving dedup.
    fn external_id(&self) -> &str {
        match self {
            BatchOp::Upsert { virt, .. } => &virt.external_id,
            BatchOp::Delete { external_id } => external_id,
        }
    }
}

/// Outcome counts for one batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchStats {
    pub written: usize,
    pub skipped: usize,
    pub deleted: usize,
    /// Items rejected individually (validation, RLS, a foreign node at the path)
    /// without aborting the batch.
    pub failed: usize,
}

impl BatchStats {
    pub fn merge(&mut self, other: BatchStats) {
        self.written += other.written;
        self.skipped += other.skipped;
        self.deleted += other.deleted;
        self.failed += other.failed;
    }
}

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
    async fn begin(
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

    /// Relocate already-synced nodes whose remapped path changed.
    ///
    /// Runs FIRST, in its own committed transaction, because a move and a write
    /// cannot be combined in one: the in-transaction read cache does not reflect
    /// the move, so writing after it collides on the id, and moving after a write
    /// relocates the pre-write version and discards the new mapping. Moving first
    /// leaves the ordinary upsert to find the node at its destination and update
    /// it in place.
    ///
    /// The node id is preserved throughout, so revision history and anything
    /// added locally survive the migration — delete-and-recreate loses all of it.
    async fn remap_moves(
        &self,
        scope: &MountScope,
        index: &mut SyncIndex,
        ops: &[BatchOp],
    ) -> Result<()> {
        let mut moves: Vec<(String, String, String)> = Vec::new(); // (ext, node_id, new_path)
        for op in ops {
            let BatchOp::Upsert { rel_path, virt, .. } = op else {
                continue;
            };
            let new_path = join_path(&scope.mount_path, rel_path);
            if let Some(existing) = index.by_external(&virt.external_id) {
                if existing.path != new_path {
                    moves.push((virt.external_id.clone(), existing.id.clone(), new_path));
                }
            }
        }
        if moves.is_empty() {
            return Ok(());
        }

        let tx = self.begin(scope, "virtual mount sync: remap move").await?;
        let mut applied: Vec<(String, String)> = Vec::new();
        for (external_id, node_id, new_path) in moves {
            ensure_folder_chain(tx.as_ref(), &scope.workspace, &new_path).await?;
            match tx
                .move_node_tree(&scope.workspace, &node_id, &new_path)
                .await
            {
                Ok(()) => applied.push((external_id, new_path)),
                Err(e) if is_item_level(&e) => {
                    tracing::warn!(
                        mount_id = %scope.mount_id,
                        external_id = %external_id,
                        new_path = %new_path,
                        error = %e,
                        "remap move rejected; leaving the node at its current path"
                    );
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    return Err(e);
                }
            }
        }
        tx.commit().await?;
        for (external_id, new_path) in applied {
            index.record_move(&external_id, &new_path);
        }
        Ok(())
    }

    /// Apply `ops` in one transaction. `allow_replay` guards the single-item
    /// retry so a failing replay cannot recurse.
    ///
    /// Returns the outcome counts plus any operations held back by the in-chunk
    /// unique guard, which the caller must write individually.
    async fn apply_chunk(
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
    fn replay<'s>(
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

    /// Stage one operation into the open transaction.
    #[allow(clippy::too_many_arguments)]
    async fn stage_op(
        &self,
        tx: &dyn TransactionalContext,
        scope: &MountScope,
        index: &SyncIndex,
        op: &BatchOp,
        pending: &mut Vec<IndexMutation>,
        unique_seen: &mut HashSet<(String, String, String)>,
        unique_props: &mut HashMap<String, Vec<String>>,
        defer_unique_collisions: bool,
    ) -> Result<Staged> {
        let (rel_path, mapped, virt) = match op {
            BatchOp::Delete { external_id } => {
                let Some(existing) = index.by_external(external_id) else {
                    return Ok(Staged::Skipped);
                };
                tx.delete_node(&scope.workspace, &existing.id).await?;
                pending.push(IndexMutation::Delete {
                    external_id: external_id.clone(),
                });
                return Ok(Staged::Deleted);
            }
            BatchOp::Upsert {
                rel_path,
                mapped,
                virt,
            } => (rel_path, mapped, virt),
        };

        let new_path = join_path(&scope.mount_path, rel_path);

        // 1. Match by __external_id within the mount subtree (survives renames).
        let (id, path) = match index.by_external(&virt.external_id) {
            Some(existing) => {
                // Etag skip-write: unchanged item → no revision churn. Bypassed
                // by a remap, which exists precisely to re-apply a mapper whose
                // output changed while the provider's item did not.
                if !scope.force_rewrite && virt.etag.is_some() && existing.etag == virt.etag {
                    return Ok(Staged::Skipped);
                }
                // Update in place, preserving id + current path (avoids dupes on
                // rename; a provider-side rename updates this node). On a remap
                // the relocation already happened in `remap_moves`, so this path
                // IS the destination.
                (existing.id.clone(), existing.path.clone())
            }
            None => {
                // 2. Fall back to a path match. A foreign (non-mount) node sitting
                // at the target path must not be clobbered.
                match index.at_path(&new_path) {
                    Some(entry) if !entry.mount_owned => {
                        tracing::warn!(
                            mount_id = %scope.mount_id,
                            path = %new_path,
                            "virtual mount upsert: foreign node occupies target path, skipping"
                        );
                        return Ok(Staged::Skipped);
                    }
                    Some(entry) => {
                        if virt.etag.is_some() && entry.etag == virt.etag {
                            return Ok(Staged::Skipped);
                        }
                        // A mount-owned entry without a known id can only come
                        // from an ancestor folder recorded this run; `upsert_node`
                        // matches by PATH, so a fresh id is harmless — the write
                        // still lands on the existing node.
                        (
                            entry.id.clone().unwrap_or_else(|| nanoid::nanoid!()),
                            new_path.clone(),
                        )
                    }
                    None => (nanoid::nanoid!(), new_path.clone()),
                }
            }
        };

        let name = mapped
            .name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("item").to_string());

        let node = Node {
            id,
            node_type: mapped.node_type.clone(),
            name,
            path: path.clone(),
            workspace: Some(scope.workspace.clone()),
            properties: build_properties(&mapped.properties, virt),
            ..Default::default()
        };

        // In-chunk unique guard (see `apply_chunk`). Only meaningful while more
        // than one item shares the transaction, hence the `defer_unique_collisions`
        // flag: a replay of one item must actually write it.
        if defer_unique_collisions
            && self
                .collides_on_unique(scope, &node, unique_seen, unique_props)
                .await?
        {
            tracing::debug!(
                mount_id = %scope.mount_id,
                external_id = %virt.external_id,
                "unique property collides within the batch; deferring to a single-item write"
            );
            return Ok(Staged::Deferred);
        }

        // Deep upsert auto-creates any missing parent folders.
        tx.upsert_deep_node(&scope.workspace, &node, "raisin:Folder")
            .await?;

        pending.push(IndexMutation::Upsert(VirtualNodeRef {
            id: node.id,
            path,
            external_id: virt.external_id.clone(),
            etag: virt.etag.clone(),
            synced_secs: chrono::DateTime::parse_from_rfc3339(&virt.synced_at)
                .ok()
                .map(|d| d.timestamp()),
        }));
        Ok(Staged::Written)
    }

    /// Whether `node` reuses a `unique: true` property value already claimed by
    /// an earlier item in this chunk. Records the values either way.
    async fn collides_on_unique(
        &self,
        scope: &MountScope,
        node: &Node,
        unique_seen: &mut HashSet<(String, String, String)>,
        unique_props: &mut HashMap<String, Vec<String>>,
    ) -> Result<bool> {
        if !unique_props.contains_key(&node.node_type) {
            let names = self.unique_property_names(scope, &node.node_type).await?;
            unique_props.insert(node.node_type.clone(), names);
        }
        let Some(names) = unique_props.get(&node.node_type) else {
            return Ok(false);
        };
        if names.is_empty() {
            return Ok(false);
        }
        let mut claims = Vec::new();
        for name in names {
            let Some(value) = node.properties.get(name) else {
                continue;
            };
            let key = (
                node.node_type.clone(),
                name.clone(),
                crate::repositories::hash_property_value(value),
            );
            if unique_seen.contains(&key) {
                return Ok(true);
            }
            claims.push(key);
        }
        unique_seen.extend(claims);
        Ok(false)
    }

    /// Property names declared `unique: true` on a node type (empty when the
    /// type is unknown — the write path makes the same assumption).
    async fn unique_property_names(
        &self,
        scope: &MountScope,
        node_type: &str,
    ) -> Result<Vec<String>> {
        use raisin_storage::{NodeTypeRepository, Storage};
        let repo = self.storage.node_types();
        let found = repo
            .get(
                raisin_storage::BranchScope::new(&scope.tenant, &scope.repo, &scope.branch),
                node_type,
                None,
            )
            .await?;
        Ok(found
            .map(|nt| crate::repositories::extract_unique_property_names(&nt))
            .unwrap_or_default())
    }
}

/// What staging one operation did.
enum Staged {
    Written,
    Deleted,
    Skipped,
    /// Held back for a single-item write (an in-chunk unique collision).
    Deferred,
}

/// An index update held until the transaction commits.
enum IndexMutation {
    Upsert(VirtualNodeRef),
    Delete { external_id: String },
}

impl IndexMutation {
    fn apply(self, index: &mut SyncIndex) {
        match self {
            IndexMutation::Upsert(node_ref) => index.record_upsert(node_ref),
            IndexMutation::Delete { external_id } => index.record_delete(&external_id),
        }
    }
}

/// Whether an error rejects ONE item rather than poisoning the transaction.
///
/// This distinction is what makes skip-and-continue safe: in `add_node` and
/// `put_node` every check that produces one of these runs BEFORE the first batch
/// write, so a rejected item contributed nothing to the shared `WriteBatch`.
fn is_item_level(error: &Error) -> bool {
    matches!(
        error,
        Error::NotFound(_)
            | Error::AlreadyExists(_)
            | Error::Validation(_)
            | Error::Conflict(_)
            | Error::Unauthorized(_)
            | Error::Forbidden(_)
            | Error::PermissionDenied(_)
    )
}

/// Collapse operations that target the same item or the same path, keeping the
/// LAST — a page's later entry is the newer state, and this is also what makes a
/// create-then-delete pair resolve to the delete.
///
/// Without this a duplicated `external_id` whose two occurrences resolve to
/// different paths produces TWO nodes claiming the same external id; the next
/// sync matches one arbitrarily and the other is orphaned forever, invisible to
/// reconcile because its external id IS in `seen`.
pub fn dedup_ops(ops: Vec<BatchOp>, mount_path: &str) -> Vec<BatchOp> {
    let mut keep_by_external: HashMap<&str, usize> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        keep_by_external.insert(op.external_id(), i);
    }
    let mut keep_by_path: HashMap<String, usize> = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        if keep_by_external.get(op.external_id()) != Some(&i) {
            continue;
        }
        if let BatchOp::Upsert { rel_path, .. } = op {
            let path = join_path(mount_path, rel_path);
            if let Some(prev) = keep_by_path.insert(path.clone(), i) {
                tracing::warn!(
                    path = %path,
                    first = %ops[prev].external_id(),
                    second = %op.external_id(),
                    "two external items resolve to the same node path; keeping the last. \
                     Check the mount's path_template or mapping function — the losing item \
                     will be re-imported and overwrite this one on every sync."
                );
            }
        }
    }

    ops.iter()
        .enumerate()
        .filter(|(i, op)| {
            if keep_by_external.get(op.external_id()) != Some(i) {
                return false;
            }
            match op {
                BatchOp::Upsert { rel_path, .. } => {
                    keep_by_path.get(&join_path(mount_path, rel_path)) == Some(i)
                }
                BatchOp::Delete { .. } => true,
            }
        })
        .map(|(_, op)| op.clone())
        .collect()
}

/// Approximate serialized size of an item, for the batch byte budget. Mail and
/// document bodies dominate, so the property payload is what is measured.
pub fn estimate_op_bytes(op: &BatchOp) -> usize {
    match op {
        BatchOp::Delete { external_id } => external_id.len() + 64,
        BatchOp::Upsert {
            rel_path,
            mapped,
            virt,
        } => {
            let props: usize = mapped
                .properties
                .iter()
                .map(|(k, v)| k.len() + json_size(v))
                .sum();
            props
                + rel_path.len()
                + mapped.node_type.len()
                + mapped.name.as_deref().map_or(0, str::len)
                + virt.external_id.len()
                + 256 // node envelope: id, timestamps, reserved props
        }
    }
}

/// Cheap size estimate for a JSON value — no allocation, unlike re-serializing.
fn json_size(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(_) => 12,
        serde_json::Value::String(s) => s.len() + 2,
        serde_json::Value::Array(a) => 2 + a.iter().map(json_size).sum::<usize>(),
        serde_json::Value::Object(o) => {
            2 + o
                .iter()
                .map(|(k, v)| k.len() + 3 + json_size(v))
                .sum::<usize>()
        }
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

/// Read `__mount_id` from a node.
fn node_mount_id(node: &Node) -> Option<&str> {
    match node.properties.get("__mount_id")? {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Read `__external_id` from a node.
fn node_external_id(node: &Node) -> Option<&str> {
    match node.properties.get("__external_id")? {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Read a `__`-prefixed string property.
fn node_str_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Read `__synced_at` as unix epoch seconds. The storage layer may coerce an
/// ISO string into a `Date`, so accept both.
fn node_synced_secs(node: &Node) -> Option<i64> {
    match node.properties.get("__synced_at")? {
        PropertyValue::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp()),
        PropertyValue::Date(d) => Some(d.timestamp()),
        _ => None,
    }
}

/// Whether `path` is at or under `prefix`.
fn under(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Join the mount path with a relative path into an absolute node path.
fn join_path(mount_path: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return mount_path.to_string();
    }
    if mount_path == "/" {
        format!("/{rel}")
    } else {
        format!("{mount_path}/{rel}")
    }
}

/// Every ancestor path of `path`, excluding `path` itself and the root.
fn ancestor_paths(path: &str) -> Vec<String> {
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for seg in &segments[..segments.len().saturating_sub(1)] {
        current.push('/');
        current.push_str(seg);
        out.push(current.clone());
    }
    out
}

/// Create any missing ancestor folders of `path`.
///
/// Only the MISSING ones: an `upsert_deep_node` on an existing folder matches by
/// path and would overwrite that folder's properties with an empty stub, which
/// on a conversation folder would wipe the thread subject the hierarchy exists
/// to display.
async fn ensure_folder_chain(
    tx: &dyn TransactionalContext,
    workspace: &str,
    path: &str,
) -> Result<()> {
    for ancestor in ancestor_paths(path) {
        if tx.get_node_by_path(workspace, &ancestor).await?.is_some() {
            continue;
        }
        let name = ancestor.rsplit('/').next().unwrap_or("folder").to_string();
        let folder = Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name,
            path: ancestor,
            workspace: Some(workspace.to_string()),
            ..Default::default()
        };
        tx.add_node(workspace, &folder).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert_op(ext: &str, rel_path: &str, etag: Option<&str>) -> BatchOp {
        BatchOp::Upsert {
            rel_path: rel_path.to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: None,
                properties: serde_json::Map::new(),
            },
            virt: VirtualMeta {
                mount_id: "m1".to_string(),
                external_id: ext.to_string(),
                etag: etag.map(str::to_string),
                synced_at: "2026-01-01T00:00:00Z".to_string(),
            },
        }
    }

    #[test]
    fn join_and_under_paths() {
        assert_eq!(join_path("/docs", "a/b"), "/docs/a/b");
        assert_eq!(join_path("/docs", "/a"), "/docs/a");
        assert_eq!(join_path("/", "a"), "/a");
        assert!(under("/docs", "/docs/a"));
        assert!(under("/docs", "/docs"));
        assert!(!under("/docs", "/documents"));
        assert!(under("/", "/anything"));
    }

    #[test]
    fn ancestors_exclude_self_and_root() {
        assert_eq!(
            ancestor_paths("/docs/a/b"),
            vec!["/docs".to_string(), "/docs/a".to_string()]
        );
        assert!(ancestor_paths("/docs").is_empty());
        assert!(ancestor_paths("/").is_empty());
    }

    #[test]
    fn dedup_keeps_the_last_op_per_external_id() {
        let ops = vec![
            upsert_op("a", "a.txt", Some("v1")),
            upsert_op("b", "b.txt", Some("v1")),
            upsert_op("a", "a.txt", Some("v2")),
        ];
        let out = dedup_ops(ops, "/docs");
        assert_eq!(out.len(), 2);
        let a = out
            .iter()
            .find(|o| o.external_id() == "a")
            .expect("a survives");
        match a {
            BatchOp::Upsert { virt, .. } => assert_eq!(virt.etag.as_deref(), Some("v2")),
            _ => panic!("expected an upsert"),
        }
    }

    #[test]
    fn create_then_delete_in_one_page_resolves_to_the_delete() {
        let ops = vec![
            upsert_op("a", "a.txt", Some("v1")),
            BatchOp::Delete {
                external_id: "a".to_string(),
            },
        ];
        let out = dedup_ops(ops, "/docs");
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], BatchOp::Delete { .. }));
    }

    #[test]
    fn dedup_keeps_the_last_op_per_resolved_path() {
        let ops = vec![
            upsert_op("a", "same.txt", Some("v1")),
            upsert_op("b", "same.txt", Some("v1")),
        ];
        let out = dedup_ops(ops, "/docs");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].external_id(), "b");
    }

    #[test]
    fn index_serves_external_path_and_foreign_lookups() {
        let mut mount_owned = Node {
            id: "n1".to_string(),
            path: "/docs/a.txt".to_string(),
            ..Default::default()
        };
        mount_owned.properties.insert(
            "__mount_id".to_string(),
            PropertyValue::String("m1".to_string()),
        );
        mount_owned.properties.insert(
            "__external_id".to_string(),
            PropertyValue::String("ext-a".to_string()),
        );
        mount_owned.properties.insert(
            "__etag".to_string(),
            PropertyValue::String("v1".to_string()),
        );
        let foreign = Node {
            id: "n2".to_string(),
            path: "/docs/user.txt".to_string(),
            ..Default::default()
        };
        let outside = Node {
            id: "n3".to_string(),
            path: "/elsewhere/x.txt".to_string(),
            ..Default::default()
        };

        let idx = SyncIndex::from_nodes(vec![mount_owned, foreign, outside], "m1", "/docs");

        assert_eq!(idx.by_external("ext-a").map(|n| n.id.as_str()), Some("n1"));
        assert_eq!(
            idx.by_external("ext-a")
                .and_then(|n| n.etag.clone())
                .as_deref(),
            Some("v1")
        );
        // The foreign node is visible by path — that guard depends on it.
        assert_eq!(
            idx.at_path("/docs/user.txt").map(|e| e.mount_owned),
            Some(false)
        );
        assert_eq!(
            idx.at_path("/docs/a.txt").map(|e| e.mount_owned),
            Some(true)
        );
        // Nodes outside the mount path are not indexed at all.
        assert!(idx.at_path("/elsewhere/x.txt").is_none());
        assert_eq!(idx.virtual_len(), 1);
    }

    #[test]
    fn recording_a_write_also_marks_its_ancestor_folders_occupied() {
        let mut idx = SyncIndex::default();
        idx.record_upsert(VirtualNodeRef {
            id: "n1".to_string(),
            path: "/docs/thread-1/msg.txt".to_string(),
            external_id: "ext-a".to_string(),
            etag: Some("v1".to_string()),
            synced_secs: None,
        });
        assert!(idx.at_path("/docs/thread-1").is_some());
        assert_eq!(
            idx.at_path("/docs/thread-1").map(|e| e.mount_owned),
            Some(false)
        );
        assert_eq!(
            idx.at_path("/docs/thread-1/msg.txt").map(|e| e.mount_owned),
            Some(true)
        );
        assert_eq!(idx.by_external("ext-a").map(|n| n.id.as_str()), Some("n1"));

        idx.record_delete("ext-a");
        assert!(idx.by_external("ext-a").is_none());
        assert!(idx.at_path("/docs/thread-1/msg.txt").is_none());
    }

    #[test]
    fn byte_estimate_tracks_the_property_payload() {
        let small = upsert_op("a", "a.txt", None);
        let mut big_props = serde_json::Map::new();
        big_props.insert(
            "body".to_string(),
            serde_json::Value::String("x".repeat(50_000)),
        );
        let big = BatchOp::Upsert {
            rel_path: "b.txt".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: None,
                properties: big_props,
            },
            virt: VirtualMeta {
                mount_id: "m1".to_string(),
                external_id: "b".to_string(),
                etag: None,
                synced_at: "2026-01-01T00:00:00Z".to_string(),
            },
        };
        assert!(estimate_op_bytes(&big) > 50_000);
        assert!(estimate_op_bytes(&small) < 1_000);
    }
}
