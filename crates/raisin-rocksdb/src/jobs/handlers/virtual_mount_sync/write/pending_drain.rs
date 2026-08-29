//! The page-boundary drain: local edits made WHILE this run is walking.
//!
//! Split out of [`super`] because it shares no code with the scheduled drain and
//! is called from somewhere else entirely — the read phases (`full`, `delta`) at
//! each page boundary, not [`super::drain`]. It is the plain UPDATE arm only,
//! deliberately: deletes, creates and submit have rails and scans that must not
//! run per page, and a submit retried from here would be a duplicate send.
//!
//! It owns nothing but that call site. The guards it repeats from `drain` are
//! shared as [`super::standing_off`] where they are genuinely identical, and
//! kept separate where the two callers are deliberately asymmetric.

use chrono::Utc;

use super::super::config::MountState;
use super::super::{batch, SyncCtx};
use super::{candidates, pending, push, resolve_conflict_policy, WriteMode};

/// How many pending edits one page boundary pushes; the rest wait for the next
/// boundary so a burst of edits cannot starve the walk.
const PENDING_PER_BOUNDARY: usize = 20;

/// Push nodes edited while THIS run has been walking the provider.
///
/// The scheduled drain nominates from an index snapshotted at run start, so an
/// edit landing mid-import is invisible to it until the next run — which a
/// long backfill can postpone for hours. The capture hook notes such edits in
/// [`pending`]; this pushes them at a page boundary, where the batcher is
/// flushed and the lease is held — exactly the invariant the ordinary drain
/// relies on (an unflushed stamp is never applied after the read's write).
///
/// Deliberately only the plain UPDATE arm. Deletes, creates and submit have
/// their own rails and scans that must not run per page — and a submit retried
/// from here would be a duplicate send.
///
/// Never fails the walk, and never writes mount state: a losing writer would
/// burn a `state_seq` the walk's own cursor writes depend on.
pub(crate) async fn drain_pending(
    ctx: &SyncCtx<'_>,
    state: &MountState,
    batcher: &mut batch::SyncBatcher<'_>,
) -> usize {
    let fields = match ctx.write_mode.get() {
        Some(WriteMode::StateOnly(fields)) => fields.clone(),
        Some(WriteMode::Mirror(plan)) => plan.fields.clone(),
        _ => return 0,
    };
    // A push landing mid-remap ping-pongs: the walk re-materializes the field
    // from the provider's pre-push view while `__pushed_state` records it as
    // pushed, so it is re-nominated on every page until the walk ends.
    if ctx.scope.force_rewrite {
        return 0;
    }
    // A BLOCKED mount makes no outbound changes at all — including here.
    //
    // A `blocked` verdict from the delete rails parks EVERY pending intent, not
    // only the deletes, and `drain` honours that by returning before it pushes
    // anything. This path did not: it is called at every page boundary of the
    // read phases, which keep running after the drain parks, so a mount an
    // operator had parked to investigate a suspected mass delete went on
    // issuing real provider updates for the rest of the run. The console said
    // "nothing was sent to the provider and nothing was lost" while it was
    // being sent.
    //
    // Checked BEFORE `pending::take`, so the notes survive to be drained once
    // the block is lifted rather than being consumed and dropped.
    if state.writeback_blocked.is_some() {
        return 0;
    }
    // Standing off after a config error means NO provider calls.
    if super::standing_off(state, Utc::now().timestamp()).is_some() {
        return 0;
    }
    if ctx.out_of_time(Utc::now().timestamp()) {
        return 0;
    }
    let key = pending::pending_key(
        &ctx.scope.tenant,
        &ctx.scope.repo,
        &ctx.config_branch,
        &ctx.scope.mount_id,
    );
    let ids = pending::take(&key, PENDING_PER_BOUNDARY);
    if ids.is_empty() {
        return 0;
    }
    let policy = match resolve_conflict_policy(&ctx.mount) {
        Ok(policy) => policy,
        Err(_) => {
            // The run's own full drain already refused and badged the mount;
            // a page boundary has nothing to add. Hand the notes back.
            for id in &ids {
                pending::note(&key, id);
            }
            return 0;
        }
    };

    let mut pushed = 0usize;
    let mut iter = ids.into_iter();
    while let Some(node_id) = iter.next() {
        if ctx.out_of_time(Utc::now().timestamp()) {
            pending::note(&key, &node_id);
            for rest in iter {
                pending::note(&key, &rest);
            }
            break;
        }
        // One point read for the external id; `push_one` re-reads and
        // re-checks divergence itself, so a stale note (already drained,
        // converged, unmapped) costs this read and nothing else. A note whose
        // push fails is dropped, not retried: the edit itself is safe — the
        // node still diverges, so the next run's full drain re-nominates it
        // from its fresh index. Only the latency shortcut is lost.
        let node = match ctx.materializer.read_node(&ctx.scope, &node_id).await {
            Ok(Some(node)) => node,
            _ => continue,
        };
        let Some(external_id) = super::materializer::node_external_id(&node) else {
            continue;
        };
        let candidate = candidates::Candidate {
            node_id: node_id.clone(),
            external_id: external_id.to_string(),
        };
        if let Ok(push::Pushed::Sent) =
            push::push_one(ctx, batcher, &candidate, &fields, &policy, false).await
        {
            pushed += 1;
        }
    }
    if pushed > 0 {
        // The pushes staged etag stamp-backs; land them before the walk reads
        // its next page — the same invariant as the ordinary drain.
        if let Err(e) = batcher.flush().await {
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                error = %e,
                "mid-run pending drain: flush failed; stamps land with the next page flush"
            );
        }
        tracing::info!(
            mount_id = %ctx.scope.mount_id,
            pushed,
            "pushed pending local edits at a page boundary"
        );
    }
    pushed
}
