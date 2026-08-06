//! Durable change detection: the `RevisionMeta` watermark walk.
//!
//! # Why this exists at all
//!
//! The drain's divergence check (`write::plan::candidates`) sees local edits to
//! nodes that are still there. It cannot see anything else, and three of the
//! four things it misses are not fixable at that layer:
//!
//! * **A delete is structurally invisible.** The node is gone from the
//!   workspace, gone from `SyncIndex.by_external`, and there is no per-mount
//!   manifest of owned external ids. A sweep over live nodes cannot distinguish
//!   "deleted" from "never existed".
//! * **A crash loses in-process state.** Anything the event bus captured and had
//!   not drained is gone with the process.
//! * **`updated_by != SYNC_ACTOR` does not work** as a change predicate. The
//!   materializer sets `AuthContext::system_as(SYNC_ACTOR)`, so it is true for
//!   every node the sync itself wrote. `RevisionMeta.actor` is the signal that
//!   IS correct — the commit path records the raw actor — so echo suppression
//!   belongs at COMMIT granularity, here, and not at node granularity.
//!
//! # What this does and deliberately does not do
//!
//! It DETECTS and queues; it does not decide. The walk records everything a
//! push will need ([`PendingDelete`] — provider id, etag, path, and the
//! bulk-origin flag) and stops there. Whether any of it is pushed is
//! `write::guard`'s call, made later, against the rails.
//!
//! The split is not tidiness: detection is the half that cannot be
//! reconstructed later. Once the watermark has passed a delete and its MVCC
//! pre-image has been garbage collected, nothing anywhere remembers the node.
//! The rails, by contrast, can be re-evaluated on every drain — which is
//! exactly what makes a soft block releasable.

use std::sync::Arc;

use chrono::Utc;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{RevisionRepository, Storage};

use super::super::config::{
    MountConfig, MountState, PendingDelete, ReconcileSummary, BULK_REVISION_THRESHOLD,
    MAX_PENDING_DELETES, SYNC_ACTOR,
};
use super::preimage::recover_delete;
use super::relocate::{probe, Relocation};
use crate::RocksDBStorage;

/// Revisions read per walk.
///
/// Bounded for the same reason `max_items_per_sync` is: a mount enabling
/// writeback on a busy repository must not spend an unbounded amount of time in
/// one pass. What is left over is picked up by the next pass, from the
/// watermark, in commit order — so a bound here costs latency and never
/// correctness.
pub(crate) const RECONCILE_REVISION_LIMIT: usize = 500;

/// Node reads one pass will spend probing for a move across the mount boundary.
///
/// A move is not visible in `RevisionMeta` — `NodeChangeInfo` carries a node id
/// and an operation, never a path — so the only way to see one is to read the
/// node and compare. That is one point read per non-echo change in the mount's
/// workspace, which is cheap per unit and unbounded in aggregate, so it gets a
/// budget like everything else in this walk.
///
/// Exhausting it stops the pass WITHOUT advancing the watermark, so the
/// unexamined revisions are walked again rather than skipped — the same
/// back-pressure `MAX_PENDING_DELETES` uses, and for the same reason: a move
/// that is never examined is a node that stays invisibly half-mounted forever.
/// The first revision of a pass is always finished regardless, or a single
/// revision wider than the budget could never make progress at all.
pub(crate) const RELOCATE_PROBE_LIMIT: usize = 2_000;

/// What one walk found. Counters for the operator; the deletes themselves are
/// written onto [`MountState::writeback_pending_deletes`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Findings {
    /// The mount had no watermark and this pass established one without walking.
    pub initialized: bool,
    pub scanned: usize,
    pub echoes: usize,
    pub local_changes: usize,
    pub deletes_found: usize,
    /// Mount-owned nodes found outside the mount path and detached.
    pub detached: usize,
    /// Foreign nodes found inside the mount path and refused adoption.
    pub rejected: usize,
    /// More to read: the revision limit was reached, or the pending-delete cap
    /// stopped the walk before it.
    pub truncated: bool,
}

impl Findings {
    fn summary(&self) -> ReconcileSummary {
        ReconcileSummary {
            at: Utc::now().timestamp(),
            scanned: self.scanned as u64,
            echoes: self.echoes as u64,
            local_changes: self.local_changes as u64,
            deletes_found: self.deletes_found as u64,
            detached: self.detached as u64,
            rejected: self.rejected as u64,
            truncated: self.truncated,
        }
    }
}

/// Walk this mount's repository from its watermark, recording what it finds.
///
/// Mutates `state` (watermark, pending deletes, summary) and leaves persisting
/// it to the caller — the caller is the one holding the mount's lease and the
/// one that knows the `state_seq` it read.
pub(crate) async fn reconcile_mount(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    mount: &MountConfig,
    state: &mut MountState,
    limit: usize,
) -> Result<Findings> {
    // Only a mount that can actually PUSH a delete may queue one.
    //
    // `write::drain` drains `writeback_pending_deletes` in `Mirror` mode and
    // nowhere else (`write/mod.rs`: `if let Some(plan) = mirror`). For a
    // `state_only` or `submit` mount the queue therefore has no consumer at all,
    // and an append-only queue behind a cap is a time bomb: once it passes
    // `MAX_PENDING_DELETES` the back-pressure break below stops advancing the
    // watermark, and — because nothing can ever shorten the queue — stops
    // advancing it FOREVER. Durable change detection for that mount is then
    // dead: no local edits, no boundary probes, no deletes, and only a `warn!`
    // that promises a backlog will drain when it cannot. The flagship
    // configuration (a `state_only` /mail/inbox) reaches that state after 501
    // locally-deleted messages.
    //
    // Not recording is also the safer reading on its own terms. A `state_only`
    // mount promises never to remove anything at the provider; keeping a year of
    // its deletes on the off-chance the mode is later flipped to `mirror` would
    // mean that flip retro-deletes remote objects for nodes deleted under a
    // promise that it would not.
    //
    // A mount whose mode is `mirror` but whose adapter currently falls short
    // still records: that is `WriteMode::Refused`, an adapter that may come back,
    // and it is the case `WriteConfig::wants_any_write` was widened for.
    let record_deletes = mount.write_config.wants_mirror();
    if !record_deletes && !state.writeback_pending_deletes.is_empty() {
        tracing::info!(
            mount_id = %mount.mount_id,
            discarded = state.writeback_pending_deletes.len(),
            mode = %mount.write_config.mode,
            "discarding queued deletes: this mount's write mode has no delete path, \
             so nothing would ever drain them"
        );
        state.writeback_pending_deletes.clear();
    }

    let Some(after) = watermark(storage, tenant, repo, state).await? else {
        // First pass: the watermark is now the repository head and there is
        // nothing to walk. See `MountState::writeback_revision` for why history
        // is not replayed.
        let findings = Findings {
            initialized: true,
            ..Default::default()
        };
        state.last_reconcile = Some(findings.summary());
        return Ok(findings);
    };

    let revisions = storage
        .revisions()
        .list_revisions_since(tenant, repo, &after, limit)
        .await?;

    let mut findings = Findings {
        scanned: revisions.len(),
        truncated: revisions.len() >= limit,
        ..Default::default()
    };

    // Spent by the boundary probe below; see `RELOCATE_PROBE_LIMIT`.
    let mut probes = 0usize;

    for (revision_index, meta) in revisions.iter().enumerate() {
        // Echo suppression at COMMIT granularity. `RevisionMeta.actor` is the
        // raw transaction actor, which the materializer sets to `SYNC_ACTOR`;
        // the node's own `updated_by` is NOT this signal (it comes from the auth
        // context and reads `virtual-mount-sync` for the same writes, but also
        // for the mount-state writes, and it is per-node rather than per-commit).
        if meta.actor == SYNC_ACTOR {
            findings.echoes += 1;
            state.writeback_revision = Some(meta.revision.to_string());
            continue;
        }
        // A mount materializes into ONE branch. A commit on any other branch —
        // a fork, a publish target, the config branch itself — is not this
        // mount's business, and pushing it would export a branch's private state
        // to the provider.
        if meta.branch != mount.target_branch {
            state.writeback_revision = Some(meta.revision.to_string());
            continue;
        }

        // Checked BEFORE the revision is processed, and never before the
        // first one: stopping here leaves the watermark below this revision, so
        // it is re-read next pass with a full budget. Stopping mid-revision
        // would advance nothing either, but would re-do the probes it had
        // already paid for.
        if revision_index > 0 && probes >= RELOCATE_PROBE_LIMIT {
            findings.truncated = true;
            tracing::info!(
                mount_id = %mount.mount_id,
                probes,
                "reconcile stopped at the boundary-probe budget; the watermark stays put \
                 and the next pass resumes from this revision"
            );
            break;
        }

        let bulk = meta.changed_nodes.len() > BULK_REVISION_THRESHOLD;
        let mut deletes = Vec::new();
        let mut local_changes = 0usize;

        for change in &meta.changed_nodes {
            if change.workspace != mount.target_workspace {
                continue;
            }
            // A translation overlay is not the node, and there is no provider
            // shape for one. Counting it as a local change would nominate the
            // node forever.
            if change.translation_locale.is_some() {
                continue;
            }
            local_changes += 1;
            if change.operation != ChangeOperation::Deleted {
                // The mount-boundary probe. A node that has left the mount path
                // is invisible to the drain — the sync index is a path-prefix
                // scan — so this walk is the only place it can be seen at all.
                probes += 1;
                match probe(storage, tenant, repo, mount, change).await {
                    Some(Relocation::Out) => findings.detached += 1,
                    Some(Relocation::In) => findings.rejected += 1,
                    None => {}
                }
                continue;
            }
            if !record_deletes {
                continue;
            }
            if let Some(pending) =
                recover_delete(storage, tenant, repo, mount, change, &meta.revision, bulk).await?
            {
                deletes.push(pending);
            }
        }

        // Back-pressure, not truncation: stop WITHOUT advancing the watermark so
        // this revision is walked again next pass. Dropping the overflow instead
        // would lose the deletes permanently — this walk is the only thing that
        // can see them, and it only gets one chance per revision.
        if state.writeback_pending_deletes.len() + deletes.len() > MAX_PENDING_DELETES {
            findings.truncated = true;
            tracing::warn!(
                mount_id = %mount.mount_id,
                pending = state.writeback_pending_deletes.len(),
                cap = MAX_PENDING_DELETES,
                "reconcile stopped at the pending-delete cap; the watermark stays put and \
                 these revisions are re-walked once the backlog drains"
            );
            break;
        }

        findings.local_changes += local_changes;
        for pending in deletes {
            // The walk is resumable and a revision can be re-read (an overflow
            // break, a state write that lost its CAS), so recording the same
            // delete twice has to be impossible rather than unlikely.
            if !state
                .writeback_pending_deletes
                .iter()
                .any(|p| p.node_id == pending.node_id)
            {
                state.writeback_pending_deletes.push(pending);
                findings.deletes_found += 1;
            }
        }
        state.writeback_revision = Some(meta.revision.to_string());
    }

    state.last_reconcile = Some(findings.summary());
    Ok(findings)
}

/// This mount's watermark, establishing one at the repository head if it has
/// none. `Ok(None)` means it was just established and there is nothing to walk.
async fn watermark(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    state: &mut MountState,
) -> Result<Option<HLC>> {
    if let Some(raw) = state.writeback_revision.as_deref() {
        match raw.parse::<HLC>() {
            Ok(hlc) => return Ok(Some(hlc)),
            Err(e) => {
                // Re-establishing at the head is the only safe reading: treating
                // an unparseable watermark as zero would replay the entire
                // repository history, and every delete in it, on a mount that
                // has been writing happily.
                tracing::warn!(
                    watermark = %raw,
                    error = %e,
                    "unparseable writeback watermark; re-establishing at the repository head"
                );
            }
        }
    }
    let head = storage
        .revisions()
        .list_revisions(tenant, repo, 1, 0)
        .await?
        .into_iter()
        .next()
        .map(|m| m.revision)
        // A repository with no revisions at all: start below everything rather
        // than at `now()`, or the first commits made after this pass are missed.
        .unwrap_or_else(|| HLC::new(0, 0));
    state.writeback_revision = Some(head.to_string());
    Ok(None)
}
