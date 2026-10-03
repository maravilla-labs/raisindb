//! Iteration over `PROPERTY_INDEX` value groups. See the module docs in
//! `reader/mod.rs` for the rule this applies.

use super::super::helpers::is_tombstone;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{DBRawIteratorWithThreadMode, ReadOptions, DB};
use std::collections::HashSet;

type RawIter<'a> = DBRawIteratorWithThreadMode<'a, DB>;

/// One end of a value range, in index-key value bytes.
pub(crate) enum ValueBound {
    Unbounded,
    Value { bytes: Vec<u8>, inclusive: bool },
}

impl ValueBound {
    /// `value` lies beyond this bound when it is the LOWER bound.
    fn below(&self, value: &[u8]) -> bool {
        match self {
            Self::Unbounded => false,
            Self::Value { bytes, inclusive } => {
                value < bytes.as_slice() || (!inclusive && value == bytes.as_slice())
            }
        }
    }

    /// `value` lies beyond this bound when it is the UPPER bound.
    fn above(&self, value: &[u8]) -> bool {
        match self {
            Self::Unbounded => false,
            Self::Value { bytes, inclusive } => {
                value > bytes.as_slice() || (!inclusive && value == bytes.as_slice())
            }
        }
    }
}

/// A key split into its value, revision and node id, by fixed width from the
/// END: `{value}\0{~rev:16}\0{node_id}` after the property prefix. Neither the
/// value (an 8-byte timestamp can hold `0x00`) nor the revision (likewise) may
/// be found by splitting on the separator; the node id never contains one.
struct Entry<'k> {
    value: &'k [u8],
    /// `None` when the 16 bytes do not decode — the entry is skipped, and the
    /// scan carries on.
    revision: Option<HLC>,
    node_id: &'k [u8],
}

fn parse(key: &[u8], base_len: usize) -> Option<Entry<'_>> {
    let rest = key.get(base_len..)?;
    let node_sep = rest.iter().rposition(|b| *b == 0)?;
    let node_id = &rest[node_sep + 1..];
    let revision_start = node_sep.checked_sub(16)?;
    let value_sep = revision_start.checked_sub(1)?;
    if node_id.is_empty() || rest[value_sep] != 0 {
        return None;
    }
    Some(Entry {
        value: &rest[..value_sep],
        revision: HLC::decode_descending(&rest[revision_start..node_sep]).ok(),
        node_id,
    })
}

fn iterator<'a>(db: &'a DB, base: &[u8]) -> Result<RawIter<'a>> {
    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.set_iterate_lower_bound(base.to_vec());
    if let Some(upper) = crate::prefix_successor(base) {
        opts.set_iterate_upper_bound(upper);
    }
    Ok(db.raw_iterator_cf_opt(cf, opts))
}

fn value_prefix(base: &[u8], value: &[u8]) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(base.len() + value.len() + 1);
    prefix.extend_from_slice(base);
    prefix.extend_from_slice(value);
    prefix.push(0);
    prefix
}

/// Decide every node of ONE value group as of `at`, calling `live(node_id)`
/// for each match. Returns `Ok(false)` when `live` asked to stop.
fn read_group(
    iter: &mut RawIter<'_>,
    base_len: usize,
    value: &[u8],
    prefix: &[u8],
    at: Option<&HLC>,
    live: &mut dyn FnMut(&str) -> Result<bool>,
) -> Result<bool> {
    let mut seek = prefix.to_vec();
    if let Some(at) = at {
        seek.extend_from_slice(&at.encode_descending());
    }
    iter.seek(&seek);

    // Node ids already decided in this group: their first entry seen was
    // their newest at or before `at`.
    let mut decided: HashSet<Vec<u8>> = HashSet::new();
    while iter.valid() {
        let (Some(key), Some(stored)) = (iter.key(), iter.value()) else {
            break;
        };
        if !key.starts_with(prefix) {
            break;
        }
        if let Some(entry) = parse(key, base_len) {
            let visible = entry.value == value
                && entry
                    .revision
                    .is_some_and(|revision| at.is_none_or(|at| &revision <= at));
            if visible && decided.insert(entry.node_id.to_vec()) && !is_tombstone(stored) {
                if let Ok(node_id) = std::str::from_utf8(entry.node_id) {
                    if !live(node_id)? {
                        return Ok(false);
                    }
                }
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(true)
}

/// Every node whose `(value, node)` entry is live at `at`.
pub(super) fn visit_value(
    db: &DB,
    base: &[u8],
    value: &[u8],
    at: Option<&HLC>,
    mut live: impl FnMut(&str) -> Result<bool>,
) -> Result<()> {
    let mut iter = iterator(db, base)?;
    let prefix = value_prefix(base, value);
    read_group(&mut iter, base.len(), value, &prefix, at, &mut live)?;
    Ok(())
}

/// Every live `(value, node)` at `at` with the value within the bounds, in
/// value order. One seek per value group; groups outside the bounds are
/// skipped whole.
#[allow(clippy::too_many_arguments)]
pub(super) fn visit_values(
    db: &DB,
    base: &[u8],
    lower: &ValueBound,
    upper: &ValueBound,
    ascending: bool,
    at: Option<&HLC>,
    mut visit: impl FnMut(&[u8], &str) -> Result<bool>,
) -> Result<()> {
    let mut iter = iterator(db, base)?;

    // Position on the first group in scan order.
    match (ascending, lower, upper) {
        (true, ValueBound::Value { bytes, .. }, _) => iter.seek(value_prefix(base, bytes)),
        (true, ValueBound::Unbounded, _) => iter.seek_to_first(),
        (false, _, ValueBound::Value { bytes, .. }) => {
            // Past every key of the bound's own group: `{base}{bytes}\x01`.
            let mut target = base.to_vec();
            target.extend_from_slice(bytes);
            target.push(1);
            iter.seek_for_prev(target)
        }
        (false, _, ValueBound::Unbounded) => iter.seek_to_last(),
    }

    while iter.valid() {
        let Some(key) = iter.key() else {
            break;
        };
        let Some(entry) = parse(key, base.len()) else {
            // Not a well-formed entry: step over this one key.
            if ascending {
                iter.next();
            } else {
                iter.prev();
            }
            continue;
        };
        let value = entry.value.to_vec();
        let prefix = value_prefix(base, &value);

        let (before_start, past_end) = if ascending {
            (lower.below(&value), upper.above(&value))
        } else {
            (upper.above(&value), lower.below(&value))
        };
        if past_end {
            break;
        }
        if !before_start {
            let mut live = |node_id: &str| visit(value.as_slice(), node_id);
            if !read_group(&mut iter, base.len(), &value, &prefix, at, &mut live)? {
                return Ok(());
            }
        }

        // On to the next group in scan order.
        if ascending {
            match crate::prefix_successor(&prefix) {
                Some(next) => iter.seek(next),
                None => break,
            }
        } else {
            iter.seek_for_prev(&prefix);
        }
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(())
}
