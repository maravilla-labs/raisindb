//! The two label reads every ORDERED_CHILDREN writer depends on (plan Phase 7
//! items 6 and 7): the parent's last LIVE label, and a child's current label
//! verified by one `(parent, label)` seek.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use super::parse_ordered_child_key;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};
use std::collections::HashSet;

/// The parent's last label in editorial order among its LIVE children — what
/// an append must be minted after when the `LAST` metadata cache is absent.
///
/// Each `(label, child)` pair is decided by its NEWEST entry (keys run newest
/// first within a pair), so a tombstone hides every older entry of the pair:
/// an older live entry of a deleted or relabelled child never counts again.
/// The previous fallbacks counted such stale labels (one returned the label
/// with the highest revision, the other the greatest label, live or not);
/// once writes may skip unchanged entries and history GC collapses runs, those
/// maxima shift, and the next append could mint a label that sorts before or
/// collides with a sibling's.
///
/// The ONE fallback scan: the repository and the replication apply path both
/// call it.
pub(crate) fn last_live_order_label(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
) -> Result<Option<String>> {
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;
    let prefix = keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
    let mut decided: HashSet<(String, String)> = HashSet::new();
    let mut last: Option<String> = None;
    for item in crate::prefix_scan(db, cf_ordered, prefix.clone()) {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        let Some(parsed) = parse_ordered_child_key(&key, &prefix) else {
            continue;
        };
        if !decided.insert((parsed.order_label.to_string(), parsed.child_id.to_string())) {
            continue; // an older entry of a pair its newest entry decided
        }
        if is_tombstone(&value) {
            continue;
        }
        if last
            .as_deref()
            .is_none_or(|current| super::sorts_after(parsed.order_label, current))
        {
            last = Some(parsed.order_label.to_string());
        }
    }
    Ok(last)
}

/// The value (the child's NAME) of `child_id`'s entry under `label`, when its
/// newest entry (at or below `at_or_below`, when given) is live; `None` when
/// there is none or it is a tombstone.
///
/// One seek on `{parent prefix}{label}\0`: the key also carries the entry's
/// revision before the child id, so an exact-key point read is impossible, but
/// every entry under one label is few (labels carry a minting HLC).
#[allow(clippy::too_many_arguments)]
pub(crate) fn live_entry_under_label(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    label: &str,
    child_id: &str,
    at_or_below: Option<&HLC>,
) -> Result<Option<Vec<u8>>> {
    if label.is_empty() || label.contains('\0') {
        return Ok(None);
    }
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;
    let parent_prefix =
        keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
    let mut label_prefix = parent_prefix.clone();
    label_prefix.extend_from_slice(label.as_bytes());
    label_prefix.push(0);
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    if let Some(upper) = crate::prefix_successor(&label_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_ordered, opts);
    iter.seek(&label_prefix);
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        if !key.starts_with(&label_prefix) {
            break;
        }
        if let Some(parsed) = parse_ordered_child_key(key, &parent_prefix) {
            let in_bound = match at_or_below {
                None => true,
                Some(bound) => keys::decode_descending_revision(parsed.revision_bytes)
                    .is_ok_and(|revision| revision <= *bound),
            };
            if in_bound && parsed.order_label == label && parsed.child_id == child_id {
                // Newest entry of the pair (within the bound) decides.
                return Ok((!is_tombstone(value)).then(|| value.to_vec()));
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    Ok(None)
}

/// A child's current label under a parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentLabel {
    pub(crate) label: String,
    /// The stored entry's value (the child's name at that entry), when the
    /// label came from the verified hint.
    pub(crate) stored_name: Option<Vec<u8>>,
}

impl NodeRepositoryImpl {
    /// `child_id`'s current label under `parent_id`: the `hint` (the stored
    /// record's `order_key`) when one `(parent, label)` seek proves it live,
    /// else the sibling scan ([`super::stored_order_label`]) — legacy records
    /// carry an empty or a copy SOURCE's `order_key`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn current_order_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        child_id: &str,
        hint: Option<&str>,
    ) -> Result<Option<CurrentLabel>> {
        current_order_label(
            &self.db, tenant_id, repo_id, branch, workspace, parent_id, child_id, hint,
        )
    }
}

/// [`NodeRepositoryImpl::current_order_label`] over a bare database handle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn current_order_label(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    child_id: &str,
    hint: Option<&str>,
) -> Result<Option<CurrentLabel>> {
    if let Some(hint) = hint.filter(|h| !h.is_empty()) {
        if let Some(name) = live_entry_under_label(
            db, tenant_id, repo_id, branch, workspace, parent_id, hint, child_id, None,
        )? {
            return Ok(Some(CurrentLabel {
                label: hint.to_string(),
                stored_name: Some(name),
            }));
        }
    }
    Ok(super::stored_order_label(
        db, tenant_id, repo_id, branch, workspace, parent_id, child_id,
    )?
    .map(|label| CurrentLabel {
        label,
        stored_name: None,
    }))
}
