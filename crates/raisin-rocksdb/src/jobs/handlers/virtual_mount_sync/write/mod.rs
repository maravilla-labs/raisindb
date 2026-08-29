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
//! Three modes:
//!
//! * `state_only` — a declared allow-list of fields on an existing remote
//!   object, pushed through the adapter's `update`.
//! * `mirror` — the same field-scoped update, plus DELETE propagation under a
//!   per-mount policy, behind the five blast-radius rails of §9. The rails ship
//!   in the same change as the first delete and never after it.
//! * `submit` — an OUTBOX, and the odd one out. Its members are commands rather
//!   than mirrors of remote objects, so it shares nothing with the two above: no
//!   divergence check, no field allow-list, no batcher, no conflict policy, and
//!   the OPPOSITE retry semantics. It gets its own drain ([`submit`]) for that
//!   last reason above all — a shared loop is how the read path's "unrecognized
//!   means transient, so retry" default would eventually reach a send, and a
//!   retried send is a duplicate email.
//!
//! Local CREATE propagation is in [`create`], and it has the provenance rule
//! this comment used to say it needed. A node under a mount path carrying no
//! `__external_id` is indistinguishable from ordinary user content — which
//! `SyncIndex.by_path` deliberately protects from the read side — so the mount
//! must name the node TYPES it may create, in `write_config.create_node_types`.
//! That list is EMPTY by default and has no fallback: a mirror mount that has
//! not opted in creates nothing, because the failure mode of guessing is
//! uploading a stranger's scratch file to a third party, which no later config
//! fix undoes.
//!
//! A MOVE is an `update` carrying the location field the adapter declared, and
//! nothing more — there is no `move` operation and no separate code path. What
//! `move_policy` decides is which fields travel ([`plan::FieldPlan`]): `push`
//! sends the location field with everything else, `detach` withholds it, and
//! `reject` withholds the whole update while it disagrees. In all three the
//! LOCAL move stands — the read path preserves an existing node's path and its
//! baseline on upsert — so the policy governs the provider and never fights the
//! local edit.
//!
//! A move ACROSS the mount boundary is a different event, handled by
//! [`relocate`] on the reconcile walk rather than here: the drain's index is
//! scoped to the mount path, so a node that has left it is not in the index to
//! nominate.

mod candidates;
/// Reading a local node's bytes for a push, and deciding whether they travel
/// inline or as a deferred transfer.
pub(crate) mod content;
// `pub(crate)` so `resolve.rs` can map the policy onto
// `MountScope::read_local_wins`; the enum itself stays `pub(crate)` too, so the
// materializer never grows a dependency on resolver plumbing.
pub(crate) mod conflict;
mod conflict_push;
mod create;
mod deletes;
mod fields;
mod guard;
pub(crate) mod pending;
mod pending_drain;
mod plan;
mod policy;
mod preimage;
mod push;
pub(super) mod reconcile;
mod reconcile_job;
pub(super) mod relocate;
mod stats;
mod submit;
mod submit_outcome;
mod submit_record;
mod submit_step;
/// Streaming deferred bytes to an adapter-supplied URL.
pub(crate) mod upload;
mod verdict;

use super::*;

pub(super) use conflict::{resolve_conflict_policy, ConflictPolicy};
pub(super) use fields::FieldPlan;
pub(super) use plan::{resolve as resolve_mode, MirrorPlan, SubmitPlan, WriteMode};

/// The run `mode` that means "drain only, read nothing".
///
/// One constant rather than a `"write"` literal in the four places that compare
/// against it: the dispatcher that starts such a run, [`super::phases`] which
/// stops after the drain, and [`super::finalize`] which must NOT stamp the read
/// scheduler's clocks for one. A typo in any of those turns a latency drain back
/// into a full sync, or — worse, and silently — into a run that postpones every
/// subsequent poll.
pub(super) const WRITE_ONLY_MODE: &str = "write";
pub(super) use pending_drain::drain_pending;
pub(super) use reconcile_job::run_write_reconcile;
pub(super) use stats::DrainStats;
pub(super) use verdict::writeback_verdict;

/// Whether this mount is standing off after a configuration failure, and until
/// when.
///
/// One helper because all three drains — the scheduled one, the page-boundary
/// one and the outbox — must make no provider call at all while the window is
/// open, and three hand-written copies of the same comparison is how one of
/// them ends up keeping calling. Each caller keeps its OWN log line and early
/// return: what "skipped" means differs per drain, and only the caller can say
/// it.
///
/// Deliberately NOT a combined "may this mount write?" predicate. The other
/// guards around it (`writeback_blocked`, `force_rewrite`, the wall-clock
/// budget) are asymmetric between these callers on purpose, and folding them
/// together would silently give one caller a guard the other never wanted.
pub(super) fn standing_off(state: &MountState, now: i64) -> Option<i64> {
    state
        .writeback_retry_after
        .filter(|retry_after| now < *retry_after)
}

/// How many candidates the drain pushes between `stop_requested` reads.
///
/// Not per item: [`super::stop_requested`] is a fresh node read on the config
/// branch, and the read phases pay it once per PAGE for that reason. Checked on
/// the first iteration too (`i % N == 0` includes 0), so a small drain honours a
/// stop instead of pushing its whole candidate list first.
const STOP_CHECK_EVERY: usize = 20;

/// How long the drain stands off after a `config_error`.
///
/// Long enough that a permanently-broken mount costs one call per quarter hour
/// instead of one per second; short enough that an operator who fixes a scope
/// sees writeback resume while they are still looking at the screen.
const MISCONFIGURED_BACKOFF_SECS: i64 = 900;

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
    let (fields, mirror) = match mode {
        WriteMode::Off => return DrainStats::default(),
        WriteMode::Refused(_) => return DrainStats::default(),
        // An OUTBOX shares nothing with the update path below — no candidate
        // divergence check, no field allow-list, no batcher, no conflict policy
        // — because its members are commands rather than mirrors of remote
        // objects. It gets its own drain rather than a mode flag threaded
        // through this one; the two have opposite retry semantics, and a shared
        // loop is how the read path's "unrecognized means retry" default would
        // eventually reach a send.
        WriteMode::Submit(plan) => return submit::drain_submit(ctx, state, batcher, plan).await,
        WriteMode::StateOnly(fields) => (fields.clone(), None),
        WriteMode::Mirror(plan) => (plan.fields.clone(), Some(plan)),
    };
    let fields = &fields;

    // Standing off after a config error. Checked before ANY provider call and
    // before the delete rails, because the point is to make no calls at all.
    let now = Utc::now().timestamp();
    if let Some(retry_after) = standing_off(state, now) {
        tracing::debug!(
            mount_id = %ctx.scope.mount_id,
            retry_in_secs = retry_after - now,
            "writeback is standing off after a configuration failure; skipping \
             this drain. Edits stay pending and go out once the window elapses."
        );
        return DrainStats::default();
    }

    let mut stats = DrainStats::default();

    // BEFORE the deletes and before any provider call, because an unresolvable
    // conflict policy is not something to discover halfway through a drain. A
    // typo in `write_config.conflict` cannot be allowed to fall back to
    // `remote_wins` — that silently discards the very edits an operator who
    // typed `local_wins` was trying to keep — so the whole drain refuses and
    // says why. Reads are untouched, exactly as they are for a tripped rail.
    let policy = match resolve_conflict_policy(&ctx.mount) {
        Ok(policy) => policy,
        Err(reason) => {
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                reason = %reason,
                "this mount's conflict policy cannot be resolved; refusing to drain \
                 rather than falling back to a policy the operator did not choose"
            );
            state.writeback_status = Some("misconfigured".to_string());
            state.writeback_last_error = Some(reason);
            return stats;
        }
    };

    // DELETES FIRST, deliberately. The rails are evaluated before the run spends
    // any of its budget on updates, so a mount whose deletes are blocked parks
    // immediately rather than after a few hundred provider calls. It also means
    // a `blocked` verdict parks EVERY pending intent (§9.4), not only the
    // deletes — which is the whole point of a soft block: an operator reviewing
    // an unexpected mass delete should not have this mount making other outbound
    // changes in the meantime.
    //
    // What it must NOT do is stop the read phases. `run_phases` calls this and
    // carries on regardless; a mount refusing to destroy remote data must still
    // bring new data in, or the pragmatic response to a scare becomes "turn the
    // rail off".
    if let Some(plan) = mirror {
        let virtual_len = batcher.virtual_len();
        if deletes::drain_deletes(ctx, state, virtual_len, plan, &mut stats).await {
            state.last_drain = Some(stats.summary());
            return stats;
        }
    }

    // CREATES next, between the deletes and the updates.
    //
    // After the rails, because a blocked mount must make no outbound changes at
    // all while an operator is looking at it — creating remote objects during a
    // suspected mass-delete incident is exactly the "carry on regardless" the
    // soft block exists to prevent. Before the updates, because an adopted node
    // joins the run's index and the update pass then correctly finds it
    // converged, rather than leaving it to look pending until the next run.
    //
    // Skipped entirely unless the mount opted a node type in, so the scan it
    // needs — which the index cannot serve, since a never-synced node is not in
    // it — is not paid for by every mirror mount.
    if mirror.is_some() && ctx.mount.write_config.creates_locally() {
        create::drain_creates(
            ctx,
            batcher,
            state,
            &mut stats,
            mirror.is_some_and(|m| m.accepts_content),
        )
        .await;
    }

    // Bounded by the mount's own per-sync item cap: a drain that discovered a
    // six-figure divergence must not spend the whole wall-clock budget on it
    // and leave no room for the read phase. What is left over is picked up by
    // the next run, in a stable order.
    let limit = ctx.mount.sync_config.max_items_per_sync.max(1) as usize;
    let candidates = candidates::candidates(
        batcher.virtual_nodes(),
        fields,
        limit,
        mirror.is_some_and(|m| m.accepts_content),
    );
    // No field updates is NOT an early exit. This used to `return stats` here,
    // and it was the only return in the whole drain that came after work may
    // have been staged and before BOTH the flush below and the status block —
    // so one branch leaked two separate defects:
    //
    //  * `drain_creates` stages an ADOPT op (the `__external_id` the provider
    //    just minted) into the batcher. A `mode: "write"` latency drain creates
    //    a node, finds no update candidates, and returns without flushing — so
    //    the node never records the id it was created under, the next drain
    //    sees it as un-adopted, and creates ANOTHER remote object. Every drain,
    //    forever.
    //  * The badge below never ran, so a mount whose conflict had since been
    //    resolved stayed amber `conflict` with no Reason row to explain it —
    //    exactly the "keeps investigating, keeps finding it healthy" the status
    //    block's own comment says it exists to prevent.
    //
    // Falling through with an empty candidate list costs one skipped loop.
    let has_candidates = !candidates.is_empty();

    push_candidates(
        ctx,
        state,
        batcher,
        &candidates,
        fields,
        &policy,
        mirror.is_some_and(|m| m.accepts_content),
        &mut stats,
    )
    .await;

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

    record_outcome(&ctx.scope.mount_id, state, &stats, &policy, has_candidates);

    stats
}

/// The UPDATE phase: push each candidate, counting every outcome into `stats`.
///
/// A named function purely so [`drain`]'s body reads as its five ordered phases;
/// it stays in this file because the ORDER of those phases — deletes, then
/// creates, then this — is what the comments around the call sites argue for,
/// and splitting them across files is how a phase gets reordered with only half
/// the reasoning in view.
///
/// Every early exit RETURNS from this phase rather than from the drain, which
/// is what keeps the flush and the receipt on the path: whatever was staged
/// before the stop must still land ahead of the read phases.
async fn push_candidates(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    batcher: &mut batch::SyncBatcher<'_>,
    candidates: &[candidates::Candidate],
    fields: &FieldPlan,
    policy: &ConflictPolicy,
    accepts_content: bool,
    stats: &mut DrainStats,
) {
    for (i, candidate) in candidates.iter().enumerate() {
        // Both checks happen BEFORE the push, never after it: they decide
        // whether to START another provider call, and a call already made
        // cannot be taken back. Ending this phase (rather than the whole drain)
        // is what keeps the flush on the path — whatever was staged so far must
        // still land ahead of the read phases.
        //
        // Everything not reached stays pending exactly as it was: no stamp, no
        // `__pushed_state`, so the next run re-nominates it. The candidate list
        // is sorted by `external_id` (see `plan::candidates`), so that next run
        // resumes deterministically rather than re-rolling an arbitrary subset.
        if ctx.out_of_time(Utc::now().timestamp()) {
            stats.truncated = true;
            stats.pending += candidates.len() - i;
            tracing::info!(
                mount_id = %ctx.scope.mount_id,
                pushed = stats.pushed,
                pending = candidates.len() - i,
                "wall-clock budget spent during writeback; ending the drain cleanly. The \
                 remaining edits stay pending and the next run picks them up."
            );
            return;
        }
        if i % STOP_CHECK_EVERY == 0 && super::stop_requested(ctx).await {
            stats.stopped = true;
            stats.pending += candidates.len() - i;
            tracing::info!(
                mount_id = %ctx.scope.mount_id,
                pushed = stats.pushed,
                pending = candidates.len() - i,
                "stop requested; ending the writeback drain with edits still pending"
            );
            return;
        }
        match push::push_one(ctx, batcher, candidate, fields, policy, accepts_content).await {
            Ok(push::Pushed::Sent) => stats.pushed += 1,
            Ok(push::Pushed::Skipped) => stats.skipped += 1,
            Ok(push::Pushed::Gone) => stats.gone += 1,
            Ok(push::Pushed::Rejected) => stats.rejected += 1,
            Ok(push::Pushed::Conflicted) => stats.conflicted += 1,
            Ok(push::Pushed::Parked(reason)) => {
                stats.parked += 1;
                if stats.first_park.is_none() {
                    stats.first_park = Some(reason);
                }
            }
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
                // Both of these end the drain, and for the same reason: the
                // fault is the MOUNT's, not this candidate's, so every
                // remaining push would fail identically.
                //
                // Config is the one that shipped broken. It used to fall
                // through to the next candidate, which meant a push that can
                // NEVER succeed — a missing Calendars.ReadWrite scope, say —
                // was retried on every drain forever. One un-pushable edit
                // became an infinite loop: push, fail, leave pending, drain
                // again, push. It burned a core in production and spent the
                // provider's rate limit on calls that were always going to 403.
                if matches!(e, AdapterError::Config(_)) {
                    stats.misconfigured = true;
                    stats.pending += candidates.len().saturating_sub(i + 1);
                    state.writeback_retry_after =
                        Some(Utc::now().timestamp() + MISCONFIGURED_BACKOFF_SECS);
                    return;
                }
                if matches!(e, AdapterError::AuthExpired) {
                    return;
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
}

/// The receipt phase: the outbound health badge, the one reason an operator
/// gets to read, and the drain summary left on the mount.
///
/// Runs on EVERY drain that reached it, clean ones included — that is what
/// CLEARS a stale badge — so it is deliberately the last thing `drain` does and
/// deliberately not skipped by any early return above it.
fn record_outcome(
    mount_id: &str,
    state: &mut MountState,
    stats: &DrainStats,
    policy: &ConflictPolicy,
    has_candidates: bool,
) {
    if stats.pushed > 0
        || stats.failed > 0
        || stats.gone > 0
        || stats.deleted > 0
        || stats.rejected > 0
        || stats.conflicted > 0
        || stats.parked > 0
    {
        tracing::info!(
            mount_id = %mount_id,
            pushed = stats.pushed,
            skipped = stats.skipped,
            failed = stats.failed,
            gone = stats.gone,
            deleted = stats.deleted,
            detached = stats.detached,
            rejected = stats.rejected,
            conflicted = stats.conflicted,
            parked = stats.parked,
            truncated = stats.truncated,
            stopped = stats.stopped,
            "virtual mount writeback drain finished"
        );
    }

    // The outbound health badge, written on EVERY drain that got this far —
    // including a clean one, which is what CLEARS it. A `conflict` status that
    // outlives the conflict is a mount an operator keeps investigating and keeps
    // finding healthy, and the second time that happens they stop looking.
    //
    // Only a PARKED conflict counts. An abandoned one (`remote_wins`) is
    // self-clearing by design: the next delta overwrites the local value and the
    // node converges without anyone doing anything, so badging the mount for it
    // would leave every `remote_wins` mount permanently red.
    // `misconfigured` outranks `conflict`: a conflict is a decision waiting to
    // be made about one edit, while a misconfiguration means nothing can be
    // pushed at all. It is also what the automatic-drain gate reads, so
    // clearing it on a drain that never reached the provider would restart the
    // retry loop it exists to stop.
    // A drain that got here without a config failure has proved the mount is
    // configured, so the stand-off goes. Otherwise a mount that recovered would
    // keep skipping drains for the rest of its window.
    if !stats.misconfigured {
        state.writeback_retry_after = None;
    }
    state.writeback_status = if stats.misconfigured {
        Some("misconfigured".to_string())
    } else if stats.parked > 0 {
        Some("conflict".to_string())
    } else {
        None
    };
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
    } else if let Some(park) = &stats.first_park {
        // Ranked directly below a hard failure and above everything else: a
        // parked conflict is the only outcome here that does not resolve itself.
        // A `gone` settles when the item is re-imported and a `rejected` when
        // the policy or the node changes; this one waits for a person.
        state.writeback_last_error = Some(if stats.parked > 1 {
            format!("{} conflict(s) parked. First: {park}", stats.parked)
        } else {
            park.clone()
        });
    } else if stats.gone > 0 {
        state.writeback_last_error = Some(format!(
            "{} pending edit(s) could not be pushed: the provider no longer has \
             the object under the id this mount recorded. They stay pending until \
             the item is re-imported under its current id.",
            stats.gone
        ));
    } else if stats.rejected > 0 {
        // Ranked below `gone` and below a real failure, and above silence. A
        // rejected move is a deliberate configuration doing exactly what it was
        // asked to do — but the mount is not writing the nodes it is supposed to
        // be writing, and an operator reading `outcome: ok` has no other way to
        // learn that.
        state.writeback_last_error = Some(format!(
            "{} pending edit(s) were withheld: move_policy is 'reject' and these              nodes have been moved locally. The local move stands; nothing will              be pushed for them until they are moved back or the policy changes.",
            stats.rejected
        ));
    } else if stats.conflicted > 0 {
        // Lowest rank, and still not silence. Under `remote_wins` this is the
        // system working as configured — and it is also N local edits that were
        // discarded. An operator seeing it every run is being told to consider a
        // different policy, which they cannot do if nothing says it happened.
        state.writeback_last_error = Some(format!(
            "{} local edit(s) were discarded: the provider had changed the object since \
             this mount last read it and the conflict policy is '{}'. The next sync \
             overwrites them with the remote values.",
            stats.conflicted,
            policy.as_str()
        ));
    }
    // Written on every drain that DID something, clean ones included: a summary
    // left over from an earlier run would read as the current state, and "120
    // pushed, 380 pending" going stale is exactly the wrong thing to leave on a
    // mount that has since caught up.
    //
    // A drain with no candidates and nothing staged leaves the previous receipt
    // alone: it did not run, and overwriting a real receipt with an empty one
    // would erase what the last drain actually did.
    if has_candidates || *stats != DrainStats::default() {
        state.last_drain = Some(stats.summary());
    }
}
