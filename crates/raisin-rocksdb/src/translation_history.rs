//! `translation_history_complete_from`: how far back this node's translation
//! history of ONE BRANCH can be trusted (plan Phase 11 item 5).
//!
//! Translations did not replicate before Phase 11, so a replica converges by
//! the admin `resync_translations` job, which re-emits every stored version at
//! its ORIGINAL revision. Where the sender itself no longer holds full history
//! of a branch (history GC deleted versions there), the replica cannot either:
//! below that revision a version it serves may be one that never existed
//! there. The sender ships the branch's floor on every resync op of that
//! branch; the replica records the highest it has received, and a
//! locale-scoped read of that branch bounded BELOW it fails loudly instead of
//! answering ([`ensure_complete_at`]).
//!
//! # Why per branch, and why capped at the branch HEAD
//!
//! A retention cutoff is time-based (`now - keep_days`) and can sit ABOVE an
//! idle branch's HEAD. Retention keeps the newest version at or below the
//! cutoff, so every read at or above `min(cutoff, head)` is exact — HEAD reads
//! of an idle branch included. A per-REPOSITORY floor equal to the cutoff
//! failed every HEAD read of a quiet `publish` branch on every replica after a
//! resync, translated node or not. So history GC records, per branch where it
//! ACTUALLY deleted a translation version, `min(cutoff, head at GC time)`
//! ([`raise_gc_cutoff`]).
//!
//! # A received floor never bounces back to its origin
//!
//! The resync fans out: a replica that received floor F re-emits it to every
//! peer, the origin included. A received floor at or below this node's OWN
//! history-GC floor for the branch says nothing new — this node already knows
//! its history there is the GC's cutoff state, and history GC's contract is
//! that such reads see that state — so it is ignored ([`accept_received_floor`]).
//! Otherwise the origin that ran GC would start refusing its own reads.
//!
//! Records live in `INDEX_STATUS`, tenant-first (tenant wipe and repository
//! purge cover them):
//! - `{tenant}\0{repo}\0translation_history\0{branch}\0complete_from` — the
//!   floor this node's READS honour (set only by applying a resync op);
//! - `{tenant}\0{repo}\0translation_history\0{branch}\0gc_cutoff` — the floor
//!   this node's own history GC produced. Not a read floor here, but exactly
//!   what this node SENDS ([`sender_floor`]).
//!
//! The read floor is consulted on every bounded locale-scoped read, so it is
//! cached per `(database, tenant, repo, branch)`; the writers here update the
//! cache and the checkpoint-ingest hook (`derived_cache_registry`) drops it,
//! which is how a peer's records arriving in a checkpoint become visible.

use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

const COMPLETE_FROM: &str = "complete_from";
const GC_CUTOFF: &str = "gc_cutoff";

fn record_key(tenant_id: &str, repo_id: &str, branch: &str, record: &str) -> Vec<u8> {
    crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("translation_history")
        .push(branch)
        .push(record)
        .build()
}

/// `(database path, tenant, repo, branch)`.
type CacheKey = (String, String, String, String);

fn cache() -> &'static RwLock<HashMap<CacheKey, Option<HLC>>> {
    static CACHE: OnceLock<RwLock<HashMap<CacheKey, Option<HLC>>>> = OnceLock::new();
    CACHE.get_or_init(|| {
        raisin_core::register_database_invalidator(|database| {
            if let Ok(mut map) = cache().write() {
                match database {
                    Some(path) => map.retain(|key, _| key.0 != path),
                    None => map.clear(),
                }
            }
        });
        RwLock::new(HashMap::new())
    })
}

fn cache_key(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> CacheKey {
    (
        db.path().to_string_lossy().into_owned(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    )
}

fn read_record(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    record: &str,
) -> Result<Option<HLC>> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    match db
        .get_cf(cf, record_key(tenant_id, repo_id, branch, record))
        .map_err(|e| Error::storage(e.to_string()))?
    {
        Some(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            text.parse::<HLC>()
                .map(Some)
                .map_err(|e| Error::storage(format!("translation history record: {e}")))
        }
        None => Ok(None),
    }
}

/// Raise `record` to at least `revision`; returns the stored value.
fn raise_record(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    record: &str,
    revision: HLC,
) -> Result<HLC> {
    // Read-modify-write of a monotonic max: serialize it per process (two
    // appliers raising concurrently must not let the lower one win).
    static RAISE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = RAISE.lock().unwrap_or_else(|e| e.into_inner());
    let current = read_record(db, tenant_id, repo_id, branch, record)?;
    if let Some(current) = current.filter(|c| *c >= revision) {
        return Ok(current);
    }
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    db.put_cf(
        cf,
        record_key(tenant_id, repo_id, branch, record),
        revision.to_string().as_bytes(),
    )
    .map_err(|e| Error::storage(e.to_string()))?;
    if record == COMPLETE_FROM {
        // Under the RAISE lock, so a concurrent raise cannot interleave; a
        // reader's racing `or_insert` can never replace this value.
        if let Ok(mut map) = cache().write() {
            map.insert(cache_key(db, tenant_id, repo_id, branch), Some(revision));
        }
    }
    Ok(revision)
}

/// This node's read floor for the branch's translation history.
pub fn complete_from(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Option<HLC>> {
    let key = cache_key(db, tenant_id, repo_id, branch);
    if let Some(cached) = cache().read().ok().and_then(|map| map.get(&key).copied()) {
        return Ok(cached);
    }
    let value = read_record(db, tenant_id, repo_id, branch, COMPLETE_FROM)?;
    // `or_insert`, never `insert`: a raise that landed between the read above
    // and this line has already cached a HIGHER floor, and overwriting it with
    // the stale `None` would serve below-floor reads until the next restart.
    Ok(cache()
        .write()
        .map(|mut map| *map.entry(key).or_insert(value))
        .unwrap_or(value))
}

/// Record that this node's translation history of the branch is complete only
/// from `revision`. Monotonic. Prefer [`accept_received_floor`] for a floor
/// that arrived from a peer.
pub fn raise_complete_from(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    revision: HLC,
) -> Result<()> {
    raise_record(db, tenant_id, repo_id, branch, COMPLETE_FROM, revision).map(|_| ())
}

/// A resync op of `branch` carried the sender's floor. Raises this node's read
/// floor to it — unless this node's own history GC already accounts for it
/// (`floor <= gc_cutoff`), which is the floor coming back to the node that
/// produced it. Returns whether the floor was recorded.
pub fn accept_received_floor(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    floor: HLC,
) -> Result<bool> {
    if gc_cutoff(db, tenant_id, repo_id, branch)?.is_some_and(|own| floor <= own) {
        return Ok(false);
    }
    raise_complete_from(db, tenant_id, repo_id, branch, floor)?;
    Ok(true)
}

/// History GC deleted translation versions of `branch` below `floor` here
/// (`min(cutoff, head)`, see the module docs).
pub fn raise_gc_cutoff(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    floor: HLC,
) -> Result<()> {
    raise_record(db, tenant_id, repo_id, branch, GC_CUTOFF, floor).map(|_| ())
}

/// The floor this node's own history GC produced for the branch.
pub fn gc_cutoff(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Option<HLC>> {
    read_record(db, tenant_id, repo_id, branch, GC_CUTOFF)
}

/// The floor this node SENDS with a resync of `branch`: the floor its OWN
/// history GC produced there, and nothing it received. A resync is broadcast,
/// so every peer gets the GC'ing node's floor from that node directly; folding
/// a received floor in here would only send it back to where it came from
/// (and on to every node again), each hop raising someone's read floor. `None`:
/// this node never deleted translation history of the branch.
pub fn sender_floor(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Option<HLC>> {
    gc_cutoff(db, tenant_id, repo_id, branch)
}

/// Fail loudly when a locale-scoped read of `branch` bounded at `bound`
/// reaches below this node's floor for it: the version it would serve may
/// never have existed. Reads at or above the floor — HEAD reads of an idle
/// branch included, since the floor is capped at its HEAD — answer.
pub fn ensure_complete_at(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    bound: &HLC,
) -> Result<()> {
    match complete_from(db, tenant_id, repo_id, branch)? {
        Some(floor) if bound < &floor => Err(Error::InvalidState(format!(
            "translation history of {tenant_id}/{repo_id} branch '{branch}' on this node is \
             complete only from revision {floor}; a locale-scoped read at {bound} cannot be \
             answered (translation_history_complete_from)"
        ))),
        _ => Ok(()),
    }
}
