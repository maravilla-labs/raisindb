//! Pushing ONE node: the exact sequence, including the two checks that make the
//! loop terminate.

use serde_json::{json, Map, Value};

use super::super::materializer::{estimate_node_bytes, write_view_of};
use super::super::{map_to_external, AdapterError, SyncBatcher, SyncCtx, ToExternalOutcome};
use super::plan::Candidate;

/// What acting on one candidate did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pushed {
    /// The adapter's `update` was called and the result stamped back.
    Sent,
    /// Nothing to do: converged or declined by the mapper.
    Skipped,
    /// The adapter answered "this object no longer exists at the provider"
    /// (a `null` / non-object result). The edit was NOT applied and is NOT
    /// baselined — see [`push_one`] step 6.
    Gone,
}

/// Claim → read current → converge → reverse-map → `update` → stamp back.
///
/// The stamp-back is what breaks the echo loop, and it is why this returns
/// after staging rather than after writing: the stamp goes through the run's
/// batcher, so it is committed before the delta phase runs and the item comes
/// back carrying the etag the provider assigned it. When the delta does deliver
/// it, `can_skip_unmapped` matches the stored etag and the item is dropped
/// before the mapper is even invoked.
pub(super) async fn push_one(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    candidate: &Candidate,
    fields: &[String],
) -> std::result::Result<Pushed, AdapterError> {
    // 2. Read the node as it stands NOW. The index entry that nominated it was
    //    read at the top of the run and the drain may have spent minutes in the
    //    provider since; pushing the nominating snapshot would re-send an edit
    //    the user has since undone.
    let node = ctx
        .materializer
        .read_node(&ctx.scope, &candidate.node_id)
        .await
        .map_err(|e| AdapterError::Transient(format!("read node for writeback failed: {e}")))?;
    let Some(node) = node else {
        return Ok(Pushed::Skipped);
    };
    let Some(view) = write_view_of(&node, fields) else {
        return Ok(Pushed::Skipped);
    };
    // A stamp re-writes this node WHOLE, so this is what the batch's byte
    // budget must charge for it. Measured here, from the node already in hand,
    // because the batcher never sees it. A drain over an existing mailbox stamps
    // every node it pushes, so without this a drain flushes on item count only
    // and packs a whole run's mail bodies into one replication record.
    let node_bytes = estimate_node_bytes(&node);

    // 3. Converge check. This is what makes the drain idempotent — and
    //    idempotence, not precise change detection, is what the whole design
    //    rests on. Running the drain twice, or handing it a noisy candidate
    //    list, must cost at most one provider call per real edit.
    //
    //    Decided against the node just read, never against the candidate flag:
    //    the index that set that flag is a snapshot, and a mount syncing while
    //    an operator edits is exactly when the two disagree.
    //
    //    A node with no `__pushed_state` at all (one that predates writeback on
    //    this mount) is NOT baselined here. Stamping its current LOCAL values as
    //    "what the provider has" is an assertion the engine cannot make: the
    //    remote values are not in scope at this point and there is no adapter
    //    `get` to fetch them, so the only thing available to stamp is the node's
    //    own properties — including any edit the user made before writeback was
    //    turned on. Recording that as already-pushed makes `diverges()` false
    //    forever and the edit is never sent: silent loss, with
    //    `writeback_supported: true` and zero failures.
    //
    //    So an unseeded node simply falls through to the divergence check, where
    //    a watched field present locally counts as diverged and is PUSHED. The
    //    cost is one provider write per pre-existing node the first time
    //    writeback is enabled (bounded by `max_items_per_sync` per run) — the
    //    burst rollout guidance must warn about. It is bounded, one-off,
    //    idempotent, and it is the only variant that cannot lose an edit. The
    //    push carries the stored `__etag` as its concurrency base, so a node
    //    whose remote moved since the last sync is rejected by the provider
    //    rather than overwritten with a stale local value.
    if !view.diverges(fields) {
        return Ok(Pushed::Skipped);
    }

    // 4. Reverse-map through the mount's own mapper, never inside the adapter:
    //    a custom mapper that reshapes nodes must be the one that reshapes them
    //    back, or the two translations of one relationship drift.
    let node_json = serde_json::to_value(&node)
        .map_err(|e| AdapterError::Transient(format!("node serialize failed: {e}")))?;
    let payload = match map_to_external(ctx, &node_json, Some(fields)).await? {
        ToExternalOutcome::Mapped(mapped) => mapped,
        // Not an error: the mapper declining this node is an answer. Nothing is
        // stamped either, so if the mapper is fixed the edit is still pending.
        ToExternalOutcome::NotWritable | ToExternalOutcome::NoMapper => {
            tracing::debug!(
                mount_id = %ctx.scope.mount_id,
                external_id = %candidate.external_id,
                "mapper declined to translate this node outward; not pushing"
            );
            return Ok(Pushed::Skipped);
        }
    };

    // 5. Call the provider. `etag` is the optimistic-concurrency base: an
    //    adapter whose provider disagrees throws `conflict` rather than
    //    overwriting, which the caller resolves by the mount's policy.
    let stored_etag = str_prop(&node, "__etag");
    let item_id = payload
        .external_id
        .clone()
        .unwrap_or_else(|| candidate.external_id.clone());
    let result = ctx
        .call(
            "update",
            json!({
                "item_id": item_id,
                "payload": payload.payload,
                "fields": fields,
                "etag": stored_etag,
            }),
        )
        .await?;

    // 6. An adapter may answer "there is nothing here to update" — the MS Graph
    //    adapter returns `null` on a 404 because Graph message ids are not
    //    stable (a message that moves folders gets a new id) and a stale id
    //    must not mark a healthy mount misconfigured.
    //
    //    That answer is NOT a completed push, and this guard is the difference
    //    between the two. Without it `result.get("etag")` on `Value::Null` is
    //    `None`, `new_etag` falls back to the STORED etag, and the stamp below
    //    records the LOCAL values as "what the provider has" — the same
    //    assertion-the-engine-cannot-make that the seed path was fixed to stop
    //    making, re-entered through the adapter. The flag would never reach the
    //    provider, `diverges()` would answer false forever, and the run would
    //    report `pushed: 1, failed: 0`.
    //
    //    So: no stamp, no baseline. The node stays diverged, which is honest —
    //    the edit really is unsent. It costs one (cheap, id-only) provider call
    //    per drain until the node goes away, which a full reconcile does: the
    //    item is no longer reported under this external id, so it is pruned,
    //    and the delta re-imports it under the id it now has.
    if !result.is_object() {
        tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            external_id = %candidate.external_id,
            node_id = %node.id,
            "the provider no longer has this object; the local edit was NOT applied \
             and is deliberately left un-baselined so it is not recorded as pushed"
        );
        return Ok(Pushed::Gone);
    }

    // 7. Stamp back, as the sync actor, in the run's own batch.
    let new_etag = result
        .get("etag")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or(stored_etag);
    batcher
        .stage_stamp(
            &node.id,
            &candidate.external_id,
            new_etag,
            Some(pushed_map(&view.watched, fields)),
            node_bytes,
        )
        .await?;
    Ok(Pushed::Sent)
}

/// The values actually sent, which is what `__pushed_state` must record — not
/// the whole watched map. A field the mount watches but did not push this time
/// (because the adapter does not accept it) must not be recorded as pushed, or
/// enabling it later would look like it had already converged.
fn pushed_map(watched: &Map<String, Value>, fields: &[String]) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        if let Some(v) = watched.get(field) {
            out.insert(field.clone(), v.clone());
        }
    }
    out
}

fn str_prop(node: &raisin_models::nodes::Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        raisin_models::nodes::properties::PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}
