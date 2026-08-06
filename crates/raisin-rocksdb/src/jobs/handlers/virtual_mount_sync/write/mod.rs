//! The write drain: local edits going OUT, ahead of remote changes coming in.
//!
//! # Why this lives inside the sync run
//!
//! The drain is the FIRST step of [`super::phases::run_phases`], not a job of
//! its own, and it reuses everything the run has already paid for: the
//! per-mount lease, the capabilities probe, the decrypted credential, the
//! mapper writeback probe, the loaded [`SyncIndex`](super::SyncIndex) and the
//! batcher. That is four savings and one correctness property.
//!
//! The correctness property is the ordering. Writeback and a sync pass must not
//! interleave — both mutate `__etag`, and a sync's etag skip-write against a
//! stale etag would silently drop a push's stamp-back. Draining under the same
//! lease, before the read phases, also means a conflict is evaluated against
//! post-push state rather than against a remote we are about to change anyway.
//!
//! A standalone job would additionally starve: `check::is_due` runs backfill
//! chunks back to back, so a mount mid-import would never release the lease for
//! it.
//!
//! # Scope
//!
//! `state_only` only: a declared allow-list of fields on an existing remote
//! object, pushed through the adapter's `update`. No creates, no deletes, no
//! moves — none of which can be made safe without the blast-radius rails, which
//! ship with the first delete and not before.

mod plan;
mod push;
mod verdict;

use super::*;

pub(super) use plan::{resolve as resolve_mode, WriteMode};
pub(super) use verdict::writeback_verdict;

/// Outcome of one drain, for logging and for the run record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DrainStats {
    pub pushed: usize,
    pub skipped: usize,
    pub failed: usize,
    /// Candidates whose remote object the adapter reported as no longer there.
    ///
    /// Counted apart from both `pushed` and `skipped` on purpose. It is not a
    /// push (nothing reached the provider and nothing was baselined) and it is
    /// not a converged no-op (the edit is still pending, and always will be for
    /// this external id). Folding it into either is what let a `null` result be
    /// reported as a completed push.
    pub gone: usize,
    pub first_error: Option<String>,
    /// The wall-clock budget ran out with candidates still pending.
    ///
    /// Recorded rather than inferred: a drain that stops halfway leaves the
    /// remaining edits pending and looks, from the outside, exactly like a drain
    /// that had nothing to do. That silence is the same class of invisibility as
    /// a sync that never renews its lease.
    pub truncated: bool,
    /// An operator's Stop landed mid-drain and the rest was abandoned.
    pub stopped: bool,
    /// Candidates never attempted because the drain ended early.
    ///
    /// Only ever non-zero alongside `truncated` or `stopped`. Carried out of the
    /// drain on [`MountState::last_drain`] because it is the one number that
    /// separates a mount that is caught up from one that is falling behind, and
    /// a clean `outcome: ok` says nothing about it.
    pub pending: usize,
}

impl DrainStats {
    /// The operator-facing receipt this drain leaves on the mount.
    fn summary(&self) -> DrainSummary {
        DrainSummary {
            pushed: self.pushed as u64,
            pending: self.pending as u64,
            gone: self.gone as u64,
            failed: self.failed as u64,
            truncated: self.truncated,
            stopped: self.stopped,
        }
    }
}

/// How many candidates the drain pushes between `stop_requested` reads.
///
/// Not per item: [`super::stop_requested`] is a fresh node read on the config
/// branch, and the read phases pay it once per PAGE for that reason. Checked on
/// the first iteration too (`i % N == 0` includes 0), so a small drain honours a
/// stop instead of pushing its whole candidate list first.
const STOP_CHECK_EVERY: usize = 20;

/// Push whatever local edits this mount owes the provider.
///
/// **Never fails the run.** A provider that refuses writes must not stop reads:
/// a mount whose writeback is broken should still bring new mail in, and the
/// operator should learn about the failure from `writeback_last_error` rather
/// than from a mount that has gone dark. The one exception is
/// [`AdapterError::AuthExpired`], which stops the drain early because every
/// subsequent call would fail the same way — the read phase then hits it too
/// and moves the mount to `auth_required`, which is the right place for that
/// decision to be made once.
pub(super) async fn drain(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    batcher: &mut batch::SyncBatcher<'_>,
    mode: &WriteMode,
) -> DrainStats {
    let fields = match mode {
        WriteMode::Off => return DrainStats::default(),
        WriteMode::Refused(_) => return DrainStats::default(),
        WriteMode::StateOnly(fields) => fields.clone(),
    };

    // Bounded by the mount's own per-sync item cap: a drain that discovered a
    // six-figure divergence must not spend the whole wall-clock budget on it
    // and leave no room for the read phase. What is left over is picked up by
    // the next run, in a stable order.
    let limit = ctx.mount.sync_config.max_items_per_sync.max(1) as usize;
    let candidates = plan::candidates(batcher.virtual_nodes(), &fields, limit);
    if candidates.is_empty() {
        return DrainStats::default();
    }

    let mut stats = DrainStats::default();
    for (i, candidate) in candidates.iter().enumerate() {
        // Both checks happen BEFORE the push, never after it: they decide
        // whether to START another provider call, and a call already made
        // cannot be taken back. Breaking (rather than returning) is what keeps
        // the flush below on the path — whatever was staged so far must still
        // land ahead of the read phases.
        //
        // Everything not reached stays pending exactly as it was: no stamp, no
        // `__pushed_state`, so the next run re-nominates it. The candidate list
        // is sorted by `external_id` (see `plan::candidates`), so that next run
        // resumes deterministically rather than re-rolling an arbitrary subset.
        if ctx.out_of_time(Utc::now().timestamp()) {
            stats.truncated = true;
            stats.pending = candidates.len() - i;
            tracing::info!(
                mount_id = %ctx.scope.mount_id,
                pushed = stats.pushed,
                pending = candidates.len() - i,
                "wall-clock budget spent during writeback; ending the drain cleanly. The \
                 remaining edits stay pending and the next run picks them up."
            );
            break;
        }
        if i % STOP_CHECK_EVERY == 0 && super::stop_requested(ctx).await {
            stats.stopped = true;
            stats.pending = candidates.len() - i;
            tracing::info!(
                mount_id = %ctx.scope.mount_id,
                pushed = stats.pushed,
                pending = candidates.len() - i,
                "stop requested; ending the writeback drain with edits still pending"
            );
            break;
        }
        match push::push_one(ctx, batcher, candidate, &fields).await {
            Ok(push::Pushed::Sent) => stats.pushed += 1,
            Ok(push::Pushed::Skipped) => stats.skipped += 1,
            Ok(push::Pushed::Gone) => stats.gone += 1,
            Err(e) => {
                stats.failed += 1;
                if stats.first_error.is_none() {
                    stats.first_error = Some(e.to_string());
                }
                tracing::warn!(
                    mount_id = %ctx.scope.mount_id,
                    external_id = %candidate.external_id,
                    error = %e,
                    "writeback push failed; leaving the edit pending"
                );
                if matches!(e, AdapterError::AuthExpired) {
                    break;
                }
            }
        }
        // Renewed after EVERY item, not at a page boundary like the read
        // phases. One `update` against a throttled provider can take tens of
        // seconds, so a drain of a few hundred edits can outlive the lease
        // between two boundaries — and a lease that lapses mid-drain lets a
        // second run start beside this one, with both writing `__etag` for the
        // same items. It is a no-op when the locks subsystem is disabled.
        ctx.renew_lease().await;
    }

    // Flush before returning: the read phases are about to write `__etag` for
    // these very items, and an unflushed stamp would be applied AFTER them —
    // stamping a push's etag over a newer remote one, which is the one way this
    // design can lose a remote change.
    if let Err(e) = batcher.flush().await {
        stats.failed += 1;
        if stats.first_error.is_none() {
            stats.first_error = Some(e.to_string());
        }
        tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            error = %e,
            "writeback stamp-back flush failed"
        );
    }

    if stats.pushed > 0 || stats.failed > 0 || stats.gone > 0 {
        tracing::info!(
            mount_id = %ctx.scope.mount_id,
            pushed = stats.pushed,
            skipped = stats.skipped,
            failed = stats.failed,
            gone = stats.gone,
            truncated = stats.truncated,
            stopped = stats.stopped,
            "virtual mount writeback drain finished"
        );
    }
    // Only a failure speaks here. The verdict written before the run is what
    // says whether writeback is possible at all, and overwriting it on a clean
    // drain would erase the reason a REFUSED mount is refused.
    //
    // A `gone` counts as one. Nothing failed in the mechanical sense — the call
    // was made and answered — but an edit the user made is not at the provider
    // and never will be under this external id. Reporting a clean run would be
    // the same silence the stamp-on-null produced. A real error still wins.
    if stats.first_error.is_some() {
        state.writeback_last_error = stats.first_error.clone();
    } else if stats.gone > 0 {
        state.writeback_last_error = Some(format!(
            "{} pending edit(s) could not be pushed: the provider no longer has \
             the object under the id this mount recorded. They stay pending until \
             the item is re-imported under its current id.",
            stats.gone
        ));
    }
    // Written on every drain that had candidates, clean ones included: a summary
    // left over from an earlier run would read as the current state, and "120
    // pushed, 380 pending" going stale is exactly the wrong thing to leave on a
    // mount that has since caught up.
    state.last_drain = Some(stats.summary());
    stats
}
