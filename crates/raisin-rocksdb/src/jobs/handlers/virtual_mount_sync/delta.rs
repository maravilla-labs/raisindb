//! Delta (incremental) sync driven by the adapter's `get_changes` feed.
//!
//! Pages until `next_token` stabilizes or `max_items_per_sync` is reached,
//! materializing each change, and persists the cursor **only after** a page is
//! fully materialized (so a crashed page re-runs safely — upserts are
//! idempotent via etag skip-write).

use serde_json::json;

use super::batch::SyncBatcher;
use super::config::{ChangesPage, MountState};
use super::{persist_state, stage_item, AdapterError, SyncCtx};

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

    loop {
        let params = json!({
            "since_token": token,
            "folder_id": ctx.mount.remote_root,
            "baseline_only": want_baseline,
        });
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
                state.delta_items_done = state.delta_items_done.saturating_add(materialized);
                if !persist_state(ctx, state).await.unwrap_or(false) {
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
            }
            None => break,
        }

        // Stop when the cursor stabilizes (no forward progress) or the cap is hit.
        if next == token || processed >= max {
            break;
        }
        token = next;
    }
    batcher.flush().await?;
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
    if !super::passes_filters(&rel_path, include, exclude) {
        return Ok(false);
    }
    stage_item(ctx, batcher, &change.item, &rel_path).await?;
    Ok(true)
}
