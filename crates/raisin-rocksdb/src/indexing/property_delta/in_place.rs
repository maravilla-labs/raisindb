//! Where an IN-PLACE write (`versionable=false`: the revision R is reused)
//! must land each PROPERTY_INDEX write.
//!
//! A tombstone at R masks only entries at or below R. An entry of the same
//! group ABOVE R — written by a legacy rebuild at the branch HEAD, or carried
//! from a second revision stream — is not masked by it, and the stale value
//! then matches at HEAD forever. So an in-place writer may write at
//! `max(R, newest entry revision of that (entry, node) group)` (plan Phase 7
//! item 8). Writing AT that newest revision overwrites the very key, which is
//! exactly the "in place" the caller asked for; the node has no history to
//! preserve there.
//!
//! **The lookup is opt-in, bounded and done before the batch lock.** Finding
//! the newest group entry above R means walking the value's prefix across
//! ALL nodes (node ids sort after the revision), and for common values
//! (`__node_type`, `__updated_by = 'system'`, an IS_A member) that is every
//! entry written with that value since the in-place node was created — on
//! every refresh of a health-check node. So:
//! - it runs only when `index.skip_unchanged` is on (the caller resolves
//!   [`InPlaceTargets`] or passes none); with the flag off an in-place write
//!   lands at R, exactly as before Phase 7;
//! - each group's walk stops after [`MAX_KEYS_PER_GROUP`] keys and then lands
//!   at R (the pre-Phase-7 answer), counted in [`in_place_scans_capped`];
//! - the caller resolves the targets BEFORE taking a transaction's batch
//!   mutex, so no read happens under it.

use super::entries::{entries_of, PropertyEntry};
use crate::indexing::IndexCtx;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, ReadOptions, DB};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Keys one group's walk may read before giving up (and landing at R).
pub const MAX_KEYS_PER_GROUP: usize = 256;

static CAPPED: AtomicU64 = AtomicU64::new(0);

/// In-place group walks that hit [`MAX_KEYS_PER_GROUP`] since process start.
pub fn in_place_scans_capped() -> u64 {
    CAPPED.load(Ordering::Relaxed)
}

/// How a write at a reused revision places its entries.
#[derive(Debug, Clone, Copy, Default)]
pub enum InPlace<'a> {
    /// A fresh revision: every write lands at it.
    #[default]
    No,
    /// A reused revision R. With targets, an entry whose group already has an
    /// entry of this node above R lands there; without, everything lands at R.
    Reused(Option<&'a InPlaceTargets>),
}

impl InPlace<'_> {
    pub fn is_in_place(&self) -> bool {
        matches!(self, Self::Reused(_))
    }
}

/// The newest revision above R of each entry group the in-place write
/// touches, resolved up front by [`InPlaceTargets::resolve`].
#[derive(Debug, Clone, Default)]
pub struct InPlaceTargets(HashMap<PropertyEntry, HLC>);

impl InPlaceTargets {
    /// Resolve, for every entry `old` or `new` indexes, the newest revision
    /// above `revision` at which `node_id` holds an entry of its group.
    pub fn resolve(
        db: &DB,
        ctx: &IndexCtx<'_>,
        old: Option<&Node>,
        new: &Node,
        revision: &HLC,
    ) -> Result<Self> {
        let cf = crate::cf_handle(db, crate::cf::PROPERTY_INDEX)?;
        let mut entries = entries_of(new);
        if let Some(old) = old {
            entries.extend(entries_of(old));
        }
        let mut out = HashMap::new();
        for entry in entries {
            if let Some(at) = newest_group_revision_above(db, cf, ctx, &entry, &new.id, revision)? {
                out.insert(entry, at);
            }
        }
        Ok(Self(out))
    }

    pub(super) fn get(&self, entry: &PropertyEntry) -> Option<HLC> {
        self.0.get(entry).copied()
    }
}

/// The newest revision above `above` at which `node_id` holds an entry (live
/// or tombstone) of `entry`'s value, if any — `None` too when the walk hit
/// [`MAX_KEYS_PER_GROUP`].
///
/// The key is `{value prefix}{~rev}\0{node}`: revisions run newest first and
/// node ids interleave under each, so the walk covers the value's entries
/// newer than `above` and stops at the first revision at or below it.
fn newest_group_revision_above(
    db: &DB,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    entry: &PropertyEntry,
    node_id: &str,
    above: &HLC,
) -> Result<Option<HLC>> {
    let prefix = entry.value_prefix(ctx);
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(&prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    iter.seek(&prefix);
    let mut read = 0usize;
    while iter.valid() {
        let Some(key) = iter.key() else { break };
        let Some(rest) = key.strip_prefix(prefix.as_slice()) else {
            break;
        };
        read += 1;
        if read > MAX_KEYS_PER_GROUP {
            CAPPED.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(
                node_id,
                name = %entry.name,
                "in-place group walk capped; writing at the reused revision"
            );
            return Ok(None);
        }
        // {~rev: 16}\0{node_id}
        if rest.len() > 17 && rest[16] == 0 {
            if let Ok(revision) = crate::keys::decode_descending_revision(&rest[..16]) {
                if revision <= *above {
                    break;
                }
                if &rest[17..] == node_id.as_bytes() {
                    return Ok(Some(revision));
                }
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(None)
}
