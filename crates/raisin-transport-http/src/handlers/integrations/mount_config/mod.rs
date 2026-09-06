// SPDX-License-Identifier: BSL-1.1

//! Field-scoped edits to a virtual mount's `sync_config`.
//!
//! # Why this is not the generic node API
//!
//! A mount node's `state` is rewritten by the sync engine on every run and by
//! every inbound webhook, and it carries `last_sync_token` (the delta cursor),
//! `push_subscription_id` and the backfill resume point. A save that PUTs the
//! whole property map — which is what the generic node API does, and what the
//! console's mount editor does with a page-load-old copy — can therefore throw
//! away a cursor written seconds ago. Losing it costs a full re-walk of the
//! mount, or a push subscription the provider keeps delivering to nobody.
//!
//! So this endpoint merges NAMED KEYS into `sync_config` and touches nothing
//! else, exactly as `mount_control.rs`'s pause/stop merge named keys into
//! `state`. The allow-list lives in [`fields`]; there is no key in it that can
//! reach `state`, `integration_ref`, `account_ref` or `remote_root`.
//!
//! # Concurrency
//!
//! The same protection pause/stop use, which is to say: none beyond a
//! server-side read-modify-write. There is no compare-and-swap on a node write,
//! so a sync that persists `state` between this handler's read and its write
//! still loses that write. Two things make it acceptable where the full-map save
//! is not. The read happens microseconds before the write, inside one request,
//! rather than being a copy the browser has held since the page loaded — the
//! window goes from minutes to a round trip. And the loser of the race is
//! bounded: this handler re-writes `state` byte-for-byte as it read it, so the
//! worst case is one lost cursor advance, which the next run re-derives. It is
//! NOT a fix for the race; it is the same exposure the existing narrow endpoints
//! accept, and the real fix (a versioned property write) belongs beneath all
//! three of them rather than in one.

mod fields;
mod followup;
mod validate;

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use serde::Serialize;
use serde_json::{Map, Value};

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Actor recorded on the config write.
const ACTOR: &str = "integration-mount-config";
/// NodeType guard, so this route cannot be aimed at an arbitrary node.
const MOUNT_NODE_TYPE: &str = "raisin:VirtualMount";

/// Result of a `sync_config` patch.
#[derive(Debug, Serialize)]
pub struct SyncConfigResponse {
    pub ok: bool,
    /// The keys this request actually changed, in request order. A key sent
    /// with the value it already had is still reported — the endpoint does not
    /// diff, and claiming a no-op would be a claim it cannot make.
    pub changed: Vec<String>,
    /// The whole `sync_config` as it now stands, so the caller re-renders from
    /// the server's answer rather than from its own optimistic copy.
    pub sync_config: Value,
    /// What has to be RUN for this change to reach items that are already
    /// synced, when the change does not reach them on its own.
    ///
    /// Advisory: the endpoint enqueues nothing. A remap re-materializes every
    /// item and writes a revision each — the cost the etag skip exists to avoid
    /// — so it is a choice the operator makes, not a side effect of saving.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<followup::FollowUp>,
}

/// `PATCH /api/integrations/{repo}/mounts/{mount_id}/sync-config`
///
/// Body is a flat object of `sync_config` keys. An ABSENT key is unchanged; an
/// explicit `null` CLEARS the key, and only `ttl_seconds` and
/// `content_ttl_seconds` accept one — for every other field "cleared" and
/// "default" are the same value, so a null would be a second spelling of a
/// state that already has one.
///
/// Nothing is written unless every key validates. A patch that is half applied
/// would leave a mount in a combination neither the operator nor the validator
/// chose.
pub async fn patch_sync_config(
    State(state): State<AppState>,
    Path((repo, mount_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(body): Json<Value>,
) -> Result<Json<SyncConfigResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let patch = body.as_object().ok_or_else(|| {
        ApiError::validation_failed("body must be a JSON object of sync_config keys")
    })?;
    if patch.is_empty() {
        return Err(ApiError::validation_failed(
            "no fields to change; send at least one sync_config key",
        ));
    }

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let mut node = svc
        .get(&mount_id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(mount_id.to_string()))?;
    if node.node_type != MOUNT_NODE_TYPE {
        return Err(ApiError::validation_failed(format!(
            "node `{mount_id}` is a {}, not a {MOUNT_NODE_TYPE}",
            node.node_type
        )));
    }

    let mut config = object_prop(&node, "sync_config");
    let changed = apply(&mut config, patch)?;

    // The connector's cached capabilities, so a setting the adapter cannot
    // honour is refused HERE rather than becoming a mount that reads as
    // configured and does nothing. Absent when the connector has never been
    // probed, which the validator treats as "cannot" — never as "may".
    let caps = capabilities(&state, &tenant.tenant_id, &repo, &node).await;
    validate::check(&config, &changed, caps.as_ref()).map_err(ApiError::validation_failed)?;

    let value = Value::Object(config);
    let property = serde_json::from_value(value.clone())
        .map_err(|e| ApiError::internal(format!("failed to encode sync_config: {e}")))?;
    node.properties.insert("sync_config".to_string(), property);
    svc.update_node(node).await?;

    tracing::info!(
        mount_id = %mount_id,
        fields = %changed.join(","),
        "virtual mount sync_config updated"
    );

    let follow_up = followup::for_changes(&changed);
    Ok(Json(SyncConfigResponse {
        ok: true,
        changed,
        sync_config: value,
        follow_up,
    }))
}

/// Merge the patch into `config`, returning the keys it touched.
///
/// Every key is checked before any is applied, so a rejected patch leaves the
/// mount exactly as it was.
fn apply(
    config: &mut Map<String, Value>,
    patch: &Map<String, Value>,
) -> Result<Vec<String>, ApiError> {
    let mut staged: Vec<(String, Value)> = Vec::with_capacity(patch.len());
    for (key, value) in patch {
        if let Some(why) = fields::refusal(key) {
            return Err(ApiError::validation_failed(format!(
                "`{key}` cannot be set here: {why}"
            )));
        }
        let Some(field) = fields::writable(key) else {
            return Err(ApiError::validation_failed(format!(
                "`{key}` is not an editable sync_config field. Editable here: {}. Connector-specific \
                 keys and anything outside sync_config are set in the mount editor",
                fields::WRITABLE
                    .iter()
                    .map(|f| f.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        let normalized = fields::validate(field, value).map_err(ApiError::validation_failed)?;
        staged.push((key.clone(), normalized));
    }

    let mut changed = Vec::with_capacity(staged.len());
    for (key, value) in staged {
        if value.is_null() {
            config.remove(&key);
        } else {
            config.insert(key.clone(), value);
        }
        changed.push(key);
    }
    Ok(changed)
}

/// Read a node property as a JSON object, defaulting to empty.
fn object_prop(node: &Node, key: &str) -> Map<String, Value> {
    node.properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default()
}

/// The connector's cached `capabilities` blob, or `None` when the connector
/// cannot be resolved or has never been probed.
///
/// `integration_ref` may be a path or a node id — the sync engine tries both, so
/// this must too, or the two disagree about which connector a mount points at.
async fn capabilities(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    mount: &Node,
) -> Option<Value> {
    let integration_ref = match mount.properties.get("integration_ref") {
        Some(raisin_models::nodes::properties::PropertyValue::String(s)) if !s.is_empty() => {
            s.clone()
        }
        _ => return None,
    };
    let svc = super::config_service(state, tenant_id, repo, ACTOR);
    let node = match svc.get_by_path(&integration_ref).await {
        Ok(Some(n)) => n,
        _ => svc.get(&integration_ref).await.ok().flatten()?,
    };
    let caps = object_prop(&node, "capabilities");
    if caps.is_empty() {
        return None;
    }
    Some(Value::Object(caps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_patch_merges_and_leaves_untouched_keys_alone() {
        let mut cfg = map(json!({ "mode": "poll", "interval_seconds": 300, "resource": "files" }));
        let changed = apply(&mut cfg, &map(json!({ "cache_content": true }))).unwrap();
        assert_eq!(changed, vec!["cache_content"]);
        assert_eq!(cfg["mode"], json!("poll"));
        // A connector key this endpoint cannot write must still survive the
        // round trip — the patch is a merge, not a replacement.
        assert_eq!(cfg["resource"], json!("files"));
        assert_eq!(cfg["cache_content"], json!(true));
    }

    #[test]
    fn an_explicit_null_clears_a_ttl() {
        let mut cfg = map(json!({ "cache_content": true, "content_ttl_seconds": 3600 }));
        apply(&mut cfg, &map(json!({ "content_ttl_seconds": null }))).unwrap();
        // REMOVED, not stored as JSON null: absent is the state the engine's
        // defaults were written for.
        assert!(!cfg.contains_key("content_ttl_seconds"));
    }

    /// One bad key must not half-apply the good ones, or a refused request
    /// leaves a combination nobody chose and the validator never saw.
    #[test]
    fn a_rejected_key_applies_nothing() {
        let mut cfg = map(json!({ "mode": "poll" }));
        let err = apply(
            &mut cfg,
            &map(json!({ "cache_content": true, "interval_seconds": 1 })),
        )
        .unwrap_err();
        assert!(format!("{err:?}").contains("interval_seconds"));
        assert!(!cfg.contains_key("cache_content"));
    }

    #[test]
    fn engine_owned_properties_are_unreachable() {
        let mut cfg = map(json!({}));
        for key in ["state", "integration_ref", "remote_root"] {
            assert!(apply(&mut cfg, &map(json!({ key: "x" }))).is_err(), "{key}");
        }
        assert!(cfg.is_empty());
    }

    /// A refused-with-a-reason key must say the reason. "Unknown field" would
    /// leave the operator exactly where the production incident left them.
    #[test]
    fn accepts_content_is_refused_with_its_reason() {
        let mut cfg = map(json!({}));
        let err = apply(&mut cfg, &map(json!({ "accepts_content": true }))).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("ADAPTER capability"), "{msg}");
        assert!(msg.contains("resource"), "{msg}");
    }
}
