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
/// Four ways that is false, each of which deletes real content:
///
///  1. The mount's `list` is not authoritative for its own content, so a walk
///     that completed perfectly still enumerated only part of what the mount
///     owns. That cannot be inferred — the engine is domain-blind — so the
///     mount states it with `sync_config.reconcile_deletes: false`. See that
///     field for the IMAP incident that forced it.
///
///  2. The walk hit `max_items_per_sync` (default 500) and stopped early. A
///     mailbox larger than the cap would have every message past item 500
///     deleted and recreated on every full sync — churn, a trigger storm,
///     and destroyed local edits, forever.
///  3. The walk RESUMED a chunked backfill. `seen` then holds only the final
///     chunk, so reconciling would delete everything the earlier chunks
///     imported — the whole point of the backfill. `seen` is per-run and
///     cannot be accumulated across runs without persisting every external
///     id, so a resumed pass simply never reconciles.
///  4. The provider answered with an empty listing (a silent hiccup, an
///     emptied-then-refilled folder, a permissions change that reads as
///     "no items" rather than an error). Deleting the entire mount subtree
///     on the strength of one empty response is never the right trade.
///
/// The safe action in all four is to skip reconciliation: a stale extra node
/// is recoverable on the next clean pass, deleted content is not. Deletes
/// therefore only ever run on a single-run, start-to-finish walk.
///
/// Cases 2-4 are TRANSIENT — the next unhindered walk reconciles what this one
/// could not, and upstream deletions reach the mount promptly through the delta
/// path meanwhile. Case 1 is PERMANENT and must not be read as the same trade:
/// a mount that turns `reconcile_deletes` off has no walk-based pruning ever
/// again, and its delta feed may not carry deletions either. The IMAP adapter
/// is exactly that shape — `opGetChanges` emits `type: "created"` items only —
/// so on such a mount an upstream deletion is never reflected at all, and
/// stale nodes are aged out by `sync_config.ephemeral` / `ttl_seconds` or not
/// at all. That is the cost of the field, and it is the smaller one: the
/// alternative was deleting every live message on a forced full run.
pub(super) async fn reconcile_deletes(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    seen: &HashSet<String>,
    flags: WalkFlags,
    processed: u64,
    max: u64,
) -> std::result::Result<(), AdapterError> {
    // Served from the run's prefetched index, kept current by every flush above —
    // this used to be another full workspace scan. Borrowed rather than cloned:
    // the guards below need only a count, and the delete branch needs only the
    // external ids it is actually going to remove, which is normally none.
    let existing_len = batcher.virtual_nodes_iter().count();

    if !ctx.mount.sync_config.reconcile_deletes {
        // The mount has declared that its `list` is not authoritative for its
        // own content, so `seen` is not a picture of everything that should
        // exist. IMAP is why: `opList` enumerates MAILBOXES only and messages
        // arrive through `get_changes`, so a forced full run staged every
        // message node for delete and the delta path — `fetchSince` the highest
        // uid — never brought them back.
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            existing = existing_len,
            seen = seen.len(),
            "skipping reconcile deletes: sync_config.reconcile_deletes is off, so this mount's \
             full walk is not authoritative for its own content"
        );
    // A stopped walk joins the same guard as a truncated or resumed one: it saw
    // only part of the provider, so `seen` cannot distinguish "deleted upstream"
    // from "not reached before the operator stopped us". Reconciling on it would
    // delete real content.
    } else if flags.truncated || flags.resuming || flags.stopped {
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            processed,
            resumed = flags.resuming,
            truncated = flags.truncated,
            stopped = flags.stopped,
            max_items_per_sync = max,
            existing = existing_len,
            "skipping reconcile deletes: this pass did not walk the provider end to end in one \
             run, so 'not seen' does not mean 'deleted upstream'"
        );
    } else if seen.is_empty() && existing_len > 0 && !ctx.mount.sync_config.allow_empty_reconcile {
        tracing::error!(
            mount_id = %ctx.mount.mount_id,
            existing = existing_len,
            "full sync returned NO items while {} mount-owned nodes exist; skipping reconcile \
             deletes rather than emptying the mount. Check the mount's remote root and filters, \
             or set sync_config.allow_empty_reconcile if the source really can be emptied.",
            existing_len
        );
    } else {
        // The ids are taken in one borrowing pass BEFORE any staging, because
        // `stage_delete` borrows the batcher mutably — and, like the clone it
        // replaces, that makes the set a snapshot taken before the first delete
        // rather than a live view being mutated as we walk it.
        let stale: Vec<String> = batcher
            .virtual_nodes_iter()
            // A COMMAND IS NOT A MIRROR. It was authored locally and sent, so
            // "not seen in the provider's listing" is its ordinary condition,
            // not evidence it was deleted upstream. Several providers answer a
            // send with no id at all — Graph's `sendMail` is a 202 with an empty
            // body — which leaves the node stamped `cmd:{node_id}`, an id no
            // listing can ever return. Without this an outbox that also lists
            // would delete every command it had just successfully sent, and the
            // record of the send with it.
            .filter(|node| !node.is_command)
            .filter(|node| !seen.contains(&node.external_id))
            .map(|node| node.external_id.clone())
            .collect();
        let mut deleted = 0usize;
        for external_id in stale {
            batcher.stage_delete(&external_id).await?;
            deleted += 1;
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
///
/// ONLY when there is no cursor yet. A baseline means "changes from now on", so
/// writing one over a cursor that still has unread pages behind it discards
/// every one of those changes, permanently and silently.
///
/// That is not a corner case: `phases::run_phases` runs the delta pass and then
/// a backfill chunk in the same run, and the delta pass ends with pages
/// outstanding whenever it spends its item budget or its clock — by design, so
/// the next run resumes. The backfill chunk that followed then overwrote the
/// resume token with a baseline, and the moves, renames and upstream deletions
/// behind it were never applied. The run reported `ok`.
///
/// Capturing on the FIRST chunk of a long backfill is still correct and still
/// wanted: it is what lets new mail arrive through the delta path while history
/// imports behind it. The narrow window it leaves — a change to an
/// already-walked item during that first chunk — is closed by the next full
/// reconcile, not by capturing a second baseline over a live cursor.
pub(super) async fn capture_delta_baseline(ctx: &SyncCtx<'_>, state: &mut MountState) {
    if state.last_sync_token.is_some() {
        tracing::debug!(
            mount_id = %ctx.mount.mount_id,
            "delta cursor already established; not capturing a baseline over it"
        );
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
        return;
    }
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
