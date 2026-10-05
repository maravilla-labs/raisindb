//! The scan half of `resync_translations`: bounded chunks of one CF, and
//! what a run reports.

use crate::cf_handle;
use raisin_error::{Error, Result};
use rocksdb::ReadOptions;
use serde::{Deserialize, Serialize};

/// Versions read per chunk (between commits of the cursor).
const CHUNK: usize = 256;

/// What one resync of one branch emitted (or, in a dry run, would emit).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslationResyncCounts {
    /// Versions found.
    pub versions: u64,
    /// Of those: properties overlays, `Hidden`, tombstones, block overlays.
    pub live: u64,
    pub hidden: u64,
    pub tombstones: u64,
    pub blocks: u64,
    /// Ops captured.
    pub emitted: u64,
    /// The floor every op carried (`None`: full history).
    pub history_complete_from: Option<String>,
}

/// Up to [`CHUNK`] `(key, value)` pairs (and about `max_bytes` of values) of
/// `cf_name` under `prefix`, strictly after `after`. Read synchronously (an
/// iterator never crosses an `.await`).
pub(super) fn read_chunk(
    db: &rocksdb::DB,
    cf_name: &str,
    prefix: &[u8],
    after: Option<&[u8]>,
    max_bytes: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let cf = cf_handle(db, cf_name)?;
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    match after {
        Some(after) => {
            let mut seek = after.to_vec();
            seek.push(0);
            iter.seek(&seek);
        }
        None => iter.seek(prefix),
    }
    let mut chunk = Vec::new();
    let mut bytes = 0usize;
    while iter.valid() && chunk.len() < CHUNK && (chunk.is_empty() || bytes < max_bytes) {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        if !key.starts_with(prefix) {
            break;
        }
        bytes += value.len();
        chunk.push((key.to_vec(), value.to_vec()));
        iter.next();
    }
    iter.status().map_err(|e| Error::storage(e.to_string()))?;
    Ok(chunk)
}
