//! Keyset-paginated scans over the `ORDERED_CHILDREN` editorial order.
//!
//! `ORDERED_CHILDREN` is already a compound index on `(parent_id, order_label)`,
//! so paging through a parent's children in editorial order is a native RocksDB
//! seek — no extra index is needed. This module is the single scan
//! implementation; the unpaginated `get_ordered_child_ids` is a thin wrapper
//! that passes no cursor and no limit.
//!
//! # Why explicit iterate bounds instead of `prefix_iterator_cf`
//!
//! The CF is configured with a custom prefix extractor (see
//! [`crate::prefix_transform`]), which makes `prefix_iterator_cf` convenient for
//! forward scans but **unsound for reverse iteration** — a backwards seek can
//! land outside the bloom-filtered prefix and stop early. Setting
//! `iterate_lower_bound` / `iterate_upper_bound` on the read options bounds the
//! iterator explicitly and behaves correctly in both directions.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use super::parse_ordered_child_key;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::collections::HashSet;

/// Where a forward/reverse ordered-children scan begins.
///
/// Keyset pagination needs the exclusive form; resuming a depth-first traversal
/// additionally needs the inclusive form, to land back *on* the label recorded in
/// a cursor so the walk can descend into that child's subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::repositories::nodes) enum OrderedScanStart<'a> {
    /// From the first (or last, when descending) child.
    Beginning,
    /// Strictly after this label — the keyset cursor.
    After(&'a str),
    /// At this label if it exists, else the next one after it.
    AtOrAfter(&'a str),
}

impl<'a> OrderedScanStart<'a> {
    fn label(&self) -> Option<&'a str> {
        match self {
            Self::Beginning => None,
            Self::After(label) | Self::AtOrAfter(label) => Some(label),
        }
    }

    /// True if `candidate` is at or past this start position.
    fn admits(&self, candidate: &str, descending: bool) -> bool {
        match self {
            Self::Beginning => true,
            Self::After(label) => {
                if descending {
                    candidate < *label
                } else {
                    candidate > *label
                }
            }
            Self::AtOrAfter(label) => {
                if descending {
                    candidate <= *label
                } else {
                    candidate >= *label
                }
            }
        }
    }
}

/// One entry of a parent's editorial order.
#[derive(Debug, Clone)]
pub(in crate::repositories::nodes) struct OrderedChildEntry {
    pub child_id: String,
    /// Full order label, including the `::{HLC}` suffix. Opaque, and directly
    /// usable as a keyset cursor (`after_label`).
    pub order_label: String,
    /// Child name, carried in the index entry's value.
    pub name: String,
}

impl NodeRepositoryImpl {
    /// Scan a parent's children in editorial order, with an optional exclusive
    /// keyset cursor and limit.
    ///
    /// - `start` — where to begin; see [`OrderedScanStart`].
    /// - `limit` — stop after this many live children. `None` scans the whole
    ///   parent.
    /// - `descending` — walk the order backwards.
    /// - `max_revision` — MVCC bound; entries newer than this are ignored.
    ///
    /// # Keyset caveat
    ///
    /// The sort key is mutable: a child reordered from before the cursor to
    /// after it will be seen again on a later page, and one moved the other way
    /// may be skipped. This is inherent to keyset pagination over a mutable
    /// ordering, not a defect of this scan.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn list_ordered_children_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        start: OrderedScanStart<'_>,
        limit: Option<usize>,
        descending: bool,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<OrderedChildEntry>> {
        if limit == Some(0) {
            return Ok(Vec::new());
        }

        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Bound the iterator to exactly this parent's key range so reverse
        // iteration is correct under the CF's prefix extractor.
        let mut opts = ReadOptions::default();
        opts.set_iterate_lower_bound(prefix.clone());
        opts.set_iterate_upper_bound(prefix_upper_bound(&prefix));

        // Owned here so the iterator can borrow it for the whole scan.
        let seek_key = seek_key(&prefix, &start, descending);
        let mode = match (seek_key.as_deref(), descending) {
            (None, false) => IteratorMode::Start,
            (None, true) => IteratorMode::End,
            (Some(seek), false) => IteratorMode::From(seek, Direction::Forward),
            (Some(seek), true) => IteratorMode::From(seek, Direction::Reverse),
        };
        let iter = self.db.iterator_cf_opt(cf_ordered, opts, mode);

        // MVCC: `(label, child_id)` dedupe keeps only the newest revision of
        // each entry (descending revision encoding puts it first). The separate
        // `child_id` set collapses a child that still has live entries at more
        // than one label — which happens when a branch merge copies labels
        // verbatim.
        let mut seen_entries: HashSet<(String, String)> = HashSet::new();
        let mut seen_child_ids: HashSet<String> = HashSet::new();
        let mut out = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            let Some(parsed) = parse_ordered_child_key(&key, &prefix) else {
                continue;
            };

            // Enforce the start bound precisely. The seek only positions us
            // close: a label is a variable-length prefix of the remaining key, so
            // the boundary group still has to be filtered here.
            if !start.admits(parsed.order_label, descending) {
                continue;
            }

            if let Some(max_rev) = max_revision {
                match parsed.revision() {
                    Some(rev) if &rev > max_rev => continue,
                    _ => {}
                }
            }

            let entry_key = (parsed.order_label.to_string(), parsed.child_id.to_string());
            if !seen_entries.insert(entry_key) {
                continue;
            }

            // Tombstones are recorded above before being skipped, so an older
            // live revision of the same entry cannot resurrect it.
            if is_tombstone(&value) {
                continue;
            }

            if !seen_child_ids.insert(parsed.child_id.to_string()) {
                continue;
            }

            out.push(OrderedChildEntry {
                child_id: parsed.child_id.to_string(),
                order_label: parsed.order_label.to_string(),
                name: String::from_utf8_lossy(&value).to_string(),
            });

            if let Some(limit) = limit {
                if out.len() >= limit {
                    break;
                }
            }
        }

        Ok(out)
    }

    /// Look up a single child's current order label.
    ///
    /// Thin wrapper over [`Self::get_order_label_for_child`], kept as the
    /// name the storage trait exposes.
    pub(in crate::repositories::nodes) fn get_child_order_label_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        child_id: &str,
    ) -> Result<Option<String>> {
        self.get_order_label_for_child(tenant_id, repo_id, branch, workspace, parent_id, child_id)
    }
}

/// Smallest key strictly greater than every key under `prefix`.
///
/// Increments the last non-`0xFF` byte and truncates, which is the standard
/// prefix-successor construction. A prefix of all `0xFF` bytes has no successor,
/// in which case the range is left unbounded above — correct, if slightly
/// broader than necessary. (Our prefixes always end in the `\0` separator, so
/// this never actually happens.)
fn prefix_upper_bound(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last != 0xFF {
            end.push(last + 1);
            return end;
        }
    }
    Vec::new()
}

/// Where to position the iterator for the requested direction and cursor.
///
/// Returns `None` when there is no cursor (start from either end).
///
/// Forward: seek to `{prefix}{label}\0\xFF`, i.e. past the whole
/// `{label}\0{~rev}\0{child}` group, so every revision of the cursor entry is
/// skipped. Reverse: seek backwards from `{prefix}{label}`, landing on the last
/// key before the cursor's group. Either way
/// [`NodeRepositoryImpl::list_ordered_children_impl`] re-checks the label so the
/// bound is exact regardless of how the seek rounds.
fn seek_key(prefix: &[u8], start: &OrderedScanStart<'_>, descending: bool) -> Option<Vec<u8>> {
    let label = start.label()?;
    let mut seek = Vec::with_capacity(prefix.len() + label.len() + 2);
    seek.extend_from_slice(prefix);
    seek.extend_from_slice(label.as_bytes());

    // Forward + exclusive is the only case that must skip past the label's whole
    // `{label}\0{~rev}\0{child}` group. Every other combination lands at or
    // before the group and is narrowed by `OrderedScanStart::admits`.
    if !descending && matches!(start, OrderedScanStart::After(_)) {
        seek.push(0);
        seek.push(0xFF);
    }
    Some(seek)
}
