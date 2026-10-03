//! Invalidating compound index state for writes this node did not index, and
//! the build's compare-and-set.
//!
//! Compound and unique entries are written by the local write paths only. A
//! replicated upsert and a merge apply write none (until Phase 8 step 3 / 2.10),
//! so after either one this node's keyspace no longer matches its node records,
//! and a `Ready` record would let the planner serve stale rows. The apply path
//! therefore marks every recorded index of the workspace `NotBuilt` — fail
//! closed: those queries scan until a local rebuild runs.
//!
//! The marker is a monotonic counter (`stale_generation`). A build reads it
//! when it starts and stamps `Ready` only if it is unchanged when it finishes;
//! otherwise a write it may not have seen arrived mid-build and its `Ready`
//! would overwrite the marker that write set. The store refuses any write that
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

fn transitions() -> std::sync::MutexGuard<'static, ()> {
    // The guarded section only reads and writes RocksDB; a panic in it leaves
    // nothing half-updated in memory, so a poisoned lock is still usable.
    TRANSITIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Mark `state` stale: `NotBuilt`, generation advanced.
fn mark(state: &mut CompoundIndexState) {
    state.phase = CompoundBuildPhase::NotBuilt;
    state.stale_generation = state.stale_generation.saturating_add(1);
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
        mut batch: WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<usize> {
        let _guard = transitions();
        let states = self.list_for_workspace(tenant_id, repo_id, branch, workspace)?;
        let mut keys = Vec::with_capacity(states.len());
        for mut state in states {
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
        self.write_marking_stale(WriteBatch::default(), tenant_id, repo_id, branch, workspace)
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

    /// Register a build of `definition` and return the generation it runs
    /// under, to be handed back to [`Self::complete_build`].
    ///
    /// The FIRST build of an index also writes a `Building` record, so that a
    /// mark arriving during it has a record to advance. A rebuild of an index
    /// that already has a record leaves the record's phase alone — use
    /// [`Self::begin_rebuild`] when the build first CLEARS the keyspace.
    pub fn begin_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
        head: raisin_hlc::HLC,
    ) -> Result<u64> {
        self.begin(
            tenant_id, repo_id, branch, workspace, definition, head, false,
        )
    }

    /// [`Self::begin_build`] for a build that empties the keyspace first: the
    /// record goes to `Building` (unusable) whatever its phase was, keeping its
    /// generation, so the planner never trusts `Ready` over a cleared keyspace.
    pub fn begin_rebuild(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
        head: raisin_hlc::HLC,
    ) -> Result<u64> {
        self.begin(
            tenant_id, repo_id, branch, workspace, definition, head, true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn begin(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
        head: raisin_hlc::HLC,
        force_building: bool,
    ) -> Result<u64> {
        let _guard = transitions();
        let existing = read_state(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &definition.name,
        )?;
        let generation = existing.as_ref().map_or(0, |s| s.stale_generation);
        if existing.is_none() || force_building {
            let mut building = CompoundIndexState::ready(definition, head);
            building.phase = CompoundBuildPhase::Building;
            building.stale_generation = generation;
            self.put_unlocked(tenant_id, repo_id, branch, workspace, &building)?;
        }
        Ok(generation)
    }

    /// Stamp `ready` if no mark arrived since [`Self::begin_build`] returned
    /// `started_under`. Returns whether it was stamped; `false` leaves the
    /// record `NotBuilt` for the next build.
    pub fn complete_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        mut ready: CompoundIndexState,
        started_under: u64,
    ) -> Result<bool> {
        let _guard = transitions();
        let current = read_state(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &ready.index_name,
        )?
        .map_or(0, |state| state.stale_generation);
        if current != started_under {
            return Ok(false);
        }
        ready.stale_generation = current;
        self.put_unlocked(tenant_id, repo_id, branch, workspace, &ready)?;
        Ok(true)
    }
}
