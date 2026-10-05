//! Run-collapse GC (plan Phase 9): shrink redundant index history without
//! changing any answer at any revision.
//!
//! # What it removes
//!
//! Within each GC group of an index column family (PROPERTY_INDEX
//! `(tag, name, value, node)`, and the analogous REFERENCE, ORDERED_CHILDREN,
//! UNIQUE and COMPOUND groups — the groups [`super::layout`] already defines),
//! versions are walked newest to oldest, and a version whose next-older version
//! has an IDENTICAL state — live with the same value bytes, or a tombstone
//! after a tombstone — is deleted. The OLDEST entry of each run is kept. Every
//! reader decides a group by its newest entry at or below the read revision,
//! so for any revision the answer is the run's state either way. Keeping the
//! oldest is also what `index.skip_unchanged` produces: an entry a skip kept
//! lives at the revision of the version that first wrote it.
//!
//! # Why it is not "lossless at will"
//!
//! The decision assumes nothing is ever inserted between the two versions.
//! Three things insert below existing entries, and each is closed off:
//!
//! - **Out-of-order replication.** Only entries STRICTLY BELOW a causal-
//!   stability watermark are deleted ([`watermark`]): on a single node the
//!   branch HEAD, bounded by `min_age` so a transaction that allocated its
//!   revision before HEAD advanced has committed. There is no trustworthy
//!   cluster-wide watermark yet (peer acks are logged, never persisted), so in
//!   cluster mode collapse REFUSES to run.
//! - **Repairs** that write tombstones at historical revisions. A CF is
//!   collapsed on a branch only after every repair that corrects it has
//!   completed there on this node ([`prereq`]); the checkpoint-ingest hook
//!   resets those records, so an ingest re-arms the refusal.
//! - **Concurrent rebuilds, repairs and branch copies.** Each slice holds the
//!   per-`(branch, CF)` exclusion (`management::cf_exclusion`); a branch copy
//!   also re-arms the prerequisite records of its target ([`prereq`]).
//! - **Late commits.** A transaction allocates its revision at its first write
//!   and may commit much later; the watermark is clamped below every revision
//!   still allocated to an open transaction (`transaction::inflight`).
//!
//! And two things DELETE or REWRITE the entries a decision was made from:
//!
//! - **Retention GC** deletes the opposite end of the same pair (it keeps the
//!   newest version at or below its cutoff). It holds the whole database
//!   against collapse slices while it runs (`cf_exclusion::enter_pruner`).
//! - **In-place writers** rewrite keys at an existing revision. A slice holds
//!   the branch's in-place guard from its reads through its commit, and
//!   re-reads anything remembered from an earlier slice ([`pass`]).
//!
//! # How it runs
//!
//! As the `collapse_runs` index repair: admin-triggered through the fan-out
//! endpoint, never at boot, gated by `RocksDBConfig::history_gc_collapse_runs`
//! (default OFF), refused on any node with replication configured. One branch
//! at a time, one CF at a time, streamed through the repair's bounded writer —
//! slices bounded by deletes, keys scanned and time, committed with a
//! resumable cursor, a rate limit on bytes read and written, a disk precheck
//! of twice each CF, and a dry run that reports the bytes it would reclaim. Space only returns after compaction,
//! and after backup retention rotates (checkpoints hard-link SSTs).

mod detector;
mod pass;
mod prereq;
#[cfg(test)]
mod tests;
mod watermark;

pub use prereq::{rearm_after_branch_copy, required_repairs};
pub use watermark::collapse_watermark;

use super::layout::{GcTarget, GC_TARGETS};
use crate::cf;
use crate::management::async_indexing::repair::{BoundedWriter, RepairOptions};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// The column families collapse may touch, in the order it walks them.
///
/// COMPOUND is here because Phase 8 step 2 has landed: the compound writer
/// derives tombstones at the write revision and no longer overwrites keys in
/// place, so its history has the same shape as the others.
///
/// Never TRANSLATION_DATA: a node overlay's liveness also depends on node
/// deletes between versions (`translation_read::ended_by_node_delete`), so
/// keeping the OLDER of two identical versions can move one below a delete
/// that ends it.
pub const COLLAPSE_CFS: &[&str] = &[
    cf::PROPERTY_INDEX,
    cf::REFERENCE_INDEX,
    cf::ORDERED_CHILDREN,
    cf::UNIQUE_INDEX,
    cf::COMPOUND_INDEX,
];

/// How long a slice waits for an inserter to release a `(branch, CF)` before
/// the run stops, resumable, as `busy`.
const BUSY_PATIENCE: Duration = Duration::from_secs(2);

/// One CF's outcome on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CfCollapseCounts {
    /// Versioned keys looked at.
    pub scanned: u64,
    /// Versions deleted (dry run: that would be).
    pub deleted: u64,
    /// Logical bytes (key + value) of those versions: what compaction can
    /// return, once backup retention has rotated.
    pub bytes_reclaimable: u64,
    /// Redundant versions kept because they are at or above the watermark.
    pub kept_above_watermark: u64,
    /// ORDERED_CHILDREN: redundant versions kept because another child's
    /// entry lies between them and their twin (deleting would reorder).
    #[serde(default)]
    pub kept_interleaved: u64,
    /// Groups forgotten to bound memory on a hot chunk (their redundant
    /// versions, if any, are kept).
    #[serde(default)]
    pub forgotten_groups: u64,
    /// Versions remembered from an earlier slice that had been rewritten
    /// before their slice could delete them, and were kept.
    #[serde(default)]
    pub skipped_changed: u64,
    /// The pass reached the end of the CF.
    pub completed: bool,
}

/// What a collapse run did on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollapseCounts {
    /// The watermark used; only versions strictly below it were eligible.
    pub watermark: Option<String>,
    pub column_families: BTreeMap<String, CfCollapseCounts>,
    /// `"{cf}: {reason}"` for each CF refused on this branch.
    pub refused: Vec<String>,
    /// CFs a running inserter kept busy; the run stopped there, resumable.
    pub busy: Vec<String>,
}

/// The branch a collapse runs on.
pub(crate) struct Scope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub node_id: &'a str,
    pub watermark: HLC,
}

/// The CFs `options` selects, in walk order. Unknown names are an error.
pub fn selected_cfs(options: &RepairOptions) -> Result<Vec<&'static str>> {
    let Some(wanted) = &options.collapse_cfs else {
        return Ok(COLLAPSE_CFS.to_vec());
    };
    for name in wanted {
        if !COLLAPSE_CFS.contains(&name.as_str()) {
            return Err(Error::Validation(format!(
                "collapse_runs: '{name}' is not a collapsible column family (expected one of {})",
                COLLAPSE_CFS.join(", ")
            )));
        }
    }
    Ok(COLLAPSE_CFS
        .iter()
        .copied()
        .filter(|c| wanted.iter().any(|w| w == c))
        .collect())
}

/// Refuse the whole run, before anything is deleted, unless each selected CF
/// has twice its size free on the data volume.
pub fn precheck_headroom(db: &DB, options: &RepairOptions) -> Result<()> {
    if options.dry_run || !options.check_headroom {
        return Ok(());
    }
    for cf_name in selected_cfs(options)? {
        crate::management::async_indexing::repair::check_headroom_assuming(
            db,
            cf_name,
            options.free_bytes_override,
        )?;
    }
    Ok(())
}

fn target(cf_name: &str) -> &'static GcTarget {
    GC_TARGETS
        .iter()
        .find(|t| t.cf == cf_name)
        .expect("every collapsible CF is a GC target")
}

/// The `collapse_runs` repair on one branch: derive the watermark from its
/// HEAD (refusing in cluster mode), then [`collapse_branch`]. A branch with no
/// HEAD has nothing to collapse.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collapse_branch_at_head(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    head: Option<HLC>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut CollapseCounts,
    options: &RepairOptions,
) -> Result<bool> {
    // A transaction that allocated its revision and has not committed yet can
    // still land entries at it: nothing at or above it is stable.
    let cap = match (
        options.collapse_watermark_cap,
        crate::transaction::oldest_inflight_revision(db),
    ) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let watermark = collapse_watermark(
        head,
        watermark::now_ms(),
        options.collapse_min_age,
        cap,
        options.cluster_mode,
    )?;
    let Some(watermark) = watermark else {
        return Ok(true);
    };
    let scope = Scope {
        tenant_id,
        repo_id,
        branch,
        node_id,
        watermark,
    };
    collapse_branch(db, &scope, writer, counts, options)
}

/// Collapse every selected CF of one branch. Returns whether every CF was
/// collapsed to the end (false: refused, busy, or stopped by the test hook).
pub(crate) fn collapse_branch(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut CollapseCounts,
    options: &RepairOptions,
) -> Result<bool> {
    counts.watermark = Some(scope.watermark.to_string());
    // Resume only the pass the persisted cursor belongs to; every other CF
    // starts from its beginning (idempotent, so a re-scan deletes nothing).
    let resume_pass = writer.state().pass.clone();
    let resume_cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|c| hex::decode(c).ok());
    let mut completed = true;
    for cf_name in selected_cfs(options)? {
        let pending = prereq::pending_repairs(
            db,
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.node_id,
            cf_name,
        )?;
        if !pending.is_empty() {
            counts.refused.push(format!(
                "{cf_name}: repair pending on this node: {}",
                pending.join(", ")
            ));
            completed = false;
            continue;
        }
        writer.begin_pass(cf_name);
        let start = (resume_pass == cf_name)
            .then(|| resume_cursor.clone())
            .flatten();
        let cf_counts = counts
            .column_families
            .entry(cf_name.to_string())
            .or_default();
        match pass::collapse_cf(
            db,
            scope,
            target(cf_name),
            writer,
            cf_counts,
            start,
            options,
        )? {
            pass::End::Done => cf_counts.completed = true,
            pass::End::Busy => {
                counts.busy.push(cf_name.to_string());
                return Ok(false);
            }
            pass::End::Stopped => return Ok(false),
        }
    }
    if !completed {
        // Refused somewhere: nothing to resume, the next run starts over.
        writer.clear_cursor();
        writer.commit("refused")?;
    }
    Ok(completed)
}
