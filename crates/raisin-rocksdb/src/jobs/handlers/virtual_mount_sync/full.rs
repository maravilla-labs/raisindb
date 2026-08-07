//! Full-listing reconcile: first sync, `supports_changes:false`, or `mode:full`.
//!
//! Recursively `list`s from `remote_root`, upserts everything (after filters),
//! then deletes any mount-owned virtual node not seen this pass. Finally tries
//! to establish a delta baseline token so subsequent syncs can go incremental.

use chrono::Utc;
use std::collections::HashSet;

use serde_json::json;

use super::batch::SyncBatcher;
use super::config::{ExternalItem, ListPage, MountState};
use super::full_reconcile::{
    capture_delta_baseline, reconcile_deletes, settle_resume_point, WalkFlags,
};
use super::{persist_state, stage_item, AdapterError, StateWrite, SyncCtx};

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
                let rel_path = resolve_item_path(ctx, item, &prefix);
                if super::passes_filters(&rel_path, include, exclude) {
                    // Subordinate nodes (mail attachments) join `seen` with
                    // their parent. They are mount-owned and carry an
                    // `__external_id`, so `reconcile_deletes` considers them —
                    // and every one of them would be "not seen upstream" if the
                    // walk only ever reported the ids the provider LISTS.
                    let children = stage_item(ctx, batcher, item, &rel_path).await?;
                    seen.extend(children);
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
                // `break 'outer`, not `break`. A bare `break` left the page loop
                // but not the folder loop, so a mount with queued subfolders
                // carried on and pulled a page from every remaining folder — a
                // stop that did not stop.
                //
                // The resume point is deliberately NOT saved here; see the
                // `stopped` branch below.
                break 'outer;
            }
            // Local edits noted while this walk has been running go out HERE,
            // under this run's lease — a mount mid-import must not make its
            // user wait for the import to finish before an edit is pushed.
            super::write::drain_pending(ctx, state, batcher).await;
            published =
                publish_progress(ctx, state, &page, processed, published, stack.len()).await?;
            match page.next_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
            // Two reasons to stop with work left: the item budget, and the CLOCK.
            //
            // The clock one is what stops the watchdog reaping us. A walk that
            // runs past the job's `timeout_seconds` is aborted at its await
            // point, which skips `finalize` entirely — status wedged at
            // `"syncing"`, lease leaked for its full TTL, and for a webhook mount
            // nothing that would ever schedule the run that could clear it. Ending
            // ourselves here instead makes it an ordinary truncated chunk, which
            // this function already knows how to resume from.
            let out_of_time = ctx.out_of_time(Utc::now().timestamp());
            if processed >= max || out_of_time {
                truncated = true;
                if out_of_time {
                    tracing::info!(
                        mount_id = %ctx.mount.mount_id,
                        processed,
                        folders_pending = stack.len() + 1,
                        "wall-clock budget spent; ending this chunk cleanly so the run is not \
                         killed mid-walk. The next tick resumes from the persisted cursor."
                    );
                }
                // Save the resume point: this folder is unfinished, so it goes
                // back on the stack with the cursor that continues it.
                stack.push((folder_id.clone(), prefix.clone()));
                state.backfill_cursor = cursor.clone();
                break 'outer;
            }
        }
    }

    state.backfill_items_done = state
        .backfill_items_done
        .saturating_add(processed.saturating_sub(published));

    let flags = WalkFlags {
        resuming,
        truncated,
        stopped,
    };
    settle_resume_point(state, &stack, flags, processed, &ctx.mount.mount_id);
    reconcile_deletes(ctx, batcher, &seen, flags, processed, max).await?;
    capture_delta_baseline(ctx, state).await;
    Ok(())
}

/// The path an item materializes at, relative to the mount.
///
/// A configured hierarchy replaces the provider-derived path;
/// an unresolvable template falls back to it (see
/// `resolve_path_template`).
fn resolve_item_path(ctx: &SyncCtx<'_>, item: &ExternalItem, prefix: &str) -> String {
    match super::config::resolve_path_template(&ctx.mount.sync_config.path_template, item) {
        Some(p) => p,
        None if prefix.is_empty() => item.name.clone(),
        None => format!("{}/{}", prefix, item.name),
    }
}

/// Publish progress as we go. A chunk can run for minutes; without
/// this the console sees nothing move until the whole chunk lands.
///
/// Advance the REAL state, never a clone. `persist_state` bumps
/// `state_seq` on success, and a clone would carry that bump to its
/// grave — every later write in this run would then look stale
/// against the stored seq and be refused, which is the exact failure
/// this whole change exists to remove.
///
/// Returns the new `published` watermark.
async fn publish_progress(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    page: &ListPage,
    processed: u64,
    published: u64,
    subtrees_queued: usize,
) -> std::result::Result<u64, AdapterError> {
    let delta = processed.saturating_sub(published);
    state.backfill_items_done = state.backfill_items_done.saturating_add(delta);
    if let Some(total) = page.total {
        state.backfill_items_total = Some(total);
    }
    // A refused write means another writer owns this mount's state. Continuing
    // is not merely useless, it is DANGEROUS: `persist_mount_state` does not
    // advance `state_seq` on refusal, so every later write this run makes is
    // refused too — and the walk would still fall through to
    // `reconcile_deletes`, which on a partial `seen` set deletes real content.
    // `delta.rs` already stops on exactly this condition; the full walk did not.
    //
    // A MISSING mount node is a different answer and must not abort: it is the
    // routine shape in tests and for a caller driving a walk whose config lives
    // on another branch. Collapsing the two is what made the first attempt at
    // this fix break five passing tests.
    match persist_state(ctx, state).await {
        Ok(StateWrite::Written) | Ok(StateWrite::MountMissing) => {}
        Ok(StateWrite::Superseded) => {
            tracing::warn!(
                mount_id = %ctx.mount.mount_id,
                "backfill progress write superseded by a concurrent writer; abandoning this \
                 pass before it can reconcile"
            );
            return Err(AdapterError::Conflict(
                "mount state write refused by a concurrent writer".to_string(),
            ));
        }
        Err(e) => {
            tracing::warn!(
                mount_id = %ctx.mount.mount_id,
                error = %e,
                "backfill progress write failed"
            );
        }
    }

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
            subtrees_queued,
        },
    );
    Ok(processed)
}
