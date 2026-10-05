//! THE compound-index writer (plan Phase 8 step 2).
//!
//! The previous writer tombstoned a node's old entries by prefix-scanning the
//! WHOLE workspace compound keyspace (twice: draft and published) on every
//! update and writing `T` over each found key IN PLACE — O(workspace) per write
//! and every superseded entry's history destroyed. Now the old tuple is
//! DERIVED from the baseline version (the same [`compound_entries`] the new
//! one comes from), the tombstone is a new key at the write revision (see
//! `group.rs` for where exactly it lands), and under a proven predecessor an
//! unchanged tuple is not re-put at all.
//!
//! The baseline is the property index's ([`Baseline`]): one decision about
//! what a write supersedes, made once (`indexing::resolve_baseline`), so the
//! two writers cannot disagree. Skipping therefore follows
//! `index.skip_unchanged` and its per-branch gate, and the transaction
//! commit's re-check (`StagedDeltaCheck`) corrects the compound write too.

use super::defs::DefsSet;
use super::entries::{compound_entries, entry_key, CompoundGroup};
use super::group::tombstone_revision as landing_revision;
use crate::indexing::{Baseline, IndexCtx};
use crate::keys::TOMBSTONE_VALUE as TOMBSTONE;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, WriteBatch, DB};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

/// The value of a live compound entry (the key carries everything).
pub const LIVE: &[u8] = b"";

static SKIPPED: AtomicU64 = AtomicU64::new(0);

/// Unchanged compound entries NOT re-put since process start.
pub fn skipped_unchanged_compound_entries() -> u64 {
    SKIPPED.load(Ordering::Relaxed)
}

/// What one compound write staged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompoundCounts {
    pub puts: usize,
    pub tombstones: usize,
    pub skipped: usize,
}

fn cf(db: &DB) -> Result<&ColumnFamily> {
    crate::cf_handle(db, crate::cf::COMPOUND_INDEX)
}

fn entries_of(defs: &DefsSet, ctx: &IndexCtx<'_>, node: &Node) -> BTreeSet<CompoundGroup> {
    compound_entries(defs.compound(&node.node_type), ctx, node)
}

/// Write `new`'s compound entries at `revision` against `baseline`.
///
/// `defs` must hold the node type of `new`, of the baseline's prior version
/// and of every successor (build it with [`DefsSet::resolve`] or
/// [`DefsSet::peek`] over [`types_of`]).
pub fn write_compound_delta(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    defs: &DefsSet,
    baseline: Baseline<'_>,
    new: &Node,
    revision: &HLC,
) -> Result<CompoundCounts> {
    let cf = cf(db)?;
    let (prior, successors) = match baseline {
        Baseline::Predecessor(old) => (Some(old), &[][..]),
        Baseline::Full(prior) => (prior, &[][..]),
        Baseline::OutOfOrder { prior, successors } => (prior, successors),
        Baseline::NoPrior => (None, &[][..]),
    };
    let old = prior
        .map(|old| entries_of(defs, ctx, old))
        .unwrap_or_default();
    let new_entries = entries_of(defs, ctx, new);
    let skip = matches!(baseline, Baseline::Predecessor(_));
    let mut counts = CompoundCounts::default();

    for group in old.difference(&new_entries) {
        let at = land(db, cf, group, &new.id, revision, successors.is_empty())?;
        batch.put_cf(cf, entry_key(group, &at, &new.id), TOMBSTONE);
        counts.tombstones += 1;
    }
    let mut in_place: Option<bool> = None;
    for group in &new_entries {
        if skip && old.contains(group) {
            counts.skipped += 1;
            continue;
        }
        let mut at = land(db, cf, group, &new.id, revision, successors.is_empty())?;
        if at != *revision && !*in_place.get_or_insert(stored_at(db, ctx, &new.id, revision)?) {
            at = *revision;
        }
        batch.put_cf(cf, entry_key(group, &at, &new.id), LIVE);
        counts.puts += 1;
    }
    if let Some((first_rev, first)) = successors.first() {
        // End this version's tuples the first successor does not hold at ITS
        // revision (a delete ends all of them), then re-assert every
        // successor: a skip-written successor keeps unchanged tuples BELOW
        // `revision`, which the tombstones above would otherwise mask.
        let first_entries = first
            .as_ref()
            .map(|node| entries_of(defs, ctx, node))
            .unwrap_or_default();
        for group in new_entries.difference(&first_entries) {
            batch.put_cf(cf, entry_key(group, first_rev, &new.id), TOMBSTONE);
        }
        reassert_successors(batch, cf, ctx, defs, &new.id, successors);
    }
    if counts.skipped > 0 {
        SKIPPED.fetch_add(counts.skipped as u64, Ordering::Relaxed);
    }
    Ok(counts)
}

fn land(
    db: &DB,
    cf: &ColumnFamily,
    group: &[u8],
    node_id: &str,
    revision: &HLC,
    no_successors: bool,
) -> Result<HLC> {
    if no_successors {
        landing_revision(db, cf, group, node_id, revision)
    } else {
        Ok(*revision)
    }
}

/// Whether `node_id` already has a stored version AT `revision` — the write
/// is IN PLACE (`versionable=false` reuses the node's current revision).
///
/// Only an in-place write lands its PUTS on a newer entry of the group (an
/// entry above its reused revision would otherwise mask it). An ordinary
/// write's put stays at R: an entry above R with no stored successor is an
/// index-only write — an ancestor move re-keying `__parent_path` — and a put
/// moved onto that move's tombstone would overwrite it with LIVE, turning a
/// tuple the move ended into a live one at HEAD. Asked lazily, only when an
/// entry above R exists.
fn stored_at(db: &DB, ctx: &IndexCtx<'_>, node_id: &str, revision: &HLC) -> Result<bool> {
    let key = crate::keys::node_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        node_id,
        revision,
    );
    let cf_nodes = crate::cf_handle(db, crate::cf::NODES)?;
    Ok(db
        .get_pinned_cf(cf_nodes, key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        .is_some())
}

/// Re-put every live successor's entries at its own revision.
pub(crate) fn reassert_successors(
    batch: &mut WriteBatch,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    defs: &DefsSet,
    node_id: &str,
    successors: &[crate::mvcc_read::StoredVersion],
) {
    for (at, version) in successors {
        if let Some(version) = version {
            for group in entries_of(defs, ctx, version) {
                batch.put_cf(cf, entry_key(&group, at, node_id), LIVE);
            }
        }
    }
}

/// Tombstone, at `revision`, every entry `old` indexes that `new` does not
/// (a merge resolution's union tombstones; the new version is written with
/// [`write_compound_delta`] afterwards).
pub fn tombstone_superseded_compound(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    defs: &DefsSet,
    old: &Node,
    new: &Node,
    revision: &HLC,
) -> Result<usize> {
    let cf = cf(db)?;
    let new_entries = entries_of(defs, ctx, new);
    let mut written = 0;
    for group in entries_of(defs, ctx, old).difference(&new_entries) {
        let at = landing_revision(db, cf, group, &old.id, revision)?;
        batch.put_cf(cf, entry_key(group, &at, &old.id), TOMBSTONE);
        written += 1;
    }
    Ok(written)
}

/// Every node type a write against `baseline` touches: the new version's, the
/// prior's, and every successor's — what its [`DefsSet`] must hold.
pub fn types_of<'a>(baseline: &Baseline<'a>, new: &'a Node) -> Vec<&'a str> {
    let mut types = vec![new.node_type.as_str()];
    let (prior, successors): (Option<&Node>, &[crate::mvcc_read::StoredVersion]) = match baseline {
        Baseline::Predecessor(old) => (Some(old), &[]),
        Baseline::Full(prior) => (*prior, &[]),
        Baseline::OutOfOrder { prior, successors } => (*prior, successors),
        Baseline::NoPrior => (None, &[]),
    };
    if let Some(prior) = prior {
        types.push(prior.node_type.as_str());
    }
    for (_, version) in successors {
        if let Some(version) = version {
            types.push(version.node_type.as_str());
        }
    }
    types.sort_unstable();
    types.dedup();
    types
}
