//! Revision-bounded point reads over `{prefix}{~revision}` keys.
//!
//! Every versioned CF that stores one record per revision under a fixed prefix
//! (`NODES`, `NODE_PATH`, `PATH_INDEX`) answers "the newest version at or
//! before revision R" the same way. Revisions are encoded descending, so the
//! keys under a prefix run newest first and that version is the first key at or
//! after `{prefix}{encode_descending(R)}`. One seek finds it.
//!
//! The readers this replaces walked the prefix from the newest key and skipped
//! every version above R — O(newer revisions) per time-travel read — and then
//! re-read the value they had just stepped over with a second `get_cf`.
//!
//! # Equivalence with the walk
//!
//! For a well-formed key (`prefix` + 16 HLC bytes), `key < seek` exactly when
//! its revision is newer than R, so every key the seek skips is one the walk
//! would have skipped as well. Two details keep the equivalence exact:
//!
//! - A key whose revision does not parse is SKIPPED and the scan continues — it
//!   never ends the read. That is what the walks did, and a seek that lands on
//!   such a key must advance past it rather than report "not found".
//! - The revision is always the fixed 16-byte key trailer. The descending HLC
//!   can contain `0x00` (and `0xFF`) bytes, so nothing here splits a key on the
//!   separator.

use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{AsColumnFamilyRef, ReadOptions, DB};

mod baseline;
mod deletes;
mod node_decode;
mod node_path;
#[cfg(test)]
mod path_rule_tests;
mod point;
mod source;
#[cfg(test)]
mod tests;

pub(crate) use baseline::{
    node_version_at_or_before, node_version_before, node_versions_above, predecessor, StoredVersion,
};
#[cfg(test)]
pub(crate) use deletes::WALKS;
pub(crate) use deletes::{deletes_in_range, deletes_in_range_counted, NodeLifeline};
pub use node_decode::decode_node_blob;
pub(crate) use node_decode::{
    decode_entry_with_path, deserialize_node_with_path, deserialize_node_with_path_as,
    deserialize_node_with_path_in, embedded_path_of,
};
pub(crate) use node_path::{
    current_path, embedded_path_may_win, embedded_path_wins, materialize_path, materialize_path_in,
    CurrentPath, EmbeddedPath, NodeScope,
};
pub(crate) use point::{node_record_revision_in, node_version_in, path_index_entry_in, PathEntry};
pub(crate) use source::{DbRead, Recorded, SnapshotRead, VersionedRead};

/// The bound for "newest, whatever its revision": a read with no
/// `max_revision` materializes paths as of the newest NODE_PATH entry.
pub(crate) const NEWEST: HLC = HLC {
    timestamp_ms: u64::MAX,
    counter: u64::MAX,
};

/// The newest entry under `prefix` whose revision is `<= max_revision` — or
/// the newest entry at all when `max_revision` is `None` — together with its
/// value, taken from the iterator that found it.
///
/// Tombstones are returned like any other value: whether a tombstone means
/// "absent" or "error" is the caller's decision.
pub(crate) fn newest_at_or_before(
    db: &DB,
    cf: &impl AsColumnFamilyRef,
    prefix: &[u8],
    max_revision: Option<&HLC>,
) -> Result<Option<(HLC, Vec<u8>)>> {
    newest_at_or_before_with(db, cf, prefix, max_revision, |revision, value| {
        (revision, value.to_vec())
    })
}

/// [`newest_at_or_before`], handing the value to `read` in place instead of
/// copying it out — for callers that only look at it (a tombstone check on a
/// node blob should not copy the blob).
pub(crate) fn newest_at_or_before_with<R>(
    db: &DB,
    cf: &impl AsColumnFamilyRef,
    prefix: &[u8],
    max_revision: Option<&HLC>,
    read: impl FnOnce(HLC, &[u8]) -> R,
) -> Result<Option<R>> {
    // Bounded exactly like `crate::prefix_scan`: the iterator cannot leave the
    // prefix whatever the CF's prefix extractor says.
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);

    match max_revision {
        Some(max) => {
            let mut seek = Vec::with_capacity(prefix.len() + 16);
            seek.extend_from_slice(prefix);
            seek.extend_from_slice(&max.encode_descending());
            iter.seek(&seek);
        }
        None => iter.seek(prefix),
    }

    while iter.valid() {
        let Some(key) = iter.key() else {
            break;
        };
        if !key.starts_with(prefix) {
            break;
        }

        match keys::extract_revision_from_key(key) {
            Ok(revision) => {
                if max_revision.is_none_or(|max| &revision <= max) {
                    return Ok(Some(read(revision, iter.value().unwrap_or_default())));
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: "rocksb::nodes::revision_lookup",
                    key_len = key.len(),
                    "Skipping versioned key with invalid revision: {}",
                    e
                );
            }
        }
        iter.next();
    }

    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(None)
}

/// Whether ANY entry under `prefix` at or before `max_revision` satisfies
/// `matches` — walked newest first, stopping at the first match.
///
/// For a question about an entry's existence rather than about the newest
/// one ("has this node ever held this UNIQUE claim?"): the answer must not
/// depend on which OTHER entries a node happens to have applied yet.
pub(crate) fn any_at_or_before(
    db: &DB,
    cf: &impl AsColumnFamilyRef,
    prefix: &[u8],
    max_revision: &HLC,
    mut matches: impl FnMut(HLC, &[u8]) -> bool,
) -> Result<bool> {
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf, opts);
    let mut seek = Vec::with_capacity(prefix.len() + 16);
    seek.extend_from_slice(prefix);
    seek.extend_from_slice(&max_revision.encode_descending());
    iter.seek(&seek);
    while iter.valid() {
        let Some(key) = iter.key() else {
            break;
        };
        if !key.starts_with(prefix) {
            break;
        }
        if let Ok(revision) = keys::extract_revision_from_key(key) {
            if &revision <= max_revision && matches(revision, iter.value().unwrap_or_default()) {
                return Ok(true);
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(false)
}
