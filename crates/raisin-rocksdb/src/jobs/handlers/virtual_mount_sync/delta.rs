//! Delta (incremental) sync driven by the adapter's `get_changes` feed.
//!
//! Pages until `next_token` stabilizes or `max_items_per_sync` is reached,
//! materializing each change, and persists the cursor **only after** a page is
//! fully materialized (so a crashed page re-runs safely — upserts are
//! idempotent via etag skip-write).

use serde_json::json;

use super::batch::SyncBatcher;
use super::config::{ChangesPage, MountState};
use super::{persist_state, stage_item, AdapterError, StateWrite, SyncCtx};

/// Run a delta sync, updating `state.last_sync_token` as pages complete.
pub async fn run(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
) -> std::result::Result<(), AdapterError> {
    let mut batcher = SyncBatcher::new(ctx).await?;
    run_with(ctx, state, &mut batcher).await
}

/// Delta sync against an already-open batcher, so a run that also does a
/// backfill chunk shares one index and one write buffer.
pub async fn run_with(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
    batcher: &mut SyncBatcher<'_>,
) -> std::result::Result<(), AdapterError> {
    let max = ctx.mount.sync_config.max_items_per_sync;
    let include = &ctx.mount.sync_config.include_patterns;
    let exclude = &ctx.mount.sync_config.exclude_patterns;
    let mut token = state.last_sync_token.clone();
    let mut processed: u64 = 0;
    // Items actually staged, as opposed to returned by the provider. The two
    // diverge on filtered-out changes, and the console must report the former.
    let mut materialized: u64 = 0;
    // How much of `materialized` has already been folded into
    // `state.delta_items_done`, so each page adds only its own increment.
    //
    // Without this watermark the running TOTAL was added once per page — 100,
    // then 300, then 600 for three pages of 100 — so the reported figure grew
    // quadratically. The same regression was fixed once already for
    // `backfill_items_done`; the full walk's watermark (`full.rs`) is the shape
    // being mirrored here.
    let mut published: u64 = 0;

    // Self-healing baseline.
    //
    // No token means one of two very different situations, and asking for the
    // wrong one is what stalled a production mailbox:
    //
    //  - The full walk has NOT completed. Delta IS the import here, so an
    //    initial enumeration is exactly what we want.
    //  - The full walk HAS completed. Everything is already materialized, so an
    //    enumeration would re-read the entire folder a page per run, reporting
    //    `0 written / N skipped` while genuinely new items waited behind it.
    //    What we want is "changes from now on".
    //
    // This also recovers a mount whose stored token is already an enumeration
    // cursor: clear the token and the next run re-baselines instead of resuming
    // a walk that cannot converge.
    let want_baseline = token.is_none() && state.backfill_complete;

    // Only the FIRST request carries them. The adapter answers a push with
    // `has_more: false`, so this normally runs once — but resending the same events on a
    // second page would re-apply them, and an adapter that paged would never converge.
    let mut pushed = ctx.pushed_events.clone();

    loop {
        let mut params = json!({
            "since_token": token,
            "folder_id": ctx.mount.remote_root,
            "baseline_only": want_baseline,
        });
        // `.take()` so the events are consumed exactly once.
        //
        // Nothing forces the adapter to use them: one that does not know the field re-reads
        // from the provider exactly as before, which is what makes this safe to send to
        // every connector rather than only the ones that opted in.
        if let Some(events) = pushed.take() {
            params["events"] = events;
        }
        let resp = ctx.call("get_changes", params).await?;
        let page: ChangesPage = serde_json::from_value(resp)
            .map_err(|e| AdapterError::Transient(format!("bad get_changes response: {e}")))?;

        for change in &page.items {
            // `staged` is false when the change was filtered out or was already
            // present unchanged. Counting returned items instead of written ones
            // is how "0 written / 1200 skipped" and "1200 imported" ended up on
            // the same screen.
            if stage_change(ctx, batcher, change, include, exclude).await? {
                materialized += 1;
            }
            processed += 1;
        }
        // The cursor below claims this page landed, so the buffer must be empty
        // before it is written.
        batcher.flush().await?;

        // A REJECTED ITEM IS NOT A LANDED PAGE.
        //
        // The materializer rejects individual items (validation, RLS, a unique
        // clash) without failing the batch, which is right — one bad item must
        // not stall a mount. But the cursor was then persisted anyway, so the
        // change was never re-delivered: that item stayed permanently stale or
        // absent while the run reported `ok`.
        //
        // Exactly one automatic retry, and no more. The first time a page rejects
        // an item, the ids are parked and the cursor is left where it was, so the
        // next run re-pages the same window (upserts are idempotent through the
        // etag skip, so the items that did land cost nothing). If the same ids
        // fail again they are already parked, the cursor advances past them, and
        // they stay on `failed_items` — visible, and keeping the run off `ok` —
        // rather than blocking the feed forever behind one poison item.
        let rejected = batcher.take_failed_ids();
        // A parked item that came round again and landed is recovered. Cleared
        // here rather than on a timer, because "this id was in the page and was
        // not rejected" is the only evidence that actually settles it.
        if !state.failed_items.is_empty() {
            state.failed_items.retain(|parked| {
                let in_page = page
                    .items
                    .iter()
                    .any(|change| change.item.external_id == *parked);
                let still_failing = rejected.iter().any(|id| id == parked);
                still_failing || !in_page
            });
        }
        let first_failure = rejected
            .iter()
            .any(|id| !state.failed_items.iter().any(|parked| parked == id));
        for id in rejected {
            if state.failed_items.len() >= super::config::MAX_FAILED_ITEMS {
                break;
            }
            if !state.failed_items.iter().any(|parked| *parked == id) {
                state.failed_items.push(id);
            }
        }
        if first_failure {
            tracing::warn!(
                mount_id = %ctx.mount.mount_id,
                parked = state.failed_items.len(),
                "delta page rejected items the store would not accept; holding the cursor so \
                 the page is re-delivered once before it is passed over"
            );
            state.delta_items_done = state
                .delta_items_done
                .saturating_add(materialized.saturating_sub(published));
            let _ = persist_state(ctx, state).await;
            break;
        }

        // Persist the cursor after the whole page materialized — but ONLY when
        // the adapter actually returned one. A `None` next_token means "no more
        // pages"; overwriting the stored cursor with `None` would clear it and
        // silently force a full resync on the next run, so instead we retain the
        // existing cursor and stop paging.
        let next = page.next_token.clone();
        match &next {
            Some(tok) => {
                state.last_sync_token = Some(tok.clone());
                // Count delta work — but in ITS OWN counter.
                //
                // This used to add to `backfill_items_done`, which is documented
                // as "the current full walk" and is reset only when a fresh walk
                // starts. Nothing in the delta path resets it, so incremental
                // work accumulated forever: a settled mount reported "433,500
                // items imported" and an import that could never complete.
                // Keeping the visibility the original change wanted, without
                // corrupting the number it borrowed.
                state.delta_items_done = state
                    .delta_items_done
                    .saturating_add(materialized.saturating_sub(published));
                published = materialized;
                if !persist_state(ctx, state)
                    .await
                    .map(StateWrite::is_written)
                    .unwrap_or(false)
                {
                    // The cursor did not land. Continuing would re-page from a
                    // position the store does not know about, so stop and let
                    // the next run resume from the last durable cursor.
                    tracing::warn!(
                        mount_id = %ctx.mount.mount_id,
                        "delta cursor write refused; stopping this pass"
                    );
                    break;
                }
                ctx.renew_lease().await;

                raisin_storage::jobs::global_mount_broadcaster().emit(
                    &raisin_storage::jobs::mount_channel_key(
                        &ctx.scope.tenant,
                        &ctx.scope.repo,
                        &ctx.scope.mount_id,
                    ),
                    raisin_storage::jobs::MountSyncEvent::Progress {
                        phase: "delta".to_string(),
                        backfill_items_done: state.backfill_items_done,
                        delta_items_done: state.delta_items_done,
                        items_total: state.backfill_items_total,
                        subtrees_queued: state.backfill_stack.len(),
                    },
                );

                // Clean stop boundary: batch flushed, cursor durable.
                if super::stop_requested(ctx).await {
                    tracing::info!(
                        mount_id = %ctx.mount.mount_id,
                        "stop requested; ending the delta pass at a page boundary"
                    );
                    break;
                }

                // Local edits noted while this pass has been running go out
                // here, under this run's lease (see `write::drain_pending`).
                super::write::drain_pending(ctx, state, batcher).await;
            }
            None => break,
        }

        // Termination, in order of authority:
        //
        // 1. The adapter said whether it has more pages. Trust it: a `false`
        //    means "caught up; the token is next run's resume point".
        // 2. Legacy adapters (no `has_more`): an EMPTY page cannot be forward
        //    progress, so stop. Token identity is NOT a usable signal here —
        //    Microsoft Graph mints a fresh delta token on every poll of an
        //    idle feed, so the old `next == token` check never fired and this
        //    loop spun empty pages at provider speed, committing the fresh
        //    cursor each round, until the watchdog killed the run (~600s),
        //    the lease leaked, and the scheduler re-enqueued — the observed
        //    "mount committed 4×/second with no sync running".
        // 3. The per-run item cap.
        // 4. The wall-clock budget — the same self-stop the full walk has,
        //    so a genuinely long delta ends as a clean truncated chunk with a
        //    durable cursor instead of a watchdog kill that skips finalize.
        match page.has_more {
            Some(false) => break,
            Some(true) => {}
            None => {
                if page.items.is_empty() || next == token {
                    break;
                }
            }
        }
        if processed >= max {
            break;
        }
        if ctx.out_of_time(chrono::Utc::now().timestamp()) {
            tracing::info!(
                mount_id = %ctx.mount.mount_id,
                processed,
                "wall-clock budget spent during delta; ending this pass cleanly at a \
                 durable cursor. The next run resumes from it."
            );
            break;
        }
        token = next;
    }
    batcher.flush().await?;
    // The final page carries no `next_token`, so it never reached the counter
    // update above — the reported figure was short by exactly the last page of
    // every catch-up. Fold in whatever is left over.
    state.delta_items_done = state
        .delta_items_done
        .saturating_add(materialized.saturating_sub(published));
    Ok(())
}

/// Stage one change: delete tombstones, upsert created/updated (after filters).
///
/// Deletes go into the SAME ordered buffer as upserts. Applying them eagerly
/// while upserts were buffered would reorder the page, so a `created X` followed
/// by `deleted X` would leave X alive.
async fn stage_change(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    change: &super::config::Change,
    include: &[String],
    exclude: &[String],
) -> std::result::Result<bool, AdapterError> {
    if change.kind == "deleted" {
        batcher.stage_delete(&change.item.external_id).await?;
        return Ok(true);
    }

    // Same resolution as the full walk — both paths MUST agree, or an item would
    // sit at one location after a backfill and another after a delta, and the
    // materializer would keep whichever it saw first.
    let templated =
        super::config::resolve_path_template(&ctx.mount.sync_config.path_template, &change.item);
    let rel_path = if let Some(p) = templated {
        p
    } else if change.relative_path.is_empty() {
        change.item.name.clone()
    } else {
        change.relative_path.clone()
    };
    // A nameless UPDATE is a real adapter fault, and it must fail HERE — as one
    // item, with its id — rather than by refusing to parse the page (which is
    // what a required `name` did) or by materialising at the mount path itself,
    // which would put the item on top of the mount's own folder.
    if rel_path.is_empty() {
        return Err(AdapterError::Transient(format!(
            "get_changes returned an item with no name and no relative_path              (external_id {}), so there is nowhere to put it",
            change.item.external_id
        )));
    }
    if !super::passes_filters(&rel_path, include, exclude) {
        return Ok(false);
    }
    // The delta path has no reconcile, so the staged child ids have nothing to
    // report to — dropped deliberately rather than threaded to a caller that
    // would ignore them.
    let _children = stage_item(ctx, batcher, &change.item, &rel_path).await?;
    Ok(true)
}
