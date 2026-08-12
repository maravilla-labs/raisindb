//! Pushing ONE node: the exact sequence, including the two checks that make the
//! loop terminate.

use serde_json::{json, Map, Value};

use super::super::materializer::{estimate_node_bytes, write_view_of};
use super::super::{map_to_external, AdapterError, SyncBatcher, SyncCtx, ToExternalOutcome};
use super::candidates::Candidate;
use super::conflict::{self, ConflictPolicy};
use super::fields::FieldPlan;

/// What acting on one candidate did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pushed {
    /// The adapter's `update` was called and the result stamped back.
    Sent,
    /// Nothing to do: converged or declined by the mapper.
    Skipped,
    /// The adapter answered "this object no longer exists at the provider"
    /// (a `null` / non-object result). The edit was NOT applied and is NOT
    /// baselined — see [`push_one`] step 6.
    Gone,
    /// `move_policy: reject` and this node's location field has moved. Nothing
    /// was sent — not even the fields that could have gone — and nothing was
    /// stamped, so the node stays diverged and says so again next drain.
    Rejected,
    /// The provider refused the push (the object changed since this mount read
    /// it) and the mount's conflict policy ABANDONED the edit — `remote_wins`,
    /// the default. Nothing was stamped, so the next delta overwrites the local
    /// value with the remote one.
    ///
    /// Not a failure (the provider answered) and not a skip (an edit was lost),
    /// which is why it is neither.
    Conflicted,
    /// The conflict was left for a human: `conflict: "error"`, or a resolver
    /// that parked, threw, or answered something this engine does not
    /// understand. The edit stays pending and the mount goes to
    /// `writeback_status: "conflict"`.
    Parked(String),
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
    plan: &FieldPlan,
    policy: &ConflictPolicy,
) -> std::result::Result<Pushed, AdapterError> {
    let fields = &plan.push;
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
    // 2b. A derived recurrence occurrence is not writable, whatever the mount
    //     thinks. `Config` rather than `Transient` on purpose: the request can
    //     never succeed as written, so retrying it every tick would be a
    //     permanent log of an error the operator can actually fix — by editing
    //     the series master or adding an exception node. The drain's candidate
    //     selection already filters the projection root; this catches a
    //     projection node that landed anywhere else.
    if let Some(reason) = crate::jobs::handlers::calendar_expand::node_refusal(&node) {
        return Err(AdapterError::Config(reason));
    }
    // The view is derived over the WHOLE effective list, moves included, so the
    // reject check below can see a field that `plan.push` deliberately withheld.
    // Deriving it over `plan.push` alone would make a rejected move invisible to
    // the very check that exists to refuse it.
    let watched: Vec<String> = fields.iter().chain(plan.moves.iter()).cloned().collect();
    let Some(view) = write_view_of(&node, &watched) else {
        return Ok(Pushed::Skipped);
    };

    // 2c. `move_policy: reject`. Refused BEFORE the converge check and before
    //     the mapper, because the answer does not depend on either: while the
    //     location field disagrees with what the provider was last told, this
    //     mount does not write this node at all. Withholding the whole update
    //     rather than just the move field is the point — see
    //     [`super::policy::MovePolicy::Reject`].
    if plan.policy == super::policy::MovePolicy::Reject && plan.moved(&view) {
        tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            external_id = %candidate.external_id,
            node_id = %node.id,
            path = %node.path,
            fields = ?plan.moves,
            "move_policy is 'reject' and this node's location field has changed locally; \
             the whole update is withheld. The local move stands — nothing moves it back — \
             so the node stays diverged from the provider until it is moved back or the \
             policy is changed."
        );
        return Ok(Pushed::Rejected);
    }
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

    // Push ONLY what actually changed, never the whole allow-list. Some
    // provider fields have side effects on mere PRESENCE in an update —
    // Microsoft Graph resends meeting invites to every attendee whenever
    // `attendees` appears in a PATCH — so an update triggered by a title edit
    // must not carry the attendee list along just because the mount permits
    // attendee edits. `diverges()` above guarantees this is non-empty.
    let push_fields = view.diverged_fields(fields);
    let push_fields = &push_fields;

    // 4. Reverse-map through the mount's own mapper, never inside the adapter:
    //    a custom mapper that reshapes nodes must be the one that reshapes them
    //    back, or the two translations of one relationship drift.
    let node_json = serde_json::to_value(&node)
        .map_err(|e| AdapterError::Transient(format!("node serialize failed: {e}")))?;
    let payload = match map_to_external(ctx, &node_json, Some(push_fields), "update").await? {
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
    //
    //    A `conflict` is the ONE failure that does not simply propagate. It is
    //    the provider's answer, not a fault, and what it means is a decision the
    //    engine cannot make: see [`super::conflict`]. Everything else — auth,
    //    rate limits, config errors — is still returned raw, because the
    //    conflict policy has nothing to say about them and pretending otherwise
    //    would route a broken credential through a merge function.
    let result = match ctx
        .call(
            "update",
            json!({
                "item_id": item_id,
                "payload": payload.payload,
                "fields": push_fields,
                "etag": stored_etag,
            }),
        )
        .await
    {
        Ok(result) => result,
        Err(e) if conflict::is_conflict(&e) => {
            let message = e.to_string();
            let resolution = conflict::resolve_one(
                ctx,
                policy,
                &node_json,
                // The mount's EFFECTIVE allow-list, not this attempt's diverged
                // subset. `push_fields` is what went on the wire; `fields` is
                // what this mount may write at all, and it is the latter that
                // bounds what a resolver is allowed to decide — see
                // `resolve_one`, where passing the wrong one silently truncated
                // the resolver's answer.
                fields,
                &view.watched,
                view.pushed.as_ref(),
                stored_etag.as_deref(),
                &message,
            )
            .await;
            return super::conflict_push::apply(
                ctx,
                batcher,
                &super::conflict_push::Refused {
                    node: &node,
                    node_json: &node_json,
                    view: view.as_ref(),
                    item_id: &item_id,
                    external_id: &candidate.external_id,
                    payload: &payload.payload,
                    fields: push_fields,
                    node_bytes,
                    message: &message,
                },
                resolution,
            )
            .await;
        }
        Err(e) => return Err(e),
    };

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
            // The stamp REPLACES `__pushed_state` wholesale, and this push
            // carried only the diverged subset — so the new map must merge the
            // just-sent values OVER the existing baseline. Stamping the subset
            // alone would drop every converged field's baseline, making them
            // all "diverged" again next drain: a perpetual re-push, and for a
            // presence-side-effect field like `attendees` the exact invite
            // spam the diverged-subset push exists to prevent.
            Some(pushed_map(view.pushed.as_ref(), &view.watched, push_fields)),
            // Not a merge: an ordinary push writes no node properties, only the
            // engine's own metadata.
            None,
            node_bytes,
        )
        .await?;
    Ok(Pushed::Sent)
}

/// The prior baseline with the values actually sent merged over it.
///
/// Two rules meet here. The stamp must record the values SENT — a field the
/// mount watches but did not push this time must not be recorded as pushed, or
/// enabling it later would look like it had already converged. And it must not
/// LOSE the baseline of fields pushed on earlier drains: the stamp replaces
/// `__pushed_state` wholesale, so the prior map is carried forward and only
/// the just-sent fields are overwritten.
fn pushed_map(
    prior: Option<&Map<String, Value>>,
    watched: &Map<String, Value>,
    sent: &[String],
) -> Map<String, Value> {
    let mut out = prior.cloned().unwrap_or_default();
    for field in sent {
        match watched.get(field) {
            Some(v) => {
                out.insert(field.clone(), v.clone());
            }
            // Sent as an absence (the local edit removed the value): the
            // baseline must record the absence too.
            None => {
                out.remove(field);
            }
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
