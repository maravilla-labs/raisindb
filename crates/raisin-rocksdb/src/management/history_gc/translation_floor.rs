//! What a history-GC run tells `translation_history` (plan Phase 11).
//!
//! For every branch where this run ACTUALLY deleted a `TRANSLATION_DATA` /
//! `BLOCK_TRANSLATIONS` version, record `min(cutoff, head)` as that branch's
//! GC floor, so a later `resync_translations` from this node tells replicas.
//!
//! - Per branch, and only where something was deleted: a branch GC did not
//!   touch still holds its full history, and must not inherit a floor because
//!   some other branch (or repository) lost versions.
//! - Capped at the branch HEAD: retention keeps the newest version at or below
//!   the cutoff, so a branch idle since before the cutoff still answers every
//!   read at or above its HEAD exactly. A floor at the cutoff itself made a
//!   replica refuse the HEAD reads of every quiet branch.

use super::Plans;
use raisin_error::Result;
use rocksdb::DB;
use std::collections::HashSet;

pub(super) fn record(db: &DB, plans: &Plans, scopes: &HashSet<Vec<u8>>) -> Result<()> {
    for scope in scopes {
        let Some(plan) = plans.branch.get(scope) else {
            continue;
        };
        let mut parts = scope.splitn(3, |b| *b == 0).map(std::str::from_utf8);
        let (Some(Ok(tenant)), Some(Ok(repo)), Some(Ok(branch))) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let floor = match plans.heads.get(scope) {
            Some(head) => plan.cutoff.min(*head),
            None => plan.cutoff,
        };
        crate::translation_history::raise_gc_cutoff(db, tenant, repo, branch, floor)?;
    }
    Ok(())
}
