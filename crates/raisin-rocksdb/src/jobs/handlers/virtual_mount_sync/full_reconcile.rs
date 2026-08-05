//! What a full walk does once its loops are done: settle the resume point,
//! decide whether deleting is safe, and capture a delta baseline.

use std::collections::HashSet;

use serde_json::json;

use super::batch::SyncBatcher;
use super::config::MountState;
use super::{persist_state, AdapterError, StateWrite, SyncCtx};

/// Why `seen` may be a partial picture of the provider. Any of these gates the
/// reconcile deletes; see [`reconcile_deletes`].
#[derive(Clone, Copy)]
pub(super) struct WalkFlags {
    /// The walk continued a chunked backfill rather than starting at the root.
    pub resuming: bool,
    /// The walk stopped on `max_items_per_sync` before exhausting the provider.
    pub truncated: bool,
    /// An operator stopped the walk at a page boundary.
    pub stopped: bool,
}

/// Persist (or clear) the resume point. `backfill_complete` is what makes
/// reconcile deletes safe: before the first end-to-end pass, "not seen"
/// only means "not reached yet".
pub(super) fn settle_resume_point(
    state: &mut MountState,
    stack: &[(Option<String>, String)],
    flags: WalkFlags,
    processed: u64,
    mount_id: &str,
) {
    if flags.truncated {
        state.backfill_stack = stack.to_vec();
        state.backfill_complete = false;
        tracing::info!(
            mount_id = %mount_id,
            processed,
            folders_pending = stack.len(),
            "backfill chunk complete; will resume on the next run"
        );
    } else if flags.stopped {
        // Stopped by an operator: DISCARD the resume point.
        //
        // This is the one place the walk must not behave like a truncated
        // chunk. The stop endpoint has already cleared `backfill_cursor` and
        // `backfill_stack`, and this state write happens AFTER it — so saving
        // them here would resurrect exactly what the operator asked to throw
        // away, leave the mount due on the next tick, and make Stop a button
        // that pauses for one run. The endpoint, this branch and the console's
        // confirmation dialog all have to agree that a stop discards; they now
        // do.
        //
        // Preserving the position is what PAUSE is for, and pause leaves the
        // walk untouched precisely so it can resume.
        state.backfill_stack.clear();
        state.backfill_cursor = None;
        state.backfill_complete = false;
        tracing::info!(
            mount_id = %mount_id,
            processed,
            "backfill stopped by request; resume point discarded"
        );
    } else {
        state.backfill_stack.clear();
        state.backfill_cursor = None;
        state.backfill_complete = true;
    }
}

/// Reconcile deletes: remove mount-owned nodes not seen this pass.
///
/// "Not seen" only means "deleted upstream" when THIS RUN saw everything.
/// Three ways that is false, each of which deletes real content:
///
///  1. The walk hit `max_items_per_sync` (default 500) and stopped early. A
///     mailbox larger than the cap would have every message past item 500
///     deleted and recreated on every full sync — churn, a trigger storm,
///     and destroyed local edits, forever.
///  2. The walk RESUMED a chunked backfill. `seen` then holds only the final
///     chunk, so reconciling would delete everything the earlier chunks
///     imported — the whole point of the backfill. `seen` is per-run and
///     cannot be accumulated across runs without persisting every external
///     id, so a resumed pass simply never reconciles.
///  3. The provider answered with an empty listing (a silent hiccup, an
///     emptied-then-refilled folder, a permissions change that reads as
///     "no items" rather than an error). Deleting the entire mount subtree
///     on the strength of one empty response is never the right trade.
///
/// The safe action in all three is to skip reconciliation: a stale extra node
/// is recoverable on the next clean pass, deleted content is not. Deletes
/// therefore only ever run on a single-run, start-to-finish walk — and
/// upstream deletions still propagate promptly through the delta path.
pub(super) async fn reconcile_deletes(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    seen: &HashSet<String>,
    flags: WalkFlags,
    processed: u64,
    max: u64,
) -> std::result::Result<(), AdapterError> {
    // Served from the run's prefetched index, kept current by every flush above —
    // this used to be another full workspace scan.
    let existing = batcher.virtual_nodes();

    // A stopped walk joins the same guard: it saw only part of the provider, so
    // `seen` cannot distinguish "deleted upstream" from "not reached before the
    // operator stopped us". Reconciling on it would delete real content.
    if flags.truncated || flags.resuming || flags.stopped {
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            processed,
            resumed = flags.resuming,
            truncated = flags.truncated,
            stopped = flags.stopped,
            max_items_per_sync = max,
            existing = existing.len(),
            "skipping reconcile deletes: this pass did not walk the provider end to end in one \
             run, so 'not seen' does not mean 'deleted upstream'"
        );
    } else if seen.is_empty()
        && !existing.is_empty()
        && !ctx.mount.sync_config.allow_empty_reconcile
    {
        tracing::error!(
            mount_id = %ctx.mount.mount_id,
            existing = existing.len(),
            "full sync returned NO items while {} mount-owned nodes exist; skipping reconcile \
             deletes rather than emptying the mount. Check the mount's remote root and filters, \
             or set sync_config.allow_empty_reconcile if the source really can be emptied.",
            existing.len()
        );
    } else {
        let mut deleted = 0usize;
        for node in existing {
            if !seen.contains(&node.external_id) {
                batcher.stage_delete(&node.external_id).await?;
                deleted += 1;
            }
        }
        batcher.flush().await?;
        if deleted > 0 {
            tracing::info!(
                mount_id = %ctx.mount.mount_id,
                deleted,
                seen = seen.len(),
                "full sync reconcile removed nodes that are gone upstream"
            );
        }
    }
    Ok(())
}

/// Best-effort delta baseline: if the provider has a changes API, capture a
/// resumable token so the next sync goes incremental. Providers without one
/// leave the token unset and stay on the full path.
///
/// `baseline_only` is load-bearing, not a hint. A changes API asked for
/// "everything since nothing" answers with an initial full ENUMERATION, not
/// a baseline — Microsoft Graph returns page after page of every message in
/// the folder and only emits a resumable delta link on the very last one.
/// Storing page 1 of that as the delta token meant every later "delta" run
/// walked the enumeration 600 items at a time, re-reading mail this walk had
/// just imported and reporting `0 written / 600 skipped` forever. New items
/// could not arrive until the whole mailbox had been re-enumerated.
///
/// We have just materialized everything, so what we want is "changes from
/// now on" — which providers expose directly (Graph: `$deltatoken=latest`).
pub(super) async fn capture_delta_baseline(ctx: &SyncCtx<'_>, state: &mut MountState) {
    match ctx
        .call(
            "get_changes",
            json!({
                "since_token": null,
                "folder_id": ctx.mount.remote_root,
                "baseline_only": true,
            }),
        )
        .await
    {
        Ok(resp) => {
            if let Some(token) = resp.get("next_token").and_then(|v| v.as_str()) {
                state.last_sync_token = Some(token.to_string());
            }
        }
        // Best-effort, but never silent. A mount that cannot capture a baseline
        // stays on the full path forever, re-walking the whole provider every
        // run — the single most expensive steady state this engine has, and
        // previously it left no trace anywhere.
        Err(e) => {
            tracing::warn!(
                mount_id = %ctx.mount.mount_id,
                error = %e,
                "could not capture a delta baseline; this mount stays on the full-walk path"
            );
        }
    }
    if !persist_state(ctx, state)
        .await
        .map(StateWrite::is_written)
        .unwrap_or(false)
    {
        tracing::warn!(
            mount_id = %ctx.mount.mount_id,
            "final full-walk state write was refused; this pass is not reflected on the mount"
        );
    }
}
