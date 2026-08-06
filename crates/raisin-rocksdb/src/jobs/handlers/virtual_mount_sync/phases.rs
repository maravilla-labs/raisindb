//! Which phase a run ACTUALLY executes, and recovering in-run from a cursor the
//! provider no longer accepts.

use super::*;

/// Pick the phase, announce it, run it, and recover from a rejected cursor.
///
/// Takes no `self`, no lock and no job: by the time this runs, everything a
/// phase needs is in `ctx`, `state` and `batcher`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_phases(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    batcher: &mut batch::SyncBatcher<'_>,
    capabilities: &Capabilities,
    write_mode: &write::WriteMode,
    remap: bool,
    mode: &str,
    trigger: &str,
) -> std::result::Result<(), AdapterError> {
    // Local edits go OUT first, under this run's lease and ahead of the read
    // phases — see the `write` module doc for why the ordering is load-bearing
    // rather than a preference. It never returns an error: a mount that cannot
    // write must still be able to read.
    write::drain(ctx, state, batcher, write_mode).await;

    // Choose the sync path. Full reconcile is forced on the first sync, an
    // explicit `mode: "full"`, or when the adapter cannot serve a changes
    // feed (`supports_changes: false`) even if a cursor happens to exist.
    // A remap (flagged above) is a full walk by definition: the delta feed
    // only reports what changed upstream, which for a remap is nothing.
    let use_full = remap
        || mode == "full"
        || state.last_sync_token.is_none()
        || !capabilities.supports_changes;

    // An unfinished backfill resumes on subsequent runs even when the mount
    // is otherwise on the delta path.
    let backfill_pending = !state.backfill_stack.is_empty() || state.backfill_cursor.is_some();

    // Record the phase that is ACTUALLY about to run, replacing the mode the
    // caller asked for.
    //
    // These diverge routinely and the difference is not cosmetic: a run
    // requested as `delta` executes a full reconcile whenever the provider
    // rejects a stale cursor, and the history then showed "Delta" against a
    // run writing five-year-old mail. An operator reading that has no way to
    // reach the right conclusion — it cost a long investigation.
    let phase = if remap {
        "remap"
    } else if use_full {
        "full"
    } else if backfill_pending {
        "delta+backfill"
    } else {
        "delta"
    };
    if let Some(run) = state.last_run.as_mut() {
        run.mode = phase.to_string();
    }
    raisin_storage::jobs::global_mount_broadcaster().emit(
        &raisin_storage::jobs::mount_channel_key(
            &ctx.scope.tenant,
            &ctx.scope.repo,
            &ctx.scope.mount_id,
        ),
        raisin_storage::jobs::MountSyncEvent::Started {
            phase: phase.to_string(),
            trigger: trigger.to_string(),
        },
    );

    let result = if use_full {
        full::run_with(ctx, state, batcher).await
    } else if backfill_pending {
        // DELTA FIRST, then one backfill chunk — in that order, deliberately.
        //
        // Importing a large mailbox takes hours of chunked walking. Running
        // the backfill first (or on its own job) would mean new mail waited
        // for the whole import to finish. Doing the incremental pass first
        // on every run keeps new items arriving within one interval while
        // history fills in behind them, and because both share this run's
        // single lease there is no second writer to race the mount state.
        let delta_outcome = delta::run_with(ctx, state, batcher).await;
        match delta_outcome {
            Ok(()) => full::run_with(ctx, state, batcher).await,
            // A failing delta must not be masked by a successful backfill
            // chunk: surface it and leave the resume point untouched.
            Err(e) => Err(e),
        }
    } else {
        delta::run_with(ctx, state, batcher).await
    };

    // A cursor the provider no longer accepts is recoverable HERE, in the
    // same run: drop it and fall back to a full reconcile.
    //
    // Every incremental API expires its cursors, and the recovery they all
    // document is "resync from scratch". Leaving it to the job retry cannot
    // work — the retry re-sends the same rejected cursor — and the failure
    // counter it accumulates then gates the backfill fast path in
    // `check::is_due`, so a mount with an expired delta token could not even
    // drain a pending import. Recovering in-run keeps `consecutive_failures`
    // at zero, which is what keeps the chunked backfill running back-to-back.
    let result = match result {
        Err(AdapterError::CursorInvalid(msg)) => {
            tracing::warn!(
                mount_id = %ctx.mount.mount_id,
                error = %msg,
                "provider rejected the stored delta cursor; discarding it and \
                 falling back to a full reconcile"
            );
            state.last_sync_token = None;
            full::run_with(ctx, state, batcher).await
        }
        other => other,
    };

    result
}
