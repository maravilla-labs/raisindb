//! Carrying out a conflict resolution: the forced re-push, the merge, and the
//! stamp that follows either.
//!
//! Split from [`super::conflict`], which only DECIDES. Two files because the two
//! halves fail differently: a decision cannot touch the provider, and everything
//! here can — so every path below is written on the assumption that it may be
//! the second write this node has seen in one drain.
//!
//! The one rule shared by both re-push paths: they send `etag: null`. That is
//! not an oversight, it is what "resolve the conflict" means — the stored etag
//! is *known stale*, so re-sending it would be refused identically and forever.
//! Which is exactly why `local_wins` is never a default: an unconditional write
//! overwrites the other party's change with no record that it existed.

use raisin_models::nodes::Node;
use serde_json::{json, Map, Value};

use super::super::materializer::WriteView;
use super::super::{map_to_external, AdapterError, SyncBatcher, SyncCtx, ToExternalOutcome};
use super::conflict::Resolution;
use super::push::Pushed;

/// Everything the follow-through needs about the push that was refused.
pub(super) struct Refused<'a> {
    pub node: &'a Node,
    pub node_json: &'a Value,
    pub view: &'a WriteView,
    /// The remote object the refused call addressed.
    pub item_id: &'a str,
    pub external_id: &'a str,
    /// The payload the provider refused, replayed verbatim by `local_wins`.
    pub payload: &'a Value,
    /// The mount's push allow-list, as sent.
    pub fields: &'a [String],
    pub node_bytes: usize,
    /// The provider's own words, carried into every log line and park reason.
    pub message: &'a str,
}

/// Act on a resolved conflict.
pub(super) async fn apply(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    refused: &Refused<'_>,
    resolution: Resolution,
) -> std::result::Result<Pushed, AdapterError> {
    match resolution {
        Resolution::Abandon => {
            tracing::info!(
                mount_id = %ctx.scope.mount_id,
                external_id = %refused.external_id,
                node_id = %refused.node.id,
                reason = %refused.message,
                "the provider refused this push and the mount's conflict policy is \
                 'remote_wins'; the local edit is abandoned and the next delta will \
                 overwrite it with the remote value"
            );
            Ok(Pushed::Conflicted)
        }
        Resolution::Park(reason) => {
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                external_id = %refused.external_id,
                node_id = %refused.node.id,
                reason = %reason,
                "conflict left for a human; the edit stays pending and nothing was \
                 written at the provider"
            );
            Ok(Pushed::Parked(reason))
        }
        Resolution::ForceLocal => force(ctx, batcher, refused).await,
        Resolution::Merged(merged) => merge(ctx, batcher, refused, merged).await,
    }
}

/// `local_wins`: replay the identical payload with no concurrency base.
async fn force(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    refused: &Refused<'_>,
) -> std::result::Result<Pushed, AdapterError> {
    tracing::warn!(
        mount_id = %ctx.scope.mount_id,
        external_id = %refused.external_id,
        node_id = %refused.node.id,
        reason = %refused.message,
        "conflict policy is 'local_wins': re-sending this push unconditionally. \
         Whatever changed at the provider since this mount last read it is being \
         OVERWRITTEN and is not recoverable from here."
    );
    let result = send(ctx, refused.item_id, refused.payload, refused.fields).await?;
    let Some(result) = result else {
        return Ok(Pushed::Parked(format!(
            "conflict resolution for '{}' was refused a second time: {}",
            refused.external_id, refused.message
        )));
    };
    // Same "gone" guard as the first attempt: a non-object answer is not a
    // completed push, so nothing is baselined.
    if !result.is_object() {
        return Ok(Pushed::Gone);
    }
    let pushed = subset(&refused.view.watched, refused.fields);
    stamp(batcher, refused, &result, pushed, None).await?;
    Ok(Pushed::Sent)
}

/// A resolver's `merged`: push the merged values, then land them locally.
///
/// The local landing is what makes a merge different from a force. Without it
/// the provider holds the merged value and the node holds the user's original
/// one, `__pushed_state` says they agree, and the difference is invisible
/// forever — a silent divergence created by the very mechanism meant to end one.
async fn merge(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    refused: &Refused<'_>,
    merged: Map<String, Value>,
) -> std::result::Result<Pushed, AdapterError> {
    let keys: Vec<String> = merged.keys().cloned().collect();
    // Re-mapped rather than hand-built: the mapper is the only thing that knows
    // how a node property becomes a provider field (`unread` → `isRead`, and
    // inverted), and a second translation here is precisely the drift
    // `to_external` exists to prevent.
    let merged_node = with_properties(refused.node_json, &merged);
    let payload = match map_to_external(ctx, &merged_node, Some(&keys), "update").await? {
        ToExternalOutcome::Mapped(mapped) => mapped.payload,
        ToExternalOutcome::NotWritable | ToExternalOutcome::NoMapper => {
            return Ok(Pushed::Parked(format!(
                "conflict resolver merged {keys:?} for '{}' but the mapper declined to \
                 translate the result outward; the edit stays pending",
                refused.external_id
            )));
        }
    };
    let result = send(ctx, refused.item_id, &payload, &keys).await?;
    let Some(result) = result else {
        return Ok(Pushed::Parked(format!(
            "the merged push for '{}' was refused as a conflict in turn: {}",
            refused.external_id, refused.message
        )));
    };
    if !result.is_object() {
        return Ok(Pushed::Gone);
    }
    // EXTENDED, not replaced. A merge covers only the fields the resolver
    // returned, so replacing the whole baseline would erase what was agreed
    // about every other watched field and re-push all of them next drain.
    let mut pushed = refused.view.pushed.clone().unwrap_or_default();
    for (key, value) in &merged {
        pushed.insert(key.clone(), value.clone());
    }
    stamp(batcher, refused, &result, pushed, Some(merged)).await?;
    Ok(Pushed::Sent)
}

/// One unconditional `update`. `Ok(None)` means the provider refused it as a
/// conflict AGAIN, which the callers turn into a park — never another attempt.
/// A resolution that can re-enter itself is an unbounded write loop against
/// someone else's mailbox.
async fn send(
    ctx: &SyncCtx<'_>,
    item_id: &str,
    payload: &Value,
    fields: &[String],
) -> std::result::Result<Option<Value>, AdapterError> {
    match ctx
        .call(
            "update",
            json!({
                "item_id": item_id,
                "payload": payload,
                "fields": fields,
                "etag": Value::Null,
            }),
        )
        .await
    {
        Ok(v) => Ok(Some(v)),
        Err(AdapterError::Conflict(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The stamp-back after a resolution landed.
async fn stamp(
    batcher: &mut SyncBatcher<'_>,
    refused: &Refused<'_>,
    result: &Value,
    pushed: Map<String, Value>,
    merged: Option<Map<String, Value>>,
) -> std::result::Result<(), AdapterError> {
    let new_etag = result
        .get("etag")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| str_prop(refused.node, "__etag"));
    batcher
        .stage_stamp(
            &refused.node.id,
            refused.external_id,
            new_etag,
            Some(pushed),
            merged,
            refused.node_bytes,
        )
        .await
}

/// `node_json` with these property values overlaid.
fn with_properties(node_json: &Value, merged: &Map<String, Value>) -> Value {
    let mut out = node_json.clone();
    let props = out
        .as_object_mut()
        .and_then(|o| o.get_mut("properties"))
        .and_then(|p| p.as_object_mut());
    if let Some(props) = props {
        for (key, value) in merged {
            props.insert(key.clone(), value.clone());
        }
    }
    out
}

/// The watched values for exactly these fields.
fn subset(watched: &Map<String, Value>, fields: &[String]) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        if let Some(v) = watched.get(field) {
            out.insert(field.clone(), v.clone());
        }
    }
    out
}

fn str_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        raisin_models::nodes::properties::PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}
