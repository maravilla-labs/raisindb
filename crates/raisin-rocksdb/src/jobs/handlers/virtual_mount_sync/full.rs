//! Full-listing reconcile: first sync, `supports_changes:false`, or `mode:full`.
//!
//! Recursively `list`s from `remote_root`, upserts everything (after filters),
//! then deletes any mount-owned virtual node not seen this pass. Finally tries
//! to establish a delta baseline token so subsequent syncs can go incremental.

use std::collections::HashSet;

use serde_json::json;

use super::config::{ListPage, MountState};
use super::{materialize_item, persist_state, AdapterError, SyncCtx};

/// Run a full reconcile.
pub async fn run(
    ctx: &SyncCtx<'_>,
    state: &mut MountState,
) -> std::result::Result<(), AdapterError> {
    let max = ctx.mount.sync_config.max_items_per_sync;
    let include = &ctx.mount.sync_config.include_patterns;
    let exclude = &ctx.mount.sync_config.exclude_patterns;

    let mut seen: HashSet<String> = HashSet::new();
    let mut processed: u64 = 0;
    // Set when the walk stops before exhausting the provider, which makes
    // `seen` a PARTIAL picture and therefore unusable for deciding what to
    // delete. See the reconcile guard below.
    let mut truncated = false;

    // Explicit stack of (folder_id, rel_prefix) to avoid async recursion.
    let root = ctx.mount.remote_root.clone();
    let mut stack: Vec<(Option<String>, String)> = vec![(root, String::new())];

    'outer: while let Some((folder_id, prefix)) = stack.pop() {
        let mut cursor: Option<String> = None;
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
                let rel_path = if prefix.is_empty() {
                    item.name.clone()
                } else {
                    format!("{}/{}", prefix, item.name)
                };
                if super::passes_filters(&rel_path, include, exclude) {
                    materialize_item(ctx, item, &rel_path).await?;
                    seen.insert(item.external_id.clone());
                    processed += 1;
                }
                if item.is_folder {
                    stack.push((Some(item.external_id.clone()), rel_path));
                }
            }

            ctx.renew_lease(state.last_fencing_token).await;
            match page.next_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
            if processed >= max {
                truncated = true;
                break 'outer;
            }
        }
    }

    // Reconcile deletes: remove mount-owned nodes not seen this pass.
    //
    // "Not seen" only means "deleted upstream" when this pass actually SAW
    // everything. Two ways that is false, and both used to delete real content:
    //
    //  1. The walk hit `max_items_per_sync` (default 500) and stopped early. A
    //     mailbox larger than the cap would have every message past item 500
    //     deleted on each full sync, then re-created by the next one — churn,
    //     a trigger storm, and destroyed local edits, forever.
    //  2. The provider answered with an empty listing (a silent hiccup, an
    //     emptied-then-refilled folder, a permissions change that reads as
    //     "no items" rather than an error). Deleting the entire mount subtree
    //     on the strength of one empty response is never the right trade.
    //
    // In both cases the safe action is to skip reconciliation entirely: a stale
    // extra node is recoverable on the next clean pass, deleted content is not.
    let existing = ctx
        .materializer
        .list_virtual(&ctx.scope)
        .await
        .map_err(|e| AdapterError::Transient(format!("list_virtual failed: {e}")))?;

    if truncated {
        tracing::warn!(
            mount_id = %ctx.mount.mount_id,
            processed,
            max_items_per_sync = max,
            existing = existing.len(),
            "full sync hit max_items_per_sync; skipping reconcile deletes because the listing is \
             incomplete. Raise max_items_per_sync above the item count for deletes to be applied."
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
                ctx.materializer
                    .delete(&ctx.scope, &node.external_id)
                    .await
                    .map_err(|e| {
                        AdapterError::Transient(format!("reconcile delete failed: {e}"))
                    })?;
                deleted += 1;
            }
        }
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
    if let Ok(resp) = ctx
        .call(
            "get_changes",
            json!({ "since_token": null, "folder_id": ctx.mount.remote_root }),
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
