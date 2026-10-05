//! The rows one sync stages: claims, reverse rows and tombstones, diffed
//! against the node's reverse rows as of the write's revision (see `sync`).

use super::keys::{self, NameScope, Segment};
use super::rows::{self, ReverseRow};
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use std::collections::{BTreeMap, BTreeSet};

/// What a sync knows beyond the stored rows — rows an earlier step staged in
/// the SAME unwritten batch, which no read can see.
#[derive(Debug, Default, Clone)]
pub(crate) struct Prior {
    /// Re-stage the final state of every locale considered (claim and reverse
    /// row, or a reverse `T`), even where the stored rows already say it. A
    /// final-view re-sync needs it: earlier syncs of the same batch, at the
    /// same revision, staged rows from a partial view (the new node with the
    /// old overlays, the old node with the new overlay) that a delta against
    /// the STORED rows would leave in place.
    pub full: bool,
    /// Further locales to consider: those an earlier sync of this batch may
    /// have staged rows for.
    pub locales: BTreeSet<String>,
    /// Reverse rows staged earlier in this batch, by locale. One newer than
    /// the stored row as of the bound IS the current state.
    pub staged: BTreeMap<String, ReverseRow>,
}

/// The puts of one plan, how many locales actually changed, and the reverse
/// rows it staged (a catch-up of the same batch builds on them).
pub(crate) struct Planned {
    pub rows: Vec<(Vec<u8>, Vec<u8>)>,
    pub changed: usize,
    pub staged: BTreeMap<String, ReverseRow>,
}

/// Stage what makes the index say `desired` (locale -> name) about `node_id`
/// under `parent_id` at `revision`, against its reverse rows as of then.
/// Returns what it staged.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node_id: &str,
    parent_id: Option<&str>,
    desired: &BTreeMap<String, String>,
    revision: &HLC,
    prior: &Prior,
) -> Result<Planned> {
    let cf = cf_handle(db, cf::LOCALIZED_NAME_INDEX)?;
    let planned = plan_rows(db, scope, node_id, parent_id, desired, revision, prior)?;
    for (key, value) in &planned.rows {
        batch.put_cf(cf, key, value);
    }
    Ok(planned)
}

/// The `(key, value)` puts [`apply`] stages, for writers that commit through
/// their own bounded writer (the rebuild). `parent_id` may be `None` only
/// when `desired` is empty (a claim needs its parent).
pub(crate) fn plan_rows(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    parent_id: Option<&str>,
    desired: &BTreeMap<String, String>,
    revision: &HLC,
    prior: &Prior,
) -> Result<Planned> {
    let mut current = rows::reverse_rows(db, scope, node_id, Some(revision))?;
    for (locale, (rev, segment)) in &prior.staged {
        let newer = current.get(locale).is_none_or(|(stored, _)| rev >= stored);
        if rev <= revision && newer {
            current.insert(locale.clone(), (*rev, segment.clone()));
        }
    }
    let mut locales: BTreeSet<&String> = current.keys().chain(desired.keys()).collect();
    if prior.full {
        locales.extend(prior.locales.iter());
    }
    let mut out = Planned {
        rows: Vec::new(),
        changed: 0,
        staged: BTreeMap::new(),
    };
    for locale in locales {
        let old = current.get(locale).and_then(|(_, s)| s.clone());
        let new = match (desired.get(locale), parent_id) {
            (Some(name), Some(parent)) => Some(Segment {
                parent_id: parent.to_string(),
                name: name.clone(),
            }),
            (Some(_), None) => {
                return Err(raisin_error::Error::invalid_state(
                    "localized name index: a claim without its parent",
                ))
            }
            (None, _) => None,
        };
        if old == new && !prior.full {
            continue;
        }
        if old != new {
            out.changed += 1;
        }
        let reverse = keys::reverse_key(scope, node_id, locale, revision);
        match &new {
            Some(segment) => {
                out.rows.push((
                    keys::forward_key(
                        scope,
                        locale,
                        &segment.parent_id,
                        &segment.name,
                        node_id,
                        revision,
                    ),
                    Vec::new(),
                ));
                out.rows.push((reverse, keys::encode_segment(segment)));
                // Applied below newer state (out of order): the newer state is
                // authoritative from its revision on, so this claim ends there
                // — no phantom claim survives the write that superseded it.
                if let Some((newer, newer_segment)) =
                    rows::next_newer_row(db, scope, node_id, locale, revision)?
                {
                    if newer_segment.as_ref() != Some(segment) {
                        out.rows.push((
                            keys::forward_key(
                                scope,
                                locale,
                                &segment.parent_id,
                                &segment.name,
                                node_id,
                                &newer,
                            ),
                            crate::keys::TOMBSTONE_VALUE.to_vec(),
                        ));
                    }
                }
            }
            None => out
                .rows
                .push((reverse, crate::keys::TOMBSTONE_VALUE.to_vec())),
        }
        if let Some(old) = old.filter(|old| new.as_ref() != Some(old)) {
            out.rows.push((
                keys::forward_key(scope, locale, &old.parent_id, &old.name, node_id, revision),
                crate::keys::TOMBSTONE_VALUE.to_vec(),
            ));
        }
        out.staged.insert(locale.clone(), (*revision, new));
    }
    Ok(out)
}
