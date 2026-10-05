//! The fail-closed build state of the localized name index, per
//! `(tenant, repo, branch, workspace)`.
//!
//! Key `{tenant}\0{repo}\0{branch}\0{ws}\0lname_state` in `INDEX_STATUS` —
//! tenant-first (tenant wipe and repository purge reach it) and carrying the
//! branch and the workspace: `INDEX_STATUS` is not copied on fork, so a fork or
//! a publish target has NO record, which reads as not built. A shorter key
//! would let a fork alias its source's `Ready`.
//!
//! A lookup uses the index only while the record is `Ready`, its fingerprint
//! equals the repository's current one (`config::NameConfig::fingerprint`) and
//! the read revision is at or above `built_from_rev`; otherwise it takes the
//! row-level fallback, which is always correct (`lookup`).
//!
//! **Generations.** Every invalidation (a config change, a checkpoint ingest,
//! the feature switched off) raises the record's generation and sets it
//! `NotBuilt`, under [`transitions`]. A build records the generation it began
//! under and is allowed to stamp `Ready` only while it is unchanged, so a
//! config change landing mid-build — even A -> B -> A, which the fingerprint
//! alone cannot see — never lets that build's result be served.

use crate::{cf, cf_handle, keys::KeyBuilder};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

pub use super::availability::{availability, Availability};

const RECORD: &str = "lname_state";

/// Where a workspace's build is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    NotBuilt,
    Building,
    Ready,
}

/// The persisted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedNameState {
    pub status: BuildStatus,
    /// `NameConfig::fingerprint` the build ran under.
    pub fingerprint: String,
    /// The revision the build read the branch at; reads below it fall back.
    pub built_from_rev: Option<HLC>,
    pub generation: u64,
    /// Sibling name collisions the build found (uniqueness enforcement waits
    /// for zero).
    #[serde(default)]
    pub collisions: u64,
    pub updated_at: String,
}

/// Serializes every read-modify-write of these records in this process.
pub(crate) fn transitions() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn state_key(tenant_id: &str, repo_id: &str, branch: &str, workspace: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(RECORD)
        .build()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The record, if any (an undecodable one reads as absent: not built).
pub fn read(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<Option<LocalizedNameState>> {
    read_in(
        &mut crate::mvcc_read::DbRead(db),
        tenant_id,
        repo_id,
        branch,
        workspace,
    )
}

/// [`read`] through `src`'s view. A lookup reads the record through the
/// SAME snapshot as the claims it then trusts: a build commits its last
/// claims and only then stamps `Ready`, so a `Ready` read live paired with
/// claims read from an earlier snapshot is a false miss.
pub(crate) fn read_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<Option<LocalizedNameState>> {
    let bytes = src.get(
        cf::INDEX_STATUS,
        &state_key(tenant_id, repo_id, branch, workspace),
    )?;
    Ok(bytes.and_then(|b| serde_json::from_slice(&b).ok()))
}

fn stage(db: &DB, batch: &mut WriteBatch, key: &[u8], state: &LocalizedNameState) -> Result<()> {
    let bytes = serde_json::to_vec(state)
        .map_err(|e| Error::storage(format!("localized name state encode: {e}")))?;
    batch.put_cf(cf_handle(db, cf::INDEX_STATUS)?, key, bytes);
    Ok(())
}

/// Every record under `prefix` of `INDEX_STATUS`, by key.
fn records_under(db: &DB, prefix: &[u8]) -> Result<Vec<(Vec<u8>, LocalizedNameState)>> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let suffix = format!("\0{RECORD}");
    let mut out = Vec::new();
    for item in crate::prefix_scan(db, cf, prefix) {
        let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
        if !key.ends_with(suffix.as_bytes()) {
            continue;
        }
        if let Ok(state) = serde_json::from_slice(&value) {
            out.push((key.to_vec(), state));
        }
    }
    Ok(out)
}

/// Stage `NotBuilt` (generation + 1) for every record of the repository —
/// in the caller's batch, which carries the config change itself. The
/// caller holds [`transitions`] until the batch is written.
pub fn stage_not_built_for_repo(
    db: &DB,
    batch: &mut WriteBatch,
    tenant_id: &str,
    repo_id: &str,
) -> Result<usize> {
    let prefix = KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .build_prefix();
    stage_not_built_under(db, batch, &prefix)
}

fn stage_not_built_under(db: &DB, batch: &mut WriteBatch, prefix: &[u8]) -> Result<usize> {
    let records = records_under(db, prefix)?;
    for (key, mut state) in records.iter().cloned() {
        state.status = BuildStatus::NotBuilt;
        state.generation += 1;
        state.updated_at = now();
        stage(db, batch, &key, &state)?;
    }
    Ok(records.len())
}

/// Every record in the database `NotBuilt` — after a checkpoint ingest (the
/// peer's records describe the peer's apply history, not this node's) and at
/// a start with the index switched off (writes made while it is off are not
/// indexed). Returns how many.
pub fn mark_all_not_built(db: &DB) -> Result<usize> {
    let _guard = transitions();
    let mut batch = WriteBatch::default();
    let count = stage_not_built_under(db, &mut batch, b"")?;
    db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
    Ok(count)
}

/// A build of `branch` starts: each workspace's record becomes `Building`
/// under `fingerprint` at `pin`, all under ONE fresh generation — above every
/// generation any record of the branch had, so no two runs ever share one.
/// Returns it; [`finish_build`] stamps only records still carrying it.
pub fn begin_build(
    db: &DB,
    scope: (&str, &str, &str),
    workspaces: &[String],
    fingerprint: &str,
    pin: &HLC,
) -> Result<u64> {
    let (tenant_id, repo_id, branch) = scope;
    let _guard = transitions();
    let mut generation = 0;
    for (_, state) in records_under(db, &branch_prefix(tenant_id, repo_id, branch))? {
        generation = generation.max(state.generation);
    }
    for workspace in workspaces {
        if let Some(state) = read(db, tenant_id, repo_id, branch, workspace)? {
            generation = generation.max(state.generation);
        }
    }
    let generation = generation + 1;
    let mut batch = WriteBatch::default();
    for workspace in workspaces {
        let state = LocalizedNameState {
            status: BuildStatus::Building,
            fingerprint: fingerprint.to_string(),
            built_from_rev: Some(*pin),
            generation,
            collisions: 0,
            updated_at: now(),
        };
        stage(
            db,
            &mut batch,
            &state_key(tenant_id, repo_id, branch, workspace),
            &state,
        )?;
    }
    db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
    Ok(generation)
}

fn branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .build_prefix()
}

/// A build of `branch` completed: each workspace still `Building` under the
/// same fingerprint, pin AND generation (the one [`begin_build`] returned)
/// becomes `Ready`. Returns the workspaces stamped.
pub fn finish_build(
    db: &DB,
    scope: (&str, &str, &str),
    fingerprint: &str,
    pin: &HLC,
    generation: u64,
    collisions: &BTreeMap<String, u64>,
) -> Result<Vec<String>> {
    let (tenant_id, repo_id, branch) = scope;
    let _guard = transitions();
    let prefix = branch_prefix(tenant_id, repo_id, branch);
    let mut batch = WriteBatch::default();
    let mut stamped = Vec::new();
    for (key, mut state) in records_under(db, &prefix)? {
        let ws_start = prefix.len();
        let ws_end = key.len() - RECORD.len() - 1;
        if ws_end <= ws_start {
            continue;
        }
        let workspace = String::from_utf8_lossy(&key[ws_start..ws_end]).into_owned();
        if workspace.contains('\0')
            || state.status != BuildStatus::Building
            || state.fingerprint != fingerprint
            || state.built_from_rev.as_ref() != Some(pin)
            || state.generation != generation
        {
            continue;
        }
        state.status = BuildStatus::Ready;
        state.collisions = collisions.get(&workspace).copied().unwrap_or(0);
        state.updated_at = now();
        stage(db, &mut batch, &key, &state)?;
        stamped.push(workspace);
    }
    db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
    Ok(stamped)
}

/// One workspace's record `NotBuilt` (generation + 1), when it has one — a
/// writer that could not index a node (its parent unresolvable) must not
/// leave a `Ready` index claiming completeness. A build running meanwhile
/// cannot stamp `Ready` over it (the generation moved).
pub fn invalidate_workspace(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<bool> {
    let _guard = transitions();
    let Some(mut state) = read(db, tenant_id, repo_id, branch, workspace)? else {
        return Ok(false);
    };
    state.status = BuildStatus::NotBuilt;
    state.generation += 1;
    state.updated_at = now();
    let mut batch = WriteBatch::default();
    stage(
        db,
        &mut batch,
        &state_key(tenant_id, repo_id, branch, workspace),
        &state,
    )?;
    db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
    Ok(true)
}

/// Stage `NotBuilt` (generation + 1) for every record of `branch` into
/// `batch` (a merge from a source whose index is not complete). The caller
/// holds [`transitions`] until the batch is written.
pub fn stage_not_built_for_branch(
    db: &DB,
    batch: &mut WriteBatch,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<usize> {
    stage_not_built_under(db, batch, &branch_prefix(tenant_id, repo_id, branch))
}

/// Stage the deletion of every record of `branch` (branch delete, and create
/// under a deleted name: a re-created branch must not inherit `Ready`).
pub fn stage_forget_branch(
    db: &DB,
    batch: &mut WriteBatch,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<usize> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let records = records_under(db, &branch_prefix(tenant_id, repo_id, branch))?;
    for (key, _) in &records {
        batch.delete_cf(cf, key);
    }
    Ok(records.len())
}
