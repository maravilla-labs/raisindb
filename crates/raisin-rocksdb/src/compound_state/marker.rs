//! Invalidating compound index state for writes this node did not index, and
//! the build's compare-and-set.
//!
//! Every write path maintains compound entries — the replicated upsert and the
//! merge resolution too, since plan Phase 8 step 3 — EXCEPT a write that
//! cannot: a replicated upsert whose node-type definitions are not cached
//! (resolving them on the apply path is the NodeType read the deadlock rule
//! forbids), the legacy apply arms, and a commit-time correction with cold
//! definitions. After one of those this node's keyspace no longer matches its
//! node records, and a `Ready` record would let the planner serve stale rows.
//! So such a write marks every recorded index of the workspace `NotBuilt` —
//! fail closed: those queries scan until the local build it requests runs.
//!
//! The marker is a monotonic counter (`stale_generation`), and every mark
//! also clears the registered build's TICKET (`build_token`, see
//! `build_cas.rs`). A build stamps `Ready` only while the record still
//! carries its own ticket; otherwise a write it may not have seen arrived
//! mid-build and its `Ready` would overwrite the marker that write set. The store refuses any write that
//! LOWERS the counter (`CompoundStateStore::stage`), so no path can reset it
//! and re-open that window.
//!
//! Every transition is serialized by one process-wide lock — the state is per
//! node, so per process is the scope that matters. A write that needs a mark
//! goes through [`CompoundStateStore::write_marking_stale`]: the mark rides in
//! the SAME `WriteBatch` as the write, staged and written under the lock. So
//! there is no moment — not even across a crash — where the write is durable
//! and the mark is not, and a build cannot begin between the two: a build that
//! begins after the write sees the advanced generation, and one that began
//! before it loses its compare-and-set.

use std::sync::Mutex;

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState};
use rocksdb::WriteBatch;

use super::store::{read_state, CompoundStateStore};
use crate::{cf, cf_handle};

/// Serializes every compound state transition that must not interleave.
static TRANSITIONS: Mutex<()> = Mutex::new(());

pub(super) fn transitions() -> std::sync::MutexGuard<'static, ()> {
    // The guarded section only reads and writes RocksDB; a panic in it leaves
    // nothing half-updated in memory, so a poisoned lock is still usable.
    TRANSITIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Which of a workspace's compound state records a mark covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleScope {
    /// Every record.
    All,
    /// The node types' indexes only (a write whose types were cold but whose
    /// workspace indexes it maintained).
    TypeOwned,
    /// The workspace's own indexes only (a declaration change).
    WorkspaceOwned,
}

impl StaleScope {
    fn covers(self, index_name: &str) -> bool {
        let workspace_owned = CompoundIndexDefinition::is_workspace_index_name(index_name);
        match self {
            StaleScope::All => true,
            StaleScope::TypeOwned => !workspace_owned,
            StaleScope::WorkspaceOwned => workspace_owned,
        }
    }
}

/// Mark `state` stale: `NotBuilt`, generation advanced, and the registered
/// build's ticket cleared — the build in flight can no longer stamp `Ready`.
pub(super) fn mark(state: &mut CompoundIndexState) {
    state.phase = CompoundBuildPhase::NotBuilt;
    state.stale_generation = state.stale_generation.saturating_add(1);
    state.build_token = 0;
}

impl CompoundStateStore {
    /// Write one record now, under the transition lock. Refused if it would
    /// lower the stored generation.
    pub fn put(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        state: &CompoundIndexState,
    ) -> Result<()> {
        let _guard = transitions();
        self.put_unlocked(tenant_id, repo_id, branch, workspace, state)
    }

    /// Write `batch` — a write this node did not compound-index — together
    /// with a `NotBuilt` mark for every recorded compound index of `workspace`,
    /// atomically. Returns how many records were marked.
    ///
    /// The indexes are found from their STATE RECORDS — never from a NodeType
    /// read, which the replication apply path must not perform (the deadlock
    /// rule). An index with no record yet reads `NotBuilt` already.
    pub fn write_marking_stale(
        &self,
        batch: WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<usize> {
        self.write_marking_stale_in(
            batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            StaleScope::All,
        )
    }

    /// [`Self::write_marking_stale`] for the records `scope` covers only — a
    /// write that maintained the workspace's own indexes but not its types'
    /// (or the reverse) leaves the ones it did maintain `Ready`.
    pub fn write_marking_stale_in(
        &self,
        mut batch: WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        scope: StaleScope,
    ) -> Result<usize> {
        let _guard = transitions();
        let states = self.list_for_workspace(tenant_id, repo_id, branch, workspace)?;
        let mut keys = Vec::with_capacity(states.len());
        for mut state in states {
            if !scope.covers(&state.index_name) {
                continue;
            }
            mark(&mut state);
            keys.push(self.stage(&mut batch, tenant_id, repo_id, branch, workspace, &state)?);
        }
        if batch.is_empty() {
            return Ok(0);
        }
        self.db
            .write(batch)
            .map_err(|e| Error::storage(format!("Failed to write marked batch: {}", e)))?;
        for key in &keys {
            self.invalidate(key);
        }
        Ok(keys.len())
    }

    /// Mark every recorded compound index of `workspace` `NotBuilt`, on its
    /// own. Prefer [`Self::write_marking_stale`] for a write that needs the
    /// mark: a separate mark leaves a crash window between the two.
    pub fn mark_workspace_stale(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<usize> {
        self.mark_workspace_stale_in(tenant_id, repo_id, branch, workspace, StaleScope::All)
    }

    /// [`Self::mark_workspace_stale`] for the records `scope` covers.
    pub fn mark_workspace_stale_in(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        scope: StaleScope,
    ) -> Result<usize> {
        self.write_marking_stale_in(
            WriteBatch::default(),
            tenant_id,
            repo_id,
            branch,
            workspace,
            scope,
        )
    }

    /// Flip one index's record to `NotBuilt` (its declaration changed), and
    /// advance its generation so a build already running under the old
    /// declaration cannot stamp `Ready` over it. No-op when there is no record —
    /// absent already reads as `NotBuilt`.
    pub fn mark_not_built(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        index_name: &str,
    ) -> Result<()> {
        let _guard = transitions();
        if let Some(mut state) =
            read_state(&self.db, tenant_id, repo_id, branch, workspace, index_name)?
        {
            mark(&mut state);
            self.put_unlocked(tenant_id, repo_id, branch, workspace, &state)?;
        }
        Ok(())
    }

    /// Mark the indexes named `names` `NotBuilt` in every workspace of the
    /// branch that has a record for them (a declaration changed: the NodeType
    /// names no workspace). Returns the workspaces marked.
    pub fn mark_names_on_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        names: &[String],
    ) -> Result<Vec<String>> {
        let _guard = transitions();
        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let prefix = format!("compound_index\0{tenant_id}\0{repo_id}\0{branch}\0").into_bytes();
        let mut marked: Vec<(String, CompoundIndexState)> = Vec::new();
        for item in crate::prefix_scan(&self.db, cf, &prefix) {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("compound state scan failed: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            // Remainder: `{workspace}\0{index_name}`.
            let rest = String::from_utf8_lossy(&key[prefix.len()..]).into_owned();
            let Some((workspace, name)) = rest.split_once('\0') else {
                continue;
            };
            if !names.iter().any(|n| n == name) {
                continue;
            }
            if let Ok(mut state) = rmp_serde::from_slice::<CompoundIndexState>(&value) {
                mark(&mut state);
                marked.push((workspace.to_string(), state));
            }
        }
        let mut workspaces = Vec::new();
        for (workspace, state) in marked {
            self.put_unlocked(tenant_id, repo_id, branch, &workspace, &state)?;
            if !workspaces.contains(&workspace) {
                workspaces.push(workspace);
            }
        }
        Ok(workspaces)
    }

    /// Mark EVERY compound state record on this node stale.
    ///
    /// For a checkpoint ingest, which copies a peer's state records verbatim: a
    /// peer's `Ready` describes the peer's apply history, never this node's.
    pub fn mark_all_stale(&self) -> Result<usize> {
        let _guard = transitions();
        let cf = cf_handle(&self.db, cf::INDEX_STATUS)?;
        let prefix = b"compound_index\0".to_vec();
        let mut batch = WriteBatch::default();
        let mut marked = 0;
        for item in crate::prefix_scan(&self.db, cf, &prefix) {
            let (key, value) =
                item.map_err(|e| Error::storage(format!("compound state scan failed: {}", e)))?;
            if !key.starts_with(&prefix) {
                break;
            }
            let Ok(mut state) = rmp_serde::from_slice::<CompoundIndexState>(&value) else {
                continue;
            };
            mark(&mut state);
            let bytes = rmp_serde::to_vec(&state).map_err(|e| {
                Error::storage(format!("Failed to serialize compound index state: {}", e))
            })?;
            batch.put_cf(cf, &key, bytes);
            marked += 1;
        }
        self.db
            .write(batch)
            .map_err(|e| Error::storage(format!("Failed to mark compound state stale: {}", e)))?;
        self.clear_cache();
        Ok(marked)
    }
}
