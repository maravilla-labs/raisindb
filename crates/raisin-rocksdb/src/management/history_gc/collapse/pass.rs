//! One CF of one branch: stream, find redundant versions, delete them.
//!
//! The decisions are [`super::detector`]'s. This file owns the slices.
//!
//! A slice holds, from its first read through its commit:
//!
//! - the `(branch, CF)` collapse exclusion (`management::cf_exclusion`),
//!   against inserters (repairs, rebuilds, branch copies) and retention GC;
//! - the WRITE side of the branch's in-place guard
//!   (`repositories::nodes::crud::indexing::in_place_guard`). In-place writers
//!   — a `versionable=false` node rewritten at its own revision, and the
//!   delta writers landing at `max(R, newest)` — overwrite exactly the keys
//!   collapse reads, under the READ side. Without it a slice could delete a
//!   key that was rewritten to a different state after it was read.
//!
//! A slice ends on whichever comes first: the delete batch is full, it has
//! scanned `collapse_slice_keys` keys, or `collapse_slice_time` has passed. It
//! then commits its deletes WITH its cursor — even when it deleted nothing, so
//! a pass over clean data still persists progress — and releases both holds
//! before the throttle (which charges scanned bytes as well as written ones)
//! sleeps. So an inserter waits for at most one short slice.
//!
//! The detector's memory survives into the next slice only while no inserter
//! or retention GC ran in between (the exclusion's epoch); otherwise it is
//! dropped. What it remembers from an earlier slice is re-read with a point
//! get before it is deleted (an in-place write may have landed between
//! slices); a key that no longer holds the remembered state is kept
//! (`skipped_changed`).

use super::super::layout::{self, GcTarget};
use super::detector::{Detector, Doomed, EntryState};
use super::{CfCollapseCounts, Scope, BUSY_PATIENCE};
use crate::management::async_indexing::repair::{iterate_from, BoundedWriter, RepairOptions};
use crate::management::cf_exclusion;
use crate::{cf_handle, keys};
use raisin_error::{Error, Result};
use std::time::Instant;

/// How a pass ended.
pub(super) enum End {
    /// Reached the end of the CF.
    Done,
    /// An inserter held the `(branch, CF)` too long; the cursor is committed.
    Busy,
    /// The test crash hook fired after a commit.
    Stopped,
}

/// Where a slice's iterator starts.
enum From {
    /// The branch's first key.
    Beginning,
    /// Strictly after this key (the previous slice's last key).
    After(Vec<u8>),
    /// The first key of this key's chunk: the detector's memory of that chunk
    /// is gone (a resume, or an inserter ran), so it is re-learned from the
    /// survivors already walked. Re-walking them deletes nothing new — two
    /// adjacent survivors of a group always differ — so a resumed or
    /// interrupted run ends exactly where an uninterrupted one does.
    ChunkOf(Vec<u8>),
}

/// Whether a doomed key read by an earlier slice still holds what it held.
fn still_holds(db: &rocksdb::DB, cf_name: &str, doomed: &Doomed) -> Result<bool> {
    let cf = cf_handle(db, cf_name)?;
    let now = db
        .get_cf(cf, &doomed.key)
        .map_err(|e| Error::storage(e.to_string()))?;
    Ok(now.is_some_and(|v| EntryState::of(&v) == doomed.state))
}

/// Collapse `target.cf` on the scope's branch. `resume` is a persisted cursor
/// of this pass (the run restarts at that key's chunk).
pub(super) fn collapse_cf(
    db: &rocksdb::DB,
    scope: &Scope<'_>,
    target: &GcTarget,
    writer: &mut BoundedWriter<'_>,
    counts: &mut CfCollapseCounts,
    resume: Option<Vec<u8>>,
    options: &RepairOptions,
) -> Result<End> {
    let prefix = keys::branch_prefix(scope.tenant_id, scope.repo_id, scope.branch);
    let mut from = match resume {
        Some(key) => From::ChunkOf(key),
        None => From::Beginning,
    };
    let mut detector = Detector::new(target, scope.watermark);
    let mut epoch: Option<u64> = None;
    loop {
        // A dry run deletes nothing, so it needs no hold.
        let holds = if writer.dry_run() {
            None
        } else {
            let Some(guard) = cf_exclusion::collapse_within(
                db,
                scope.tenant_id,
                scope.repo_id,
                scope.branch,
                target.cf,
                BUSY_PATIENCE,
            ) else {
                return Ok(End::Busy);
            };
            if epoch.is_some_and(|e| e != guard.epoch()) {
                // An inserter or retention GC ran between slices: an entry
                // may now sit between (or be gone from) two versions the
                // detector remembers.
                if let From::After(key) = from {
                    from = From::ChunkOf(key);
                }
            }
            epoch = Some(guard.epoch());
            let in_place = crate::repositories::nodes::backfill_write_guard(
                scope.tenant_id,
                scope.repo_id,
                scope.branch,
            );
            Some((guard, in_place))
        };
        detector.begin_slice();

        let mut iter = iterate_from(db, target.cf, &prefix, None)?;
        match &from {
            From::Beginning => {}
            From::After(key) => {
                let mut after = key.clone();
                after.push(0);
                iter.seek(&after);
            }
            From::ChunkOf(key) => {
                detector.forget();
                match layout::locate(target, key) {
                    Some(loc) => iter.seek(loc.chunk(key)),
                    None => iter.seek(key),
                }
            }
        }
        let started = Instant::now();
        let mut scanned: u64 = 0;
        let mut more = false;
        while iter.valid() {
            let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
                break;
            };
            if !key.starts_with(&prefix) {
                break;
            }
            let key = key.to_vec();
            scanned += 1;
            writer.note_read((key.len() + value.len()) as u64);
            if let Some(doomed) = detector.visit(&key, value, counts)? {
                if doomed.stale && !still_holds(db, target.cf, &doomed)? {
                    counts.skipped_changed += 1;
                } else {
                    writer.delete(target.cf, &doomed.key)?;
                    counts.deleted += 1;
                    counts.bytes_reclaimable += doomed.size;
                }
            }
            writer.mark(target.cf, &key);
            iter.next();
            let slice_over = scanned >= options.collapse_slice_keys
                || (scanned % 256 == 0 && started.elapsed() >= options.collapse_slice_time);
            if writer.batch_full(0) || slice_over {
                from = From::After(key);
                more = true;
                break;
            }
        }
        if !more {
            iter.status().map_err(|e| Error::storage(e.to_string()))?;
        }
        drop(iter);
        // The slice's deletes and its cursor land in one batch, under both
        // holds; they are released before the throttle sleeps.
        writer.commit_prepared("running", move |_| Ok(holds))?;
        if !more {
            return Ok(End::Done);
        }
        if writer.stop_requested() {
            return Ok(End::Stopped);
        }
    }
}
