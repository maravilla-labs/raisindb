//! What an outbox drain leaves behind: the outward translation it starts from,
//! and the stamps a completed command carries.
//!
//! Split out of [`super::submit`] to keep both files readable, not because the
//! two halves are independent — `sent_stamp` is what makes a completed command
//! visible to the mount's TTL cleanup, and getting it wrong leaves a permanently
//! growing outbox with nothing reporting it.

use chrono::Utc;
use serde_json::{json, Map, Value};

use super::super::{map_to_external, SyncCtx, ToExternalOutcome};

/// Reverse-map a command, or say why it cannot be issued.
///
/// Runs BEFORE the claim, deliberately. `to_external` is pure and I/O-free, so
/// nothing is lost by asking first — and asking after the claim would strand a
/// command in `sending`, and then at `unknown`, for a reason that has nothing to
/// do with the provider and everything to do with the mount's own mapper.
pub(super) async fn map_command(
    ctx: &SyncCtx<'_>,
    node: &raisin_models::nodes::Node,
) -> Result<Option<super::super::ToExternal>, String> {
    let node_json =
        serde_json::to_value(node).map_err(|e| format!("command serialize failed: {e}"))?;
    // `fields: None` on purpose. A field allow-list is what makes a `state_only`
    // push a PATCH; a command is submitted whole, and narrowing it would send a
    // mail with its recipients filtered out.
    match map_to_external(ctx, &node_json, None, "submit").await {
        Ok(ToExternalOutcome::Mapped(mapped)) => Ok(Some(mapped)),
        Ok(ToExternalOutcome::NotWritable) => Ok(None),
        Ok(ToExternalOutcome::NoMapper) => Ok(None),
        Err(e) => Err(format!("mapper failed: {e}")),
    }
}

/// The reserved metadata a completed command is stamped with.
///
/// This is what hands it to the mount's `ephemeral` + `ttl_seconds` cleanup,
/// which is the whole garbage-collection story for an outbox. `__external_id`
/// falls back to a value derived from the node id because most providers answer
/// a send with no id at all — Graph's `sendMail` is a 202 with an empty body —
/// and an unstamped node would sit in the outbox forever.
pub(super) fn sent_stamp(
    node_id: &str,
    mount_id: &str,
    external_id: Option<&str>,
    etag: Option<&str>,
) -> Map<String, Value> {
    let now = Utc::now().to_rfc3339();
    let mut set = Map::new();
    set.insert("sent_at".into(), json!(now));
    set.insert("__virtual".into(), json!(true));
    set.insert("__mount_id".into(), json!(mount_id));
    set.insert("__synced_at".into(), json!(now));
    set.insert(
        "__external_id".into(),
        json!(external_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("cmd:{node_id}"))),
    );
    if let Some(external_id) = external_id {
        set.insert("sent_external_id".into(), json!(external_id));
    }
    if let Some(etag) = etag {
        set.insert("__etag".into(), json!(etag));
    }
    // Cleared, not left: a command that failed, was fixed and then sent must not
    // keep displaying the error that no longer applies.
    set.insert("last_error".into(), Value::Null);
    set
}
