//! Mapping one external item to a node shape and staging it.

use super::*;

/// Map one external item to a node, honoring a custom mapping function.
/// Returns `None` when the mapper filters the item out.
pub async fn map_item(
    ctx: &SyncCtx<'_>,
    item: &ExternalItem,
) -> std::result::Result<Option<MappedNode>, AdapterError> {
    let Some(mapper) = &ctx.mount.mapping_function else {
        return Ok(Some(default_mapping(item)));
    };
    let input = json!({
        "external_item": item,
        "mount": ctx.mount_snapshot,
    });
    let out = ctx.invoker.invoke(&ctx.scope, mapper, input).await?;
    if out.is_null() {
        return Ok(None);
    }
    let node_type = out
        .get("node_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AdapterError::Transient("mapping missing node_type".to_string()))?
        .to_string();
    let name = out
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let properties = out
        .get("properties")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    Ok(Some(MappedNode {
        node_type,
        name,
        properties,
    }))
}

/// Stage one item (map → buffer) at `rel_path`. The batcher decides when the
/// buffered work is actually written.
///
/// An item whose etag already matches what is stored is dropped BEFORE the
/// mapper runs: the skip decision needs only `external_id` + `etag`, both of
/// which the provider supplied, so mapping it first would spend a QuickJS
/// invocation to produce a value that is immediately discarded. On a re-sync of
/// an unchanged mailbox that was the entire cost of the run.
pub async fn stage_item(
    ctx: &SyncCtx<'_>,
    batcher: &mut batch::SyncBatcher<'_>,
    item: &ExternalItem,
    rel_path: &str,
) -> std::result::Result<(), AdapterError> {
    if batcher.can_skip_unmapped(item) {
        batcher.note_unmapped_skip();
        return Ok(());
    }
    let Some(mapped) = map_item(ctx, item).await? else {
        return Ok(());
    };
    batcher.stage_upsert(item, rel_path, mapped).await
}
