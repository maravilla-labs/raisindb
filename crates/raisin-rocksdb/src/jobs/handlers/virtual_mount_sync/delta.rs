//! Delta (incremental) sync driven by the adapter's `get_changes` feed.
//!
//! Pages until `next_token` stabilizes or `max_items_per_sync` is reached,
//! materializing each change, and persists the cursor **only after** a page is
//! fully materialized (so a crashed page re-runs safely — upserts are
//! idempotent via etag skip-write).

use serde_json::json;

use super::config::{ChangesPage, MountState};
use super::{materialize_item, persist_state, AdapterError, SyncCtx};

/// Run a delta sync, updating `state.last_sync_token` as pages complete.
pub async fn run(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
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
            apply_change(ctx, change, include, exclude).await?;
            processed += 1;
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
    Ok(())
}

/// Apply one change: delete tombstones, upsert created/updated (after filters).
async fn apply_change(
    ctx: &SyncCtx<'_>,
    change: &super::config::Change,
    include: &[String],
    exclude: &[String],
) -> std::result::Result<(), AdapterError> {
    if change.kind == "deleted" {
        ctx.materializer
            .delete(&ctx.scope, &change.item.external_id)
            .await
            .map_err(|e| AdapterError::Transient(format!("delete failed: {e}")))?;
        return Ok(());
    }

    let rel_path = if change.relative_path.is_empty() {
        change.item.name.clone()
    } else {
        change.relative_path.clone()
    };
    if !super::passes_filters(&rel_path, include, exclude) {
        return Ok(());
    }
    materialize_item(ctx, &change.item, &rel_path).await?;
    Ok(())
}
