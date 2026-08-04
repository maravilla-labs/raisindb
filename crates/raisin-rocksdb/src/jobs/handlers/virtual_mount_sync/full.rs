//! Full-listing reconcile: first sync, `supports_changes:false`, or `mode:full`.
//!
//! Recursively `list`s from `remote_root`, upserts everything (after filters),
//! then deletes any mount-owned virtual node not seen this pass. Finally tries
//! to establish a delta baseline token so subsequent syncs can go incremental.

use std::collections::HashSet;

use serde_json::json;

use super::batch::SyncBatcher;
use super::config::{ListPage, MountState};
use super::{persist_state, stage_item, AdapterError, SyncCtx};

/// Run a full reconcile.
pub async fn run(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
) -> std::result::Result<(), AdapterError> {
    let mut batcher = SyncBatcher::new(ctx).await?;
    run_with(ctx, state, &mut batcher).await
}

/// Full reconcile against an already-open batcher, so a run that also does a
/// delta pass shares one index and one write buffer.
pub async fn run_with(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    batcher: &mut SyncBatcher<'_>,
) -> std::result::Result<(), AdapterError> {
    let max = ctx.mount.sync_config.max_items_per_sync;
    let include = &ctx.mount.sync_config.include_patterns;
    let exclude = &ctx.mount.sync_config.exclude_patterns;

    let mut seen: HashSet<String> = HashSet::new();
    let mut processed: u64 = 0;
    // How much of `processed` has already been folded into
    // `state.backfill_items_done` by a progress write, so the final tally adds
    // only the remainder instead of double-counting it.
    let mut published: u64 = 0;
    // Set when the walk stops before exhausting the provider, which makes
    // `seen` a PARTIAL picture and therefore unusable for deciding what to
    // delete. See the reconcile guard below.
    let mut truncated = false;
    // Set when an operator stopped the walk. Like `truncated`, it makes `seen`
    // a partial picture — so it must gate reconcile deletes for the same reason.
    let mut stopped = false;

    // Explicit stack of (folder_id, rel_prefix) to avoid async recursion.
    //
    // RESUMED from mount state when a previous pass ran out of budget. Before
    // this, the stack was rebuilt from the root on every run and the page cursor
    // was a local starting at None, so a provider with more items than
    // `max_items_per_sync` (default 500) re-walked its first 500 forever and the
    // remainder was never imported — a production mailbox could not be synced at
    // all. `resume_cursor` applies only to the first folder popped, which is the
    // one the previous pass was in the middle of.
    let resuming = !state.backfill_stack.is_empty() || state.backfill_cursor.is_some();
    let mut stack: Vec<(Option<String>, String)> = if resuming {
        state.backfill_stack.clone()
    } else {
        vec![(ctx.mount.remote_root.clone(), String::new())]
    };
    let mut resume_cursor = state.backfill_cursor.take();
    // A fresh walk restarts the progress count; a resumed one continues it, so
    // the console shows total-so-far rather than just this chunk.
    if !resuming {
        state.backfill_items_done = 0;
    }
    if resuming {
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            folders_pending = stack.len(),
            "resuming backfill walk from the persisted cursor"
        );
    }

    'outer: while let Some((folder_id, prefix)) = stack.pop() {
        let mut cursor: Option<String> = resume_cursor.take();
        loop {
            let params = json!({
                "folder_id": folder_id,
                "cursor": cursor,
                "limit": max,
            });
            let resp = ctx.call("list", params).await?;
            let page: ListPage = serde_json::from_value(resp)
                .map_err(|e| AdapterError::Transient(format!("bad list response: {e}")))?;

            for item in &page.items {
                // A configured hierarchy replaces the provider-derived path;
                // an unresolvable template falls back to it (see
                // `resolve_path_template`).
                let rel_path = match super::config::resolve_path_template(
                    &ctx.mount.sync_config.path_template,
                    item,
                ) {
                    Some(p) => p,
                    None if prefix.is_empty() => item.name.clone(),
                    None => format!("{}/{}", prefix, item.name),
                };
                if super::passes_filters(&rel_path, include, exclude) {
                    stage_item(ctx, batcher, item, &rel_path).await?;
                    seen.insert(item.external_id.clone());
                    processed += 1;
                }
                if item.is_folder {
                    stack.push((Some(item.external_id.clone()), rel_path));
                }
            }

            // Flush before publishing progress or a resume point. Both claim the
            // page landed, and a buffer that is still holding it would be dropped
            // silently on the next run's fresh start.
            batcher.flush().await?;
            ctx.renew_lease().await;

            // A stop lands HERE and nowhere else: the batch is flushed and the
            // cursor is about to be durable, so abandoning the walk loses
            // nothing and leaves a resume point the operator can discard or
            // keep. Checked after the flush, never before it.
            if super::stop_requested(ctx).await {
                tracing::info!(
                    mount_id = %ctx.mount.mount_id,
                    "stop requested; ending the full walk at a page boundary"
                );
                stopped = true;
                break;
            }
            // Publish progress as we go. A chunk can run for minutes; without
            // this the console sees nothing move until the whole chunk lands.
            //
            // Advance the REAL state, never a clone. `persist_state` bumps
            // `state_seq` on success, and a clone would carry that bump to its
            // grave — every later write in this run would then look stale
            // against the stored seq and be refused, which is the exact failure
            // this whole change exists to remove.
            {
                let delta = processed.saturating_sub(published);
                state.backfill_items_done = state.backfill_items_done.saturating_add(delta);
                published = processed;
                if let Some(total) = page.total {
                    state.backfill_items_total = Some(total);
                }
                let _ = persist_state(ctx, state).await;

                // Emit AFTER the state is durable, never before: an event is a
                // claim that this page landed, and a claim that a later failure
                // could roll back is worse than no event at all.
                raisin_storage::jobs::global_mount_broadcaster().emit(
                    &raisin_storage::jobs::mount_channel_key(
                        &ctx.scope.tenant,
                        &ctx.scope.repo,
                        &ctx.scope.mount_id,
                    ),
                    raisin_storage::jobs::MountSyncEvent::Progress {
                        phase: "full".to_string(),
                        backfill_items_done: state.backfill_items_done,
                        delta_items_done: state.delta_items_done,
                        items_total: state.backfill_items_total,
                        subtrees_queued: stack.len(),
                    },
                );
            }
            match page.next_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
            if processed >= max {
                truncated = true;
                // Save the resume point: this folder is unfinished, so it goes
                // back on the stack with the cursor that continues it.
                stack.push((folder_id.clone(), prefix.clone()));
                state.backfill_cursor = cursor.clone();
                break 'outer;
            }
        }
    }

    // Persist (or clear) the resume point. `backfill_complete` is what makes
    // reconcile deletes safe: before the first end-to-end pass, "not seen"
    // only means "not reached yet".
    state.backfill_items_done = state
        .backfill_items_done
        .saturating_add(processed.saturating_sub(published));

    if truncated {
        state.backfill_stack = stack.clone();
        state.backfill_complete = false;
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            processed,
            folders_pending = stack.len(),
            "backfill chunk complete; will resume on the next run"
        );
    } else if stopped {
        // Stopped by an operator. Save the resume point exactly as a truncated
        // chunk does — a stop that discarded the stack here would silently turn
        // "pause this import" into "start it over". Whether to keep or discard
        // the cursor is the endpoint's decision, not the walk's.
        state.backfill_stack = stack.clone();
        state.backfill_complete = false;
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            processed,
            folders_pending = stack.len(),
            "backfill stopped by request; resume point preserved"
        );
    } else {
        state.backfill_stack.clear();
        state.backfill_cursor = None;
        state.backfill_complete = true;
    }

    // Reconcile deletes: remove mount-owned nodes not seen this pass.
    //
    // "Not seen" only means "deleted upstream" when THIS RUN saw everything.
    // Three ways that is false, each of which deletes real content:
    //
    //  1. The walk hit `max_items_per_sync` (default 500) and stopped early. A
    //     mailbox larger than the cap would have every message past item 500
    //     deleted and recreated on every full sync — churn, a trigger storm,
    //     and destroyed local edits, forever.
    //  2. The walk RESUMED a chunked backfill. `seen` then holds only the final
    //     chunk, so reconciling would delete everything the earlier chunks
    //     imported — the whole point of the backfill. `seen` is per-run and
    //     cannot be accumulated across runs without persisting every external
    //     id, so a resumed pass simply never reconciles.
    //  3. The provider answered with an empty listing (a silent hiccup, an
    //     emptied-then-refilled folder, a permissions change that reads as
    //     "no items" rather than an error). Deleting the entire mount subtree
    //     on the strength of one empty response is never the right trade.
    //
    // The safe action in all three is to skip reconciliation: a stale extra node
    // is recoverable on the next clean pass, deleted content is not. Deletes
    // therefore only ever run on a single-run, start-to-finish walk — and
    // upstream deletions still propagate promptly through the delta path.
    // Served from the run's prefetched index, kept current by every flush above —
    // this used to be another full workspace scan.
    let existing = batcher.virtual_nodes();

    // A stopped walk joins the same guard: it saw only part of the provider, so
    // `seen` cannot distinguish "deleted upstream" from "not reached before the
    // operator stopped us". Reconciling on it would delete real content.
    if truncated || resuming || stopped {
        tracing::info!(
            mount_id = %ctx.mount.mount_id,
            processed,
            resumed = resuming,
            truncated,
            stopped,
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

    // Best-effort delta baseline: if the provider has a changes API, capture a
    // resumable token so the next sync goes incremental. Providers without one
    // leave the token unset and stay on the full path.
    //
    // `baseline_only` is load-bearing, not a hint. A changes API asked for
    // "everything since nothing" answers with an initial full ENUMERATION, not
    // a baseline — Microsoft Graph returns page after page of every message in
    // the folder and only emits a resumable delta link on the very last one.
    // Storing page 1 of that as the delta token meant every later "delta" run
    // walked the enumeration 600 items at a time, re-reading mail this walk had
    // just imported and reporting `0 written / 600 skipped` forever. New items
    // could not arrive until the whole mailbox had been re-enumerated.
    //
    // We have just materialized everything, so what we want is "changes from
    // now on" — which providers expose directly (Graph: `$deltatoken=latest`).
    if let Ok(resp) = ctx
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
        if let Some(token) = resp.get("next_token").and_then(|v| v.as_str()) {
            state.last_sync_token = Some(token.to_string());
        }
    }

    let _ = persist_state(ctx, state).await;
    Ok(())
}
