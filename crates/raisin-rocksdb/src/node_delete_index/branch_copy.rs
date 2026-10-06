//! A branch copy into a branch (`copy_branch_indexes`: a fork, a merge
//! replaying its source) and that branch's readiness.

use super::state::{
    invalidate_locked, put, read, state_key, transitions, write, BranchScope, IndexStatus,
};
use raisin_error::Result;
use rocksdb::{WriteBatch, DB};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

type CopyKey = (String, String, String);

/// Branches a `copy_branch_indexes` is writing into right now, with how many
/// copies. No backfill may stamp such a branch `Ready`: NODES lands before
/// this CF, so mid-copy the target holds tombstones whose entries are still
/// to come — a backfill that scanned NODES before they arrived would vouch
/// for them. (A generation alone does not stop a backfill that BEGINS during
/// the copy; this does.)
fn copying() -> MutexGuard<'static, HashMap<CopyKey, usize>> {
    static COPYING: OnceLock<Mutex<HashMap<CopyKey, usize>>> = OnceLock::new();
    COPYING
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn copy_key((t, r, b): BranchScope<'_>) -> CopyKey {
    (t.to_string(), r.to_string(), b.to_string())
}

/// Whether a copy is writing into the branch right now (no backfill may
/// stamp it `Ready` meanwhile).
pub(super) fn in_progress(scope: BranchScope<'_>) -> bool {
    copying().contains_key(&copy_key(scope))
}

/// What [`before_branch_copy`] decided: re-stamp the target after the copy.
#[derive(Debug, Clone, Copy)]
pub struct CopyTicket {
    target_generation: u64,
    source_generation: u64,
}

/// `copy_branch_indexes` is about to put `source`'s entries (NODES first,
/// this CF after) into `target`. The target stops trusting its index for
/// the copy. When BOTH were `Ready`, the copy keeps the target complete — its
/// own tombstones were indexed, and every source tombstone it copies had its
/// entry before NODES was copied — so the ticket lets [`after_branch_copy`]
/// re-stamp it, provided neither record changed meanwhile.
pub fn before_branch_copy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    source: &str,
    target: &str,
) -> Result<Option<CopyTicket>> {
    let _guard = transitions();
    let target_state = read(db, (tenant_id, repo_id, target))?;
    let source_state = read(db, (tenant_id, repo_id, source))?;
    let invalidated = invalidate_locked(db, (tenant_id, repo_id, target))?;
    // Only once nothing above can fail: `after_branch_copy` undoes it.
    *copying()
        .entry(copy_key((tenant_id, repo_id, target)))
        .or_default() += 1;
    let Some(target_generation) = invalidated else {
        return Ok(None);
    };
    let both_ready = target_state.is_some_and(|s| s.status == IndexStatus::Ready)
        && source_state
            .as_ref()
            .is_some_and(|s| s.status == IndexStatus::Ready);
    Ok(both_ready.then(|| CopyTicket {
        target_generation,
        source_generation: source_state.map_or(0, |s| s.generation),
    }))
}

/// The copy is done (or failed: pass no ticket). Re-stamp the target `Ready`
/// when the ticket still holds, otherwise invalidate it again (a backfill
/// that began during the copy cannot vouch for the tombstones it brought).
/// Every [`before_branch_copy`] must be paired with exactly one call. Returns
/// whether the target is `Ready`; when not, the caller requests its backfill.
pub fn after_branch_copy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    source: &str,
    target: &str,
    ticket: Option<CopyTicket>,
) -> Result<bool> {
    let _guard = transitions();
    {
        let mut copies = copying();
        let key = copy_key((tenant_id, repo_id, target));
        if let Some(n) = copies.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                copies.remove(&key);
            }
        }
    }
    if let Some(ticket) = ticket {
        let source_held = read(db, (tenant_id, repo_id, source))?.is_some_and(|s| {
            s.status == IndexStatus::Ready && s.generation == ticket.source_generation
        });
        let target_held = read(db, (tenant_id, repo_id, target))?.is_some_and(|s| {
            s.status == IndexStatus::NotBuilt && s.generation == ticket.target_generation
        });
        if source_held && target_held {
            let mut batch = WriteBatch::default();
            put(
                db,
                &mut batch,
                &state_key(tenant_id, repo_id, target),
                IndexStatus::Ready,
                ticket.target_generation,
            )?;
            write(db, batch)?;
            return Ok(true);
        }
    }
    invalidate_locked(db, (tenant_id, repo_id, target))?;
    Ok(false)
}
