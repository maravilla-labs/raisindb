//! The ONE PROPERTY_INDEX writer: write what changed, keep what did not (plan
//! Phase 7, the PostgreSQL HOT idea).
//!
//! A node update used to re-put every property-index entry of the node — every
//! custom property, every pseudo-property and every IS_A / HAS_MIXIN member —
//! at the new revision, so an edit of one field on a node with 30 properties
//! wrote ~40 entries, and three mirrored writers (transaction, repository,
//! replication) did it each in their own way. [`write_property_index_delta`]
//! is now the only writer: it derives a version's entries once
//! ([`entries::entries_of`]), tombstones those the new version no longer has,
//! and puts the new ones — ALL of them, or, under a proven baseline, only the
//! ones that changed.
//!
//! # The baseline decides what may be skipped
//!
//! Skipping an unchanged entry leaves the live entry at the PREDECESSOR's
//! revision answering for the new one. That is right only when the
//! predecessor is the version strictly below the write revision, read bounded
//! by it, and nothing newer exists ([`Baseline::Predecessor`]). Anything else
//! is [`Baseline::Full`]: a full put at the revision, tombstones relative to
//! the prior version, a successor's entries left alone. A wrong "latest"
//! baseline would either leave live puts at R that nothing later tombstones
//! (phantom HEAD matches) or write nothing at R (time travel shows the old
//! value). See [`resolve_baseline`] for the one place a baseline is decided.

mod baseline;
mod entries;
mod in_place;
mod probe;
mod staged;
mod staged_compound;
mod staged_declarations;
mod staged_markers;
mod staged_neighbours;
mod staged_rederive;
#[cfg(test)]
mod tests;

pub use baseline::resolve_baseline;
pub(crate) use entries::{entries_of, EntryValue, PropertyEntry};
pub use in_place::{in_place_scans_capped, InPlace, InPlaceTargets};
pub(crate) use probe::{entry_state_as_of, EntryState};
pub use staged::{corrected_staged_writes, PendingDeltaCheck, StagedDeltaCheck};
pub use staged_declarations::fail_changed_declarations;

use crate::indexing::IndexCtx;
use crate::keys::TOMBSTONE_VALUE as TOMBSTONE;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, WriteBatch, DB};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

static SKIPPED: AtomicU64 = AtomicU64::new(0);

/// Unchanged PROPERTY_INDEX entries the writer has NOT re-put, since process
/// start — the operational signal that `index.skip_unchanged` is in effect
/// (it stays 0 while the flag is off or no branch is rebuilt).
pub fn skipped_unchanged_entries() -> u64 {
    SKIPPED.load(Ordering::Relaxed)
}

/// What a property-index write is diffed against.
#[derive(Debug, Clone, Copy)]
pub enum Baseline<'a> {
    /// The newest version strictly below the write revision, and no version
    /// at or above it exists: unchanged entries are skipped.
    Predecessor(&'a Node),
    /// Full put of every entry at the revision, tombstoning what the prior
    /// version (if any) indexed that the new one does not. Used whenever skip
    /// is not proven safe: the flag is off, the node was not rebuilt by this
    /// writer, a newer version exists, the replication apply path, merge, and
    /// re-stamps (move, reorder, rebalance).
    Full(Option<&'a Node>),
    /// A full put against `prior` (the newest version strictly below the
    /// revision) while NEWER versions are already stored — a write landing
    /// below what a skip-unchanged writer already wrote. Each successor's
    /// entries may sit BELOW this revision (skipped, kept from an older
    /// version), so a tombstone written here would mask them: the writer
    /// re-asserts every successor's entries at its own revision, and ends this
    /// version's values at the first successor's (see [`write_successors`]).
    OutOfOrder {
        prior: Option<&'a Node>,
        successors: &'a [crate::mvcc_read::StoredVersion],
    },
    /// A create: the id provably has no live version on the branch.
    NoPrior,
}

/// [`Baseline`], owning its node (what [`resolve_baseline`] returns).
#[derive(Debug, Clone)]
pub enum OwnedBaseline {
    Predecessor(Node),
    Full(Option<Node>),
    OutOfOrder {
        prior: Option<Node>,
        successors: Vec<crate::mvcc_read::StoredVersion>,
    },
    NoPrior,
}

impl OwnedBaseline {
    pub fn as_ref(&self) -> Baseline<'_> {
        match self {
            Self::Predecessor(node) => Baseline::Predecessor(node),
            Self::Full(node) => Baseline::Full(node.as_ref()),
            Self::OutOfOrder { prior, successors } => Baseline::OutOfOrder {
                prior: prior.as_ref(),
                successors,
            },
            Self::NoPrior => Baseline::NoPrior,
        }
    }

    /// Whether unchanged entries will be skipped.
    pub fn skips(&self) -> bool {
        matches!(self, Self::Predecessor(_))
    }
}

/// What one write staged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeltaCounts {
    /// Live entries put.
    pub puts: usize,
    /// Tombstones written.
    pub tombstones: usize,
    /// Unchanged entries left at the predecessor's revision.
    pub skipped: usize,
}

/// Where the writes go.
#[derive(Clone, Copy)]
pub struct PropertyIndexTarget<'a> {
    pub db: &'a DB,
    pub cf: &'a ColumnFamily,
}

impl<'a> PropertyIndexTarget<'a> {
    pub fn from_db(db: &'a DB) -> Result<Self> {
        Ok(Self {
            db,
            cf: crate::cf_handle(db, crate::cf::PROPERTY_INDEX)?,
        })
    }
}

/// THE property-index writer. `in_place` marks a write at a REUSED revision
/// (`versionable=false`): with resolved targets a write then lands at
/// `max(revision, newest entry of its group)` — see [`in_place`].
pub fn write_property_index_delta(
    batch: &mut WriteBatch,
    target: PropertyIndexTarget<'_>,
    ctx: &IndexCtx<'_>,
    baseline: Baseline<'_>,
    new: &Node,
    revision: &HLC,
    in_place: InPlace<'_>,
) -> Result<DeltaCounts> {
    let old = match baseline {
        Baseline::Predecessor(old)
        | Baseline::Full(Some(old))
        | Baseline::OutOfOrder {
            prior: Some(old), ..
        } => entries_of(old),
        Baseline::Full(None) | Baseline::OutOfOrder { prior: None, .. } | Baseline::NoPrior => {
            BTreeSet::new()
        }
    };
    let new_entries = entries_of(new);
    let mut counts = DeltaCounts::default();

    for entry in old.difference(&new_entries) {
        let at = write_revision(entry, revision, in_place);
        batch.put_cf(target.cf, entry.key(ctx, &at, &new.id), TOMBSTONE);
        counts.tombstones += 1;
    }
    let skip = matches!(baseline, Baseline::Predecessor(_));
    for entry in &new_entries {
        if skip && old.contains(entry) {
            counts.skipped += 1;
            continue;
        }
        let at = write_revision(entry, revision, in_place);
        batch.put_cf(target.cf, entry.key(ctx, &at, &new.id), new.id.as_bytes());
        counts.puts += 1;
    }
    if let Baseline::OutOfOrder { successors, .. } = baseline {
        write_successors(batch, target.cf, ctx, new, &new_entries, successors);
    }
    if counts.skipped > 0 {
        SKIPPED.fetch_add(counts.skipped as u64, Ordering::Relaxed);
    }
    Ok(counts)
}

/// A write at R below stored successors (oldest first): end R's values the
/// first successor does not hold at ITS revision (a delete ends all of them),
/// and re-put every live successor's entries at its own revision, so nothing
/// written at R masks an entry a successor kept from below R.
fn write_successors(
    batch: &mut WriteBatch,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    new: &Node,
    new_entries: &BTreeSet<PropertyEntry>,
    successors: &[crate::mvcc_read::StoredVersion],
) {
    let Some((first_rev, first)) = successors.first() else {
        return;
    };
    let first_entries = first.as_ref().map(entries_of).unwrap_or_default();
    for entry in new_entries.difference(&first_entries) {
        batch.put_cf(cf, entry.key(ctx, first_rev, &new.id), TOMBSTONE);
    }
    reassert_successors(batch, cf, ctx, &new.id, successors);
}

/// Re-put every live successor's entries at its own revision — after a write
/// at a revision BELOW `successors` (oldest first) staged tombstones there.
/// A successor written with skips keeps unchanged entries below that
/// revision; this is what stops the lower write's tombstones masking them.
/// Shared by the out-of-order writer above and the delete tombstoner (a
/// delete applied below a stored version, e.g. a replicated delete older
/// than a local skip-written update).
pub(crate) fn reassert_successors(
    batch: &mut WriteBatch,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    successors: &[crate::mvcc_read::StoredVersion],
) {
    for (at, version) in successors {
        if let Some(version) = version {
            for entry in entries_of(version) {
                batch.put_cf(cf, entry.key(ctx, at, node_id), node_id.as_bytes());
            }
        }
    }
}

/// Tombstone, at `revision`, every entry `old` indexes that `new` does not —
/// the superseded-version half of the writer, for callers that write the new
/// version's entries elsewhere or not at all (an out-of-order apply ending the
/// older version's values at the newer revision; a merge's union tombstones).
pub fn tombstone_superseded_entries(
    batch: &mut WriteBatch,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    old: &Node,
    new: &Node,
    revision: &HLC,
) -> usize {
    let new_entries = entries_of(new);
    let mut written = 0;
    for entry in entries_of(old).difference(&new_entries) {
        batch.put_cf(cf, entry.key(ctx, revision, &old.id), TOMBSTONE);
        written += 1;
    }
    written
}

/// Tombstone every entry `node` indexes, at `revision` (a delete).
pub fn tombstone_all_entries(
    batch: &mut WriteBatch,
    cf: &ColumnFamily,
    ctx: &IndexCtx<'_>,
    node: &Node,
    revision: &HLC,
) -> usize {
    let entries = entries_of(node);
    for entry in &entries {
        batch.put_cf(cf, entry.key(ctx, revision, &node.id), TOMBSTONE);
    }
    entries.len()
}

fn write_revision(entry: &PropertyEntry, revision: &HLC, in_place: InPlace<'_>) -> HLC {
    match in_place {
        InPlace::Reused(Some(targets)) => targets.get(entry).unwrap_or(*revision),
        InPlace::No | InPlace::Reused(None) => *revision,
    }
}
