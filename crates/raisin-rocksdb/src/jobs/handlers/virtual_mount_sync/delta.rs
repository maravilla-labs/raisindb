//! Delta (incremental) sync driven by the adapter's `get_changes` feed.
//!
//! Pages until `next_token` stabilizes or `max_items_per_sync` is reached,
//! materializing each change, and persists the cursor **only after** a page is
//! fully materialized (so a crashed page re-runs safely — upserts are
//! idempotent via etag skip-write).

use serde_json::json;

use super::batch::SyncBatcher;
use super::config::{ChangesPage, MountState};
use super::{stage_item, persist_state, AdapterError, SyncCtx};

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

    loop {
        let params = json!({
            "since_token": token,
            "folder_id": ctx.mount.remote_root,
        });
        let resp = ctx.call("get_changes", params).await?;
        let page: ChangesPage = serde_json::from_value(resp)
            .map_err(|e| AdapterError::Transient(format!("bad get_changes response: {e}")))?;

        for change in &page.items {
            stage_change(ctx, batcher, change, include, exclude).await?;
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
                let _ = persist_state(ctx, state).await;
                ctx.renew_lease(state.last_fencing_token).await;
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
) -> std::result::Result<(), AdapterError> {
    if change.kind == "deleted" {
        batcher.stage_delete(&change.item.external_id).await?;
        return Ok(());
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
        return Ok(());
    }
    stage_item(ctx, batcher, &change.item, &rel_path).await?;
    Ok(())
}
