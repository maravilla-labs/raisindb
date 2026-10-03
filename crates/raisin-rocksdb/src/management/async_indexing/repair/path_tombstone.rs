//! Rewrite merge apply's legacy `\0` PATH_INDEX tombstones as `T`.
//!
//! Merge apply wrote a single `\0` byte as its path tombstone. Every reader
//! now treats that as a tombstone (`keys::is_tombstone_value`), permanently —
//! a checkpoint from an older peer can bring the markers back — so this is
//! cleanup only: the same key, the canonical value. Idempotent; a second run
//! finds nothing.

use super::cursor::BoundedWriter;
use crate::{cf, cf_handle};
use raisin_error::Result;
use rocksdb::{ReadOptions, DB};

pub(super) const PASS_PATH: &str = "path";

/// Rewrite every one-byte `\0` value under `branch_prefix` in PATH_INDEX.
/// Returns `false` when the run must stop early.
pub(super) fn path_pass(
    db: &DB,
    branch_prefix: &[u8],
    writer: &mut BoundedWriter<'_>,
    scanned: &mut u64,
) -> Result<bool> {
    writer.begin_pass(PASS_PATH);
    let cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|h| hex::decode(h).ok());

    let cf = cf_handle(db, cf::PATH_INDEX)?;
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(branch_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    match cursor {
        Some(mut after) => {
            after.push(0);
            iter.seek(&after);
        }
        None => iter.seek(branch_prefix),
    }

    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        *scanned += 1;
        if value == b"\x00" {
            let key = key.to_vec();
            writer.put(cf::PATH_INDEX, &key, crate::keys::TOMBSTONE_VALUE)?;
            if !writer.checkpoint(PASS_PATH, &key)? {
                return Ok(false);
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(true)
}
