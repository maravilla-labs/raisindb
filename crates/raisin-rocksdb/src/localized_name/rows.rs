//! Revision-bounded reads of `cf::LOCALIZED_NAME_INDEX`: a node's reverse rows,
//! and the claims on one segment. Both go through the shared grouped
//! newest-at-or-before scan (`translation_read::for_each_newest`): per group,
//! the newest version at or before the bound decides, and `T` means "none".

use super::keys::{self, NameScope, Segment};
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::BTreeMap;

/// One locale's reverse row as of a bound: its revision, and the segment
/// (`None`: a tombstone — no name from then on).
pub type ReverseRow = (HLC, Option<Segment>);

/// The node's reverse row per locale, newest at or before `bound` (newest at
/// all when `None`).
pub fn reverse_rows(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    bound: Option<&HLC>,
) -> Result<BTreeMap<String, ReverseRow>> {
    let prefix = keys::reverse_node_prefix(scope, node_id);
    let mut out = BTreeMap::new();
    crate::translation_read::for_each_newest(
        db,
        cf::LOCALIZED_NAME_INDEX,
        &prefix,
        1,
        bound,
        |seg, rev, value| {
            out.insert(seg[0].to_string(), (rev, keys::decode_segment(value)));
        },
    )?;
    Ok(out)
}

/// The newest revision of ANY reverse row of the node (no bound).
pub(crate) fn newest_reverse_revision(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
) -> Result<Option<HLC>> {
    Ok(reverse_rows(db, scope, node_id, None)?
        .into_values()
        .map(|(rev, _)| rev)
        .max())
}

/// Every node with a LIVE claim on `(locale, parent_id, name)` as of
/// `bound`, newest claim first (ties: by node id) — the candidates a lookup
/// verifies. A claim is a hint: the selector decides (`lookup::verify`).
pub fn claims(
    db: &DB,
    scope: NameScope<'_>,
    locale: &str,
    parent_id: &str,
    name: &str,
    bound: Option<&HLC>,
) -> Result<Vec<(HLC, String)>> {
    claims_in(
        &mut crate::mvcc_read::DbRead(db),
        scope,
        locale,
        parent_id,
        name,
        bound,
    )
}

/// [`claims`] through a read source (a lookup's pinned iterators).
pub(crate) fn claims_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    scope: NameScope<'_>,
    locale: &str,
    parent_id: &str,
    name: &str,
    bound: Option<&HLC>,
) -> Result<Vec<(HLC, String)>> {
    let prefix = keys::forward_prefix(scope, locale, parent_id, name);
    let mut out = Vec::new();
    crate::translation_read::for_each_newest_in(
        src,
        cf::LOCALIZED_NAME_INDEX,
        &prefix,
        1,
        bound,
        |seg, rev, value| {
            if !crate::keys::is_tombstone_value(value) {
                out.push((rev, seg[0].to_string()));
            }
        },
    )?;
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(out)
}

/// The OLDEST reverse row of `(node, locale)` strictly above `revision` —
/// the state that superseded a write landing at `revision` out of order.
pub(crate) fn next_newer_row(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    locale: &str,
    revision: &HLC,
) -> Result<Option<ReverseRow>> {
    let cf = cf_handle(db, cf::LOCALIZED_NAME_INDEX)?;
    let prefix = keys::reverse_locale_prefix(scope, node_id, locale);
    // Keys run newest first: the last one above `revision` is the next newer.
    let mut found = None;
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        let Some((_, rev)) = keys::parse_reverse_key(&key) else {
            continue;
        };
        if rev <= *revision {
            break;
        }
        found = Some((rev, keys::decode_segment(&value)));
    }
    Ok(found)
}
