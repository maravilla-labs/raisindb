//! Node repository implementation
//!
//! This module provides the RocksDB-backed implementation of the NodeRepository trait.
//! The implementation is split across multiple modules for maintainability:
//!
//! - `crud`: Core CRUD operations (create, read, update, delete) - now a module directory
//!   - `cascade`: Cascade deletion operations (within crud)
//! - `queries`: Query operations (filtering, searching, listing)
//! - `ordering`: Child ordering with fractional indexing
//! - `mvcc`: Time-travel and MVCC operations
//! - `publishing`: Publishing/unpublishing operations
//! - `helpers`: Utility functions
//! - `trait_impl`: NodeRepository trait delegation to internal methods

pub(crate) mod crud; // This is now a directory with sub-modules
pub mod helpers;
mod mvcc;
mod ordering;
mod publishing;
mod queries;
mod replication_capture;
mod storage_node;
mod trait_impl;
mod validation;

// Re-export StorageNode for use within this crate
pub(crate) use storage_node::StorageNode;

// Attribution carried into replication capture by every direct (non-transaction)
// write path. The transaction path resolves the same pair from its AuthContext.
pub(crate) use replication_capture::WriteAttribution;

// Re-export hash_property_value for use by property_index repository
pub(crate) use helpers::hash_property_value;

// Re-export the stale property-index tombstone helper for the transactional
// write path (both write paths must keep the property index hygienic).
pub(crate) use crud::indexing::property_indexes::add_stale_property_tombstones;
pub(crate) use crud::indexing::reference_indexes::{
    add_reference_index_entries, add_stale_reference_tombstones, walk_references,
};

use raisin_error::Result;
use raisin_events::EventBus;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchRepository;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct NodeRepositoryImpl {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
    revision_repo: Arc<crate::repositories::RevisionRepositoryImpl>,
    branch_repo: Arc<crate::repositories::BranchRepositoryImpl>,
    tag_repo: Arc<crate::repositories::TagRepositoryImpl>,
    pub(crate) node_type_repo: Arc<crate::repositories::NodeTypeRepositoryImpl>,
    workspace_repo: Arc<crate::repositories::WorkspaceRepositoryImpl>,
    /// Per-parent ordering locks to prevent concurrent modification of child order
    /// Key format: {tenant_id}/{repo_id}/{branch}/{workspace}/{parent_id}
    ordering_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    /// Operation capture for replication
    operation_capture: Arc<crate::OperationCapture>,
    /// In-flight CREATE path reservations enforcing path uniqueness.
    ///
    /// Create paths do a read-check ("does a node already exist at this path?")
    /// followed by a later write (the WriteBatch commit). Two concurrent creates
    /// for the same path can both pass the read-check before either write lands,
    /// producing two physical rows at one path. This registry closes that TOCTOU
    /// window: a create must reserve `{tenant}\u{1}{repo}\u{1}{branch}\u{1}{workspace}\u{1}{path}`
    /// (keyed to an owner token) BEFORE performing the committed-storage
    /// existence check, and releases it after its write is durable (or on
    /// rollback/abort). Reserve-then-check guarantees that whoever wins the
    /// reservation either sees the loser's committed row or is the only writer.
    ///
    /// Values are opaque owner tokens from [`Self::new_path_reservation_owner`],
    /// so a transaction can reserve the same path idempotently while a different
    /// owner gets a Conflict. This is an in-process guard: it protects all write
    /// paths of a single node (which share this repository). Cross-node
    /// uniqueness under CRDT replication is out of scope here.
    path_reservations: Arc<StdMutex<HashMap<String, u64>>>,
    /// Monotonic source of reservation owner tokens.
    reservation_owner_counter: Arc<AtomicU64>,
}

impl NodeRepositoryImpl {
    pub fn new(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        revision_repo: Arc<crate::repositories::RevisionRepositoryImpl>,
        branch_repo: Arc<crate::repositories::BranchRepositoryImpl>,
        tag_repo: Arc<crate::repositories::TagRepositoryImpl>,
        node_type_repo: Arc<crate::repositories::NodeTypeRepositoryImpl>,
        workspace_repo: Arc<crate::repositories::WorkspaceRepositoryImpl>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            event_bus,
            revision_repo,
            branch_repo,
            tag_repo,
            node_type_repo,
            workspace_repo,
            ordering_locks: Arc::new(Mutex::new(HashMap::new())),
            operation_capture,
            path_reservations: Arc::new(StdMutex::new(HashMap::new())),
            reservation_owner_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Allocate a fresh owner token for CREATE path reservations.
    ///
    /// Each logical creator (a transaction, or a single non-transactional
    /// `create` call) gets its own token so reservations by the same owner are
    /// idempotent while different owners conflict.
    pub(crate) fn new_path_reservation_owner(&self) -> u64 {
        self.reservation_owner_counter
            .fetch_add(1, Ordering::Relaxed)
    }

    /// Build the reservation key for a scoped path.
    pub(crate) fn path_reservation_key(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> String {
        format!(
            "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
            tenant_id, repo_id, branch, workspace, path
        )
    }

    /// Reserve a path for creation, failing with `Conflict` if another
    /// in-flight creator already holds it.
    ///
    /// Returns the reservation key on success so the caller can release it
    /// later. Reserving a path already held by the SAME owner is a no-op
    /// (idempotent within one transaction).
    ///
    /// IMPORTANT ordering contract: reserve FIRST, then perform the
    /// committed-storage existence check. Release only happens after the
    /// creator's write is durable (or it aborts), so a winner's committed row
    /// is always visible to the next reserver's existence check.
    pub(crate) fn try_reserve_create_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        owner: u64,
    ) -> Result<String> {
        let key = Self::path_reservation_key(tenant_id, repo_id, branch, workspace, path);
        let mut reservations = self
            .path_reservations
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Reservation lock error: {}", e)))?;
        match reservations.get(&key) {
            Some(existing_owner) if *existing_owner != owner => {
                Err(raisin_error::Error::Conflict(format!(
                    "Node with path '{}' is being created by a concurrent operation",
                    path
                )))
            }
            _ => {
                reservations.insert(key.clone(), owner);
                Ok(key)
            }
        }
    }

    /// Reserve a path for creation, WAITING (bounded) for a foreign in-flight
    /// reservation to clear instead of failing immediately.
    ///
    /// Used where a concurrent creator of the same path is potentially
    /// CONVERGENCE rather than conflict — e.g. two deep creates racing to
    /// materialize the same missing INTERMEDIATE folder. The loser waits for
    /// the winner to finish (reservation released after its durable write),
    /// wins the reservation itself, and then re-runs the committed-storage
    /// existence check: if the winner committed the folder, the caller sees
    /// it and converges on the existing row; if the winner aborted, the
    /// caller creates the folder itself.
    ///
    /// Deadlock-safety: all multi-path reservers acquire ancestor paths
    /// before descendant paths (top-down), so waits never form a cycle.
    ///
    /// Returns `Conflict` only if the foreign reservation is still held after
    /// `timeout` (e.g. a long-running transaction holds it until commit).
    pub(crate) async fn try_reserve_create_path_waiting(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        owner: u64,
        timeout: std::time::Duration,
    ) -> Result<String> {
        let start = std::time::Instant::now();
        loop {
            match self.try_reserve_create_path(tenant_id, repo_id, branch, workspace, path, owner) {
                Ok(key) => return Ok(key),
                Err(err @ raisin_error::Error::Conflict(_)) => {
                    if start.elapsed() >= timeout {
                        return Err(err);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Release CREATE path reservations held by `owner`.
    ///
    /// Only removes entries that still belong to the owner, so a stale release
    /// can never drop a newer reservation.
    pub(crate) fn release_create_path_reservations(&self, owner: u64, keys: &[String]) {
        // A poisoned mutex only means another thread panicked while holding it;
        // the map itself is still coherent, so recover and release anyway
        // (leaking reservations would wedge future creates at these paths).
        let mut reservations = match self.path_reservations.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for key in keys {
            if reservations.get(key) == Some(&owner) {
                reservations.remove(key);
            }
        }
    }

    /// Acquire an ordering lock for a parent to prevent concurrent modifications
    ///
    /// This ensures that only one ordering operation can modify a parent's children at a time,
    /// preventing race conditions in fractional index label assignment.
    async fn acquire_ordering_lock(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Arc<Mutex<()>> {
        let lock_key = format!(
            "{}/{}/{}/{}/{}",
            tenant_id, repo_id, branch, workspace, parent_id
        );

        let mut locks = self.ordering_locks.lock().await;
        locks
            .entry(lock_key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Resolve the HEAD revision for a branch or tag
    ///
    /// First tries to resolve as a branch. If that fails, checks if it's a tag
    /// and returns the tag's revision.
    async fn resolve_head_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_or_tag: &str,
    ) -> Result<Option<HLC>> {
        // First try as a branch
        match self
            .branch_repo
            .get_head(tenant_id, repo_id, branch_or_tag)
            .await
        {
            Ok(head) => Ok(Some(head)),
            Err(_) => {
                // If branch lookup fails, try as a tag
                use raisin_storage::TagRepository;
                match self
                    .tag_repo
                    .get_tag(tenant_id, repo_id, branch_or_tag)
                    .await?
                {
                    Some(tag) => Ok(Some(tag.revision)),
                    None => Ok(None),
                }
            }
        }
    }

    /// Add a brand new node (optimized - for testing)
    ///
    /// This is exposed for testing purposes to measure performance difference
    /// between put() and add() operations.
    #[allow(dead_code)]
    pub async fn add(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: Node,
    ) -> Result<()> {
        self.add_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            WriteAttribution::default(),
        )
        .await
    }
}

/// RAII guard bundling a CREATE path-reservation owner token with the
/// reservation keys it holds.
///
/// This is the single reservation idiom for every NON-transactional
/// path-creating write (`dispatch_create`, `create_deep_node_impl`,
/// `copy_node_impl`, `copy_node_tree_impl`); transactions carry their own
/// owner token for the transaction's lifetime instead (see
/// `RocksDBTransaction::reserve_create_path`).
///
/// Contract (same as the registry): reserve BEFORE the committed-storage
/// existence check; the guard releases everything on drop, which the caller
/// arranges to happen only after its write is durable — including on error
/// paths and future cancellation, so reservations can never leak.
pub(crate) struct PathReservationGuard<'a> {
    repo: &'a NodeRepositoryImpl,
    owner: u64,
    keys: Vec<String>,
}

impl<'a> PathReservationGuard<'a> {
    /// Start a new logical creator: allocates a fresh owner token.
    pub(crate) fn new(repo: &'a NodeRepositoryImpl) -> Self {
        Self {
            repo,
            owner: repo.new_path_reservation_owner(),
            keys: Vec::new(),
        }
    }

    /// Reserve `path`, failing immediately with `Conflict` if another
    /// in-flight creator holds it. Idempotent for this guard's owner.
    pub(crate) fn reserve(
        &mut self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<()> {
        let key = self
            .repo
            .try_reserve_create_path(tenant_id, repo_id, branch, workspace, path, self.owner)?;
        if !self.keys.contains(&key) {
            self.keys.push(key);
        }
        Ok(())
    }

    /// Reserve `path`, waiting (bounded) for a foreign in-flight reservation
    /// to clear first. See `try_reserve_create_path_waiting` for when this is
    /// the right semantic (intermediate-folder convergence in deep creates).
    pub(crate) async fn reserve_waiting(
        &mut self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        timeout: std::time::Duration,
    ) -> Result<()> {
        let key = self
            .repo
            .try_reserve_create_path_waiting(
                tenant_id, repo_id, branch, workspace, path, self.owner, timeout,
            )
            .await?;
        if !self.keys.contains(&key) {
            self.keys.push(key);
        }
        Ok(())
    }

    /// Release the reservation for one path early.
    ///
    /// Used when a reserved path turns out to already exist and this owner
    /// will therefore NOT create it (deep-create convergence on an existing
    /// intermediate folder) — holding it any longer would only block other
    /// creators that want to converge on the same existing row.
    pub(crate) fn release_path(
        &mut self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) {
        let key =
            NodeRepositoryImpl::path_reservation_key(tenant_id, repo_id, branch, workspace, path);
        if let Some(pos) = self.keys.iter().position(|k| k == &key) {
            let key = self.keys.swap_remove(pos);
            self.repo
                .release_create_path_reservations(self.owner, std::slice::from_ref(&key));
        }
    }
}

impl Drop for PathReservationGuard<'_> {
    fn drop(&mut self) {
        if !self.keys.is_empty() {
            self.repo
                .release_create_path_reservations(self.owner, &self.keys);
        }
    }
}
