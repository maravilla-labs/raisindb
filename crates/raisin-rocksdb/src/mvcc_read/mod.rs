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

mod node_decode;
#[cfg(test)]
mod tests;

pub(crate) use node_decode::deserialize_node_with_path;

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

/// Materialize a node's path from the `NODE_PATH` index at `target_revision`.
///
/// The ONE implementation: the repository read path, the transaction read path
/// and the index rebuilds all resolve a `StorageNode` blob's path through here.
/// Which source wins — a blob's embedded path or this index — is decided by the
/// callers (`deserialize_node_with_path*`), not here.
///
/// # Errors
/// - the newest entry at or before `target_revision` is a tombstone (the node
///   was deleted);
/// - there is no entry at or before `target_revision`;
/// - the stored path is not UTF-8.
pub(crate) fn materialize_path(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    target_revision: &HLC,
) -> Result<String> {
    let prefix = keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let cf = crate::cf_handle(db, crate::cf::NODE_PATH)?;

    match newest_at_or_before(db, cf, &prefix, Some(target_revision))? {
        Some((_, value)) if crate::repositories::nodes::helpers::is_tombstone(&value) => {
            Err(raisin_error::Error::storage(format!(
                "Node {} was deleted (tombstone in NODE_PATH)",
                node_id
            )))
        }
        Some((_, value)) => String::from_utf8(value)
            .map_err(|e| raisin_error::Error::storage(format!("Invalid path encoding: {}", e))),
        None => Err(raisin_error::Error::storage(format!(
            "Path not found for node_id={} at revision={}",
            node_id, target_revision
        ))),
    }
}
