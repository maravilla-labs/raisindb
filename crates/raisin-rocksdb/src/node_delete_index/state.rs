//! Whether a branch's `NODE_DELETES` is complete — the readiness the reader
//! asks before it trusts the index over the `NODES` walk.
//!
//! Key `{tenant}\0{repo}\0{branch}\0ndel_state` in `INDEX_STATUS`:
//! tenant-first (tenant wipe and repository purge reach it), carrying the
//! branch, and NOT copied on fork (INDEX_STATUS is not), so a fork or a
//! re-created branch reads as not built. Node-id-free, like the localized
//! name index's records: a checkpoint ingest brings the PEER's record, so the
//! ingest marks every record `NotBuilt` before and after its copy.
//!
//! **Generations.** Every invalidation (ingest, a branch copy into the
//! branch — `super::branch_copy`) raises the generation and sets `NotBuilt`,
//! under [`transitions`].
//! A backfill records the generation it began under and may stamp `Ready`
//! only while the record still carries it — so a backfill that ran while a
//! copy was bringing in tombstones it never saw cannot vouch for them.

use crate::{cf, cf_handle, keys::KeyBuilder};
use raisin_error::{Error, Result};
use rocksdb::{WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, MutexGuard};

const RECORD: &str = "ndel_state";

/// `(tenant, repo, branch)`.
pub type BranchScope<'a> = (&'a str, &'a str, &'a str);

/// Where a branch's index is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexStatus {
    NotBuilt,
    Building,
    Ready,
}

/// The persisted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDeleteIndexState {
    pub status: IndexStatus,
    pub generation: u64,
    pub updated_at: String,
}

/// Serializes every read-modify-write of these records in this process.
pub(super) fn transitions() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `{tenant}\0{repo}\0{branch}\0ndel_state`.
pub fn state_key(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(RECORD)
        .build()
}

fn decode(bytes: &[u8]) -> Option<NodeDeleteIndexState> {
    serde_json::from_slice(bytes).ok()
}

/// The record through `src`'s view (an undecodable one reads as absent).
pub(crate) fn read_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    (tenant_id, repo_id, branch): BranchScope<'_>,
) -> Result<Option<NodeDeleteIndexState>> {
    let bytes = src.get(cf::INDEX_STATUS, &state_key(tenant_id, repo_id, branch))?;
    Ok(bytes.as_deref().and_then(decode))
}

/// Whether the branch's index is `Ready`, through `src`'s view — the SAME
/// view the entries are then read from.
pub(crate) fn ready_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    scope: BranchScope<'_>,
) -> Result<bool> {
    // A database opened without these column families (a minimal test
    // database) has no index to trust: it walks.
    let db = src.db();
    if db.cf_handle(cf::INDEX_STATUS).is_none() || db.cf_handle(cf::NODE_DELETES).is_none() {
        return Ok(false);
    }
    Ok(read_in(src, scope)?.is_some_and(|s| s.status == IndexStatus::Ready))
}

/// The branch's record on the live database.
pub fn read(db: &DB, scope: BranchScope<'_>) -> Result<Option<NodeDeleteIndexState>> {
    read_in(&mut crate::mvcc_read::DbRead(db), scope)
}

/// Whether reads on the branch use the index.
pub fn is_ready(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> bool {
    read(db, (tenant_id, repo_id, branch))
        .ok()
        .flatten()
        .is_some_and(|s| s.status == IndexStatus::Ready)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub(super) fn put(
    db: &DB,
    batch: &mut WriteBatch,
    key: &[u8],
    status: IndexStatus,
    generation: u64,
) -> Result<()> {
    let state = NodeDeleteIndexState {
        status,
        generation,
        updated_at: now(),
    };
    let bytes = serde_json::to_vec(&state)
        .map_err(|e| Error::storage(format!("node delete index state encode: {e}")))?;
    batch.put_cf(cf_handle(db, cf::INDEX_STATUS)?, key, bytes);
    Ok(())
}

pub(super) fn write(db: &DB, batch: WriteBatch) -> Result<()> {
    db.write(batch).map_err(|e| Error::storage(e.to_string()))
}

/// A backfill starts. With `resume` the generation of the interrupted run it
/// continues: kept while the record still says `Building` under it (no
/// invalidation since). Otherwise a fresh generation, above any the record
/// had. Returns `(generation, continued)`; a run that did not continue must
/// start from the first key.
pub fn begin_build(db: &DB, scope: BranchScope<'_>, resume: Option<u64>) -> Result<(u64, bool)> {
    let _guard = transitions();
    let current = read(db, scope)?;
    if let (Some(generation), Some(state)) = (resume, current.as_ref()) {
        if state.status == IndexStatus::Building && state.generation == generation {
            return Ok((generation, true));
        }
    }
    let generation = current.map_or(0, |s| s.generation) + 1;
    let mut batch = WriteBatch::default();
    put(
        db,
        &mut batch,
        &state_key(scope.0, scope.1, scope.2),
        IndexStatus::Building,
        generation,
    )?;
    write(db, batch)?;
    Ok((generation, false))
}

/// A backfill completed (its entries are committed): `Ready`, if the record
/// still says `Building` under `generation`. Returns whether it stamped.
pub fn finish_build(db: &DB, scope: BranchScope<'_>, generation: u64) -> Result<bool> {
    let _guard = transitions();
    let still = !super::branch_copy::in_progress(scope)
        && read(db, scope)?
            .is_some_and(|s| s.status == IndexStatus::Building && s.generation == generation);
    if still {
        let mut batch = WriteBatch::default();
        put(
            db,
            &mut batch,
            &state_key(scope.0, scope.1, scope.2),
            IndexStatus::Ready,
            generation,
        )?;
        write(db, batch)?;
    }
    Ok(still)
}

/// `NotBuilt` under a raised generation, if the branch has a record (no
/// record already reads as not built). Returns the new generation.
pub fn invalidate(db: &DB, scope: BranchScope<'_>) -> Result<Option<u64>> {
    let _guard = transitions();
    invalidate_locked(db, scope)
}

pub(super) fn invalidate_locked(db: &DB, scope: BranchScope<'_>) -> Result<Option<u64>> {
    let Some(state) = read(db, scope)? else {
        return Ok(None);
    };
    let generation = state.generation + 1;
    let mut batch = WriteBatch::default();
    put(
        db,
        &mut batch,
        &state_key(scope.0, scope.1, scope.2),
        IndexStatus::NotBuilt,
        generation,
    )?;
    write(db, batch)?;
    Ok(Some(generation))
}

/// Every record in the database `NotBuilt` (a checkpoint ingest: before the
/// copy, so no read trusts the index while peer tombstones arrive, and after
/// it, so neither a peer's record nor a backfill that ran during the copy
/// survives). Returns how many.
pub fn mark_all_not_built(db: &DB) -> Result<usize> {
    let _guard = transitions();
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let suffix = format!("\0{RECORD}");
    let mut batch = WriteBatch::default();
    let mut count = 0;
    let mut iter = db.raw_iterator_cf(cf);
    iter.seek_to_first();
    while iter.valid() {
        if let (Some(key), Some(value)) = (iter.key(), iter.value()) {
            // Exactly `{t}\0{r}\0{b}\0ndel_state`: three separators.
            let shaped =
                key.ends_with(suffix.as_bytes()) && key.iter().filter(|b| **b == 0).count() == 3;
            if let Some(state) = decode(value).filter(|_| shaped) {
                put(
                    db,
                    &mut batch,
                    key,
                    IndexStatus::NotBuilt,
                    state.generation + 1,
                )?;
                count += 1;
            }
        }
        iter.next();
    }
    iter.status().map_err(|e| Error::storage(e.to_string()))?;
    drop(iter);
    write(db, batch)?;
    Ok(count)
}

/// A branch created or deleted under this name starts with no record.
pub fn stage_forget_branch(
    db: &DB,
    batch: &mut WriteBatch,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<()> {
    batch.delete_cf(
        cf_handle(db, cf::INDEX_STATUS)?,
        state_key(tenant_id, repo_id, branch),
    );
    Ok(())
}

pub use super::branch_copy::{after_branch_copy, before_branch_copy, CopyTicket};
