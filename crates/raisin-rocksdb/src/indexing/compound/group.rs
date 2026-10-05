//! Where a compound tombstone lands: `max(R, newest entry of its (group, node)
//! above R)` (plan Phase 7 item 8, applied to COMPOUND by Phase 8 step 2).
//!
//! A tombstone at R masks nothing above R. Entries above a write's revision
//! for a group the write ends exist when something wrote at a revision that is
//! not a NODES version of the node: a pre-Phase-8 rebuild stamped every entry
//! with the branch HEAD, and a `versionable=false` write reuses an older
//! revision. Landing the tombstone on that newest entry is what retires it.
//!
//! Never used when NODES successors exist (an out-of-order write): an entry
//! above R is then a successor's own, and the writer re-asserts successors
//! instead (`writer.rs`).
//!
//! The walk is cheap in the normal case: keys within a group sort newest
//! first, so the seek lands on the group's newest entry and stops at the first
//! one at or below R — for a fresh revision, immediately. Other nodes sharing
//! the tuple above R are walked past, capped at [`MAX_KEYS_PER_GROUP`].

use super::entries::parse_entry_key;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ColumnFamily, DB};
use std::sync::atomic::{AtomicU64, Ordering};

/// Keys walked above R per group before giving up and writing at R.
pub const MAX_KEYS_PER_GROUP: usize = 256;

static CAPPED: AtomicU64 = AtomicU64::new(0);

/// Group walks that hit [`MAX_KEYS_PER_GROUP`] since process start (the
/// tombstone then lands at R).
pub fn compound_group_walks_capped() -> u64 {
    CAPPED.load(Ordering::Relaxed)
}

/// The revision a tombstone of `node_id`'s entry in `group` lands at.
pub fn tombstone_revision(
    db: &DB,
    cf: &ColumnFamily,
    group: &[u8],
    node_id: &str,
    revision: &HLC,
) -> Result<HLC> {
    let mut opts = rocksdb::ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(group) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    iter.seek(group);
    let mut walked = 0usize;
    while iter.valid() {
        let Some(key) = iter.key() else { break };
        if !key.starts_with(group) {
            break;
        }
        let Some((key_group, at, owner)) = parse_entry_key(key) else {
            iter.next();
            continue;
        };
        if key_group != group {
            // A longer tuple sharing this prefix: not this group.
            iter.next();
            continue;
        }
        if at <= *revision {
            break;
        }
        if owner == node_id {
            return Ok(at);
        }
        walked += 1;
        if walked >= MAX_KEYS_PER_GROUP {
            CAPPED.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(
                node_id,
                "compound group walk capped; tombstone lands at the write revision"
            );
            break;
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(*revision)
}
