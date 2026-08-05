//! Terminal state for a run: status, failure counters, the run record, and the
//! job result the console reads.

use super::*;

impl VirtualMountSyncHandler {
    /// Apply terminal state (status / failure counters), close the run record,
    /// persist, and return the run summary as the job result.
    pub(super) async fn finalize(
        &self,
        ctx: &SyncCtx<'_>,
        state: &mut MountState,
        result: std::result::Result<(), AdapterError>,
        counts: BatchStats,
    ) -> Result<Option<Value>> {
        // Stamped on EVERY outcome, before the match, so no arm can forget it.
        // This is what the scheduler backs off against; leaving it unset on the
        // failure paths is what let a broken mount retry on every tick forever
        // (see `MountState::last_attempt_at` and `check::is_due`).
        let now = Utc::now().timestamp();
        state.last_attempt_at = Some(now);

        // Fold this run's counts into the open run record. Every arm below goes
        // through `close_run`, so no outcome can leave the history without an
        // entry — the reason the old code could report `ok` for a run that had
        // in fact been refused.
        let mut run = state.last_run.take().unwrap_or_else(|| {
            SyncRun::started(now, ctx.mount.sync_config.mode.as_str(), "unknown")
        });
        run.written = counts.written as u64;
        run.skipped = counts.skipped as u64;
        run.deleted = counts.deleted as u64;
        run.failed = counts.failed as u64;
        run.items_done = state.backfill_items_done;

        let retryable_err = match result {
            Ok(()) => {
                state.status = Some("ok".to_string());
                state.consecutive_failures = 0;
                state.last_error = None;
                state.last_sync_at = Some(now);
                run.finish(now, "ok", None);
                None
            }
            Err(AdapterError::AuthExpired) => {
                state.status = Some("auth_required".to_string());
                state.last_error = Some("auth_expired".to_string());
                run.finish(now, "auth_required", Some("auth_expired".to_string()));
                None
            }
            Err(e) => {
                state.consecutive_failures += 1;
                state.last_error = Some(e.to_string());

                // A misconfigured mount gets its own status so the UI can say
                // "fix this" rather than "it is flaky" — the operator action is
                // completely different.
                let outcome = if matches!(e, AdapterError::Config(_)) {
                    state.status = Some("misconfigured".to_string());
                    "misconfigured"
                } else {
                    if state.consecutive_failures >= config::DEGRADE_THRESHOLD {
                        state.status = Some("degraded".to_string());
                    }
                    "error"
                };
                run.finish(now, outcome, Some(e.to_string()));

                // Returning Err makes the JOB layer retry the whole sync (3x)
                // on top of the scheduler's own re-enqueue. That is right for a
                // blip and wrong for anything that cannot succeed: it multiplies
                // a permanent failure into four provider calls per tick. For a
                // non-retryable error the run is complete — the state carries
                // the diagnosis, and `last_attempt_at` backs the schedule off.
                if e.is_retryable() {
                    Some(Error::Backend(format!("virtual mount sync failed: {e}")))
                } else {
                    tracing::warn!(
                        mount_id = %ctx.mount.mount_id,
                        error = %e,
                        status = state.status.as_deref().unwrap_or("degraded"),
                        "virtual mount sync failed permanently; not retrying until the mount is changed"
                    );
                    None
                }
            }
        };

        // Push onto the bounded history BEFORE the write, so the record lands in
        // the same revision as the status it describes.
        state.recent_runs.insert(0, run.clone());
        state.recent_runs.truncate(config::MAX_RUN_HISTORY);
        state.last_run = Some(run.clone());

        let wrote = persist_state(ctx, state)
            .await
            .map(StateWrite::is_written)
            .unwrap_or(false);
        if !wrote {
            // The run happened; its record did not land. Say so loudly and put
            // it in the returned summary, because the mount node now under-
            // reports what actually occurred.
            tracing::warn!(
                mount_id = %ctx.mount.mount_id,
                written = counts.written,
                deleted = counts.deleted,
                "mount-state write was refused: this run's progress is NOT reflected on the mount"
            );
            run.state_write_skipped = Some(true);
        }

        // One emit, at the single point every outcome funnels through — the same
        // reason `close_run` exists. An emit per branch would eventually miss
        // one, and a console left showing a run that never ends is exactly the
        // phantom this work set out to remove.
        raisin_storage::jobs::global_mount_broadcaster().emit(
            &raisin_storage::jobs::mount_channel_key(
                &ctx.scope.tenant,
                &ctx.scope.repo,
                &ctx.scope.mount_id,
            ),
            raisin_storage::jobs::MountSyncEvent::Finished {
                phase: run.mode.clone(),
                outcome: run.outcome.clone(),
                written: run.written,
                skipped: run.skipped,
                deleted: run.deleted,
                failed: run.failed,
                backfill_complete: state.backfill_complete,
                error: run.error.clone(),
            },
        );

        let summary = serde_json::to_value(&run).unwrap_or(Value::Null);
        match retryable_err {
            Some(e) => Err(e),
            None => Ok(Some(summary)),
        }
    }
}
