//! Marking a mount whose CONFIG NODE cannot be parsed.
//!
//! Every other misconfiguration is recorded by `preflight::mark_misconfigured`,
//! which needs a parsed [`MountConfig`](super::config::MountConfig) — the one
//! thing this case does not have. A corrupt `write_config`, `sync_config` or
//! `state` blob therefore used to reach an operator through neither channel:
//! the periodic scan logged a `warn` and moved on (production runs at
//! `RUST_LOG=warn`, so at best one line in a log nobody reads), and the job
//! path failed with a validation error that wrote no mount state at all. The
//! console kept showing the last successful run, `status: "ok"`, a frozen
//! `last_attempt_at` and no error — while the mount had silently stopped
//! syncing in both directions.
//!
//! So this writes the same three fields `mark_misconfigured` writes, straight
//! onto the node's raw `state` object, without going through the typed
//! [`MountState`](super::config::MountState) — which by construction cannot be
//! deserialized here.

use std::sync::Arc;

use chrono::Utc;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::TransactionalStorage;
use serde_json::{Map, Value};

use super::config::{SYNC_ACTOR, SYSTEM_WORKSPACE};
use crate::RocksDBStorage;

/// Record `error` on the mount node as `status: "misconfigured"`.
///
/// Returns `true` when the node was written, `false` when it was already
/// carrying this exact verdict (or is not on this branch). The idempotence is
/// load-bearing, not an optimization: the periodic scan reaches this every 60
/// seconds for as long as the mount stays broken, and an unconditional write
/// would mint a node revision — replicated — on every tick, forever.
pub(super) async fn mark_unparseable_mount(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    config_branch: &str,
    mount_id: &str,
    error: &str,
) -> Result<bool> {
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(config_branch)?;
    tx.set_actor(SYNC_ACTOR)?;
    tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
    tx.set_message("virtual mount sync: mark misconfigured")?;
    // Engine-authored mount-node writes are bookkeeping (see
    // `state_store::persist_mount_state_detailed`); operator config edits via
    // the HTTP surface stay ordinary writes and fan out normally.
    tx.set_bookkeeping(true)?;

    let Some(mut node) = tx.get_node(SYSTEM_WORKSPACE, mount_id).await? else {
        return Ok(false);
    };

    let stored: Option<Value> = node
        .properties
        .get("state")
        .and_then(|pv| serde_json::to_value(pv).ok());

    let mut state: Map<String, Value> = match stored {
        // The ordinary case, including a `state` that is a perfectly good object
        // whose TYPED parse fails (a field of the wrong type) and every case
        // where the corruption is in some other property entirely. Merging
        // preserves the cursor, the backfill stack and `state_seq`.
        Some(Value::Object(map)) => map,
        // `state` is present but not even an object. There is nothing to merge
        // into and no way to keep it as `state`, so it is set aside verbatim
        // rather than dropped: refusing to default a corrupt blob is what stops
        // a mount silently forgetting its cursor, and quietly overwriting one
        // here would do exactly that under a different name.
        Some(other) => {
            let mut fresh = Map::new();
            fresh.insert("state_unparseable".to_string(), other);
            fresh
        }
        None => Map::new(),
    };

    let already = state.get("status").and_then(|v| v.as_str()) == Some("misconfigured")
        && state.get("last_error").and_then(|v| v.as_str()) == Some(error);
    if already {
        return Ok(false);
    }

    state.insert("status".to_string(), Value::String("misconfigured".into()));
    state.insert("last_error".to_string(), Value::String(error.to_string()));
    // Stamped for the same reason `mark_misconfigured` stamps it: this path
    // returns before `finalize`, and the console reads "when was this last
    // tried" from here.
    state.insert(
        "last_attempt_at".to_string(),
        Value::from(Utc::now().timestamp()),
    );
    // Advance the guard so a straggler writer holding an older `state_seq`
    // cannot quietly undo the verdict.
    let seq = state
        .get("state_seq")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .saturating_add(1);
    state.insert("state_seq".to_string(), Value::from(seq));

    let pv: PropertyValue = serde_json::from_value(Value::Object(state))
        .map_err(|e| raisin_error::Error::Validation(format!("state to PropertyValue: {e}")))?;
    node.properties.insert("state".to_string(), pv);
    tx.upsert_node(SYSTEM_WORKSPACE, &node).await?;
    tx.commit().await?;
    Ok(true)
}
