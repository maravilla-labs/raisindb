//! Whether a node's PROPERTY_INDEX entry is live AS OF a revision — an MVCC
//! read of the entry's `(value, node)` group, not a point read.
//!
//! An entry written by a skip-unchanged writer lives at the revision of the
//! version that FIRST wrote it, and history GC may since have deleted that
//! NODES version while keeping the entry (it is the newest of its group).
//! A point read at the revisions of the node's retained versions then finds
//! nothing, so a verify built on point reads reported a hole after every GC
//! (`verify_after_history_gc_finds_no_hole`). The group's newest key at or
//! below the revision decides instead.

use super::entries::PropertyEntry;
use crate::indexing::IndexCtx;
use crate::repositories::nodes::helpers::is_tombstone;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ColumnFamily, ReadOptions, DB};

/// What [`entry_state_as_of`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryState {
    /// The group's newest key at or below the revision is live.
    Live,
    /// It is a tombstone.
    Tombstone,
    /// The group has no key at or below the revision.
    Absent,
    /// The walk read `max_keys` keys of other nodes first (a common value):
    /// no verdict.
    Inconclusive,
}

/// The state of `node_id`'s `entry` as of `at`.
///
/// Keys are `{value prefix}{~rev}\0{node}`: a seek to `{value prefix}{~at}`
/// lands on the newest entry at or below `at`, and the walk then passes other
/// nodes' entries of the same value (newest first) until it meets this
/// node's. Bounded by `max_keys`.
pub(crate) fn entry_state_as_of(
    db: &DB,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    entry: &PropertyEntry,
    node_id: &str,
    at: &HLC,
    max_keys: usize,
) -> Result<EntryState> {
    let prefix = entry.value_prefix(ctx);
    let mut start = prefix.clone();
    start.extend_from_slice(&at.encode_descending());
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(&prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    iter.seek(&start);
    let mut read = 0usize;
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let Some(rest) = key.strip_prefix(prefix.as_slice()) else {
            break;
        };
        if read >= max_keys {
            return Ok(EntryState::Inconclusive);
        }
        read += 1;
        // {~rev: 16}\0{node_id}
        if rest.len() > 17 && rest[16] == 0 && &rest[17..] == node_id.as_bytes() {
            return Ok(if is_tombstone(value) {
                EntryState::Tombstone
            } else {
                EntryState::Live
            });
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(EntryState::Absent)
}
