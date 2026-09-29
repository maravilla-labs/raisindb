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
use rocksdb::ReadOptions;
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

        // A raw iterator: the entries are only looked at, and the boxed
        // iterator copied every key and value it yielded. On a real content
        // tree a parent's range holds one entry per revision of every child
        // (each update restamps the label), so this loop runs far more often
        // than it emits, and its per-entry cost is the cost of listing.
        let seek_key = seek_key(&prefix, &start, descending);
        let mut iter = self.db.raw_iterator_cf_opt(cf_ordered, opts);
        match (seek_key.as_deref(), descending) {
            (None, false) => iter.seek_to_first(),
            (None, true) => iter.seek_to_last(),
            (Some(seek), false) => iter.seek(seek),
            (Some(seek), true) => iter.seek_for_prev(seek),
        }

        // MVCC: `(label, child_id)` dedupe keeps only the first entry seen of
        // each — the newest revision, which descending revision encoding puts
        // first. Keys sort by label before revision and child, so one label's
        // entries are contiguous: the dedupe only has to remember the children
        // of the label group it is in, not every pair seen so far. The separate
        // `child_id` set collapses a child that still has live entries at more
        // than one label — which happens when a branch merge copies labels
        // verbatim — and is only touched for entries that are emitted.
        let mut group_label: Vec<u8> = Vec::new();
        let mut group_children: Vec<Vec<u8>> = Vec::new();
        let mut seen_child_ids: HashSet<String> = HashSet::new();
        let mut out = Vec::new();

        while iter.valid() {
            let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
                break;
            };

            if let Some(parsed) = parse_ordered_child_key(key, &prefix) {
                // Enforce the start bound precisely. The seek only positions us
                // close: a label is a variable-length prefix of the remaining
                // key, so the boundary group still has to be filtered here.
                let visible = start.admits(parsed.order_label, descending)
                    && match (max_revision, parsed.revision()) {
                        (Some(max_rev), Some(rev)) => &rev <= max_rev,
                        _ => true,
                    };

                if visible {
                    if parsed.order_label.as_bytes() != group_label.as_slice() {
                        group_label.clear();
                        group_label.extend_from_slice(parsed.order_label.as_bytes());
                        group_children.clear();
                    }
                    let child = parsed.child_id.as_bytes();
                    let first_of_entry = !group_children.iter().any(|c| c.as_slice() == child);
                    if first_of_entry {
                        group_children.push(child.to_vec());
                    }

                    // Tombstones are recorded above before being skipped, so an
                    // older live revision of the same entry cannot resurrect it.
                    if first_of_entry
                        && !is_tombstone(value)
                        && seen_child_ids.insert(parsed.child_id.to_string())
                    {
                        out.push(OrderedChildEntry {
                            child_id: parsed.child_id.to_string(),
                            order_label: parsed.order_label.to_string(),
                            name: String::from_utf8_lossy(value).to_string(),
                        });

                        if limit.is_some_and(|limit| out.len() >= limit) {
                            break;
                        }
                    }
                }
            }

            if descending {
                iter.prev();
            } else {
                iter.next();
            }
        }
        iter.status()
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

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
