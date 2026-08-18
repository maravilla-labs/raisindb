// SPDX-License-Identifier: BSL-1.1

//! `GET|POST /api/integrations/{repo}/notifications/{mount_token}` — the generic,
//! provider-agnostic push-notification endpoint for virtual mounts (Experimental).
//!
//! A push notification is treated purely as an **invalidation signal**: the
//! provider payload is ignored, the ping just means "re-run this mount's normal
//! delta sync". Every provider quirk is handled by *shape*, never by provider
//! identity:
//!
//! - **Validation handshake** — if the request carries a challenge parameter
//!   (`validationToken` for Microsoft Graph, `challenge` / `hub.challenge` for
//!   pub/sub-style hubs) it is echoed back verbatim as `text/plain` 200. A bare
//!   `GET` with no challenge (Google channel sync / a health probe) returns 200.
//! - **Token → mount** — the unguessable `{mount_token}` path segment is matched
//!   against the token the engine baked into each mount's stored
//!   `state.push_notification_url` (ground truth: that URL is literally what the
//!   provider was told to call), falling back to a `state.push_token` field or
//!   the mount id. No match → 404.
//! - **Secret verification** — the mount's stored `state.push_secret` is looked
//!   for, matched in constant time, in *any* request carrier: any query value,
//!   any header value, or any string ANYWHERE in the JSON body (nested, depth-
//!   limited — providers put it inside an envelope: Graph uses
//!   `value[].clientState`, Google Pub/Sub `message.attributes`). This is
//!   deliberately carrier-agnostic — a provider may echo the per-subscription
//!   secret back in whatever query param, header, or body field it chooses, and
//!   a user-authored connector gets the exact same treatment as the shipped
//!   ones; no carrier name is special-cased. If a secret is stored and it
//!   appears in no carrier → 401. If no secret is stored, the unguessable token
//!   is the only guard (allowed).
//!
//! This endpoint is provider-facing and therefore **public** (providers cannot
//! send a bearer token); it is guarded solely by the token + secret. No secret
//! or token material is ever logged or returned in a response body.

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use serde_json::Value;

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Query/body parameter names that carry a provider validation challenge.
const CHALLENGE_KEYS: &[&str] = &[
    "validationToken",
    "validation_token",
    "challenge",
    "hub.challenge",
];

/// Provider validation handshake + invalidation-signal handler.
///
/// Returns fast (never blocks on the sync) so latency-sensitive providers get a
/// prompt ack; the actual work is an idempotently-enqueued `VirtualMountSync`.
#[cfg(feature = "storage-rocksdb")]
pub async fn notify(
    State(state): State<AppState>,
    Path((repo, mount_token)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    method: Method,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    // 1. Validation handshake: echo any challenge as text/plain, regardless of
    //    method. Graph sends this at subscription-creation time, before a secret
    //    is in play, and requires a prompt verbatim echo.
    if let Some(challenge) = extract_challenge(&query, &body_json) {
        return Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            challenge,
        )
            .into_response());
    }

    // A bare GET with no challenge is a Google channel `sync` / health probe.
    if method == Method::GET {
        return Ok(StatusCode::OK.into_response());
    }

    // 2. Map the token to a mount by scanning the repo's system-workspace mounts.
    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-notify");
    let mounts = svc
        .list_by_type("raisin:VirtualMount")
        .await
        .map_err(|e| ApiError::internal(format!("failed to list virtual mounts: {e}")))?;

    let Some(mount) = mounts
        .into_iter()
        .find(|n| token_matches(&mount_token, &mount_state(n), &n.id))
    else {
        // An ORPHANED subscription: the mount is gone but the provider was never
        // told, so it keeps delivering forever.
        //
        // In production this was 2,147 of 2,282 notifications — 94% of all
        // webhook traffic — for one deleted mount. Each one wakes the server and
        // scans every `raisin:VirtualMount` in the repo looking for a token that
        // cannot match. A bare 404 gave the provider no reason to stop and left
        // no trace naming what to clean up.
        //
        // `410 Gone` is the standard "this endpoint is permanently dead" signal
        // and is what makes a provider retire a subscription rather than retry
        // it. The subscription id is logged so an operator can delete it at the
        // provider directly — we cannot unsubscribe ourselves, because the
        // credential to do so lived on the mount that no longer exists.
        let subscription_ids = orphan_subscription_ids(&body_json);
        tracing::warn!(
            repo = %repo,
            mount_token_prefix = %mount_token.split('.').next().unwrap_or(""),
            subscription_ids = %subscription_ids,
            "orphaned push subscription: no mount matches this notification token. \
             Delete the subscription at the provider — its mount is gone, so this \
             server cannot unsubscribe it."
        );
        return Err(ApiError::new(
            StatusCode::GONE,
            "MOUNT_NOT_FOUND",
            "No virtual mount matches this notification token",
        ));
    };

    let state_json = mount_state(&mount);

    // 3. Verify the provider secret in constant time (when one is stored),
    //    accepting it from any carrier the provider chose.
    let stored_secret = string_field(&state_json, "push_secret");
    if !secret_ok(stored_secret.as_deref(), &query, &headers, &body_json) {
        // Record the rejection on the mount. A subscription can sit at
        // `push_status: "active"` — the provider accepted our URL once — while
        // every delivery since is turned away, and until this was recorded there
        // was no way to see that from the console or the logs. `warn!` because
        // production runs at that level.
        tracing::warn!(
            mount_id = %mount.id,
            "rejected push notification: stored secret not present in any carrier"
        );
        record_delivery(&state, &tenant.tenant_id, &repo, &mount.id, false).await;
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_NOTIFICATION_SECRET",
            "Notification secret does not match",
        ));
    }

    record_delivery(&state, &tenant.tenant_id, &repo, &mount.id, true).await;

    // 4. Enqueue a delta sync idempotently (mirrors manage.rs::sync_mount) and
    //    ack immediately — never block on the sync itself.
    let branch = super::config_branch(&state, &tenant.tenant_id, &repo).await;
    let registered =
        enqueue_delta_sync(&state, &tenant.tenant_id, &repo, &branch, &mount.id).await?;
    let status = if registered {
        "queued"
    } else {
        "already_running"
    };
    Ok((
        StatusCode::OK,
        axum::Json(serde_json::json!({ "status": status })),
    )
        .into_response())
}

/// Stamp webhook DELIVERY health onto the mount's `state`.
///
/// Distinct from subscription health: `push_status` says whether the provider
/// accepted our URL, this says whether its notifications are actually getting
/// through. Best-effort — a failure here must never turn into a non-200 for the
/// provider, which would make it retry a ping we already handled.
#[cfg(feature = "storage-rocksdb")]
async fn record_delivery(state: &AppState, tenant: &str, repo: &str, mount_id: &str, ok: bool) {
    let Some(storage) = state.rocksdb_storage() else {
        return;
    };
    // Record on the branch the mount was FOUND on, not on a separately-resolved
    // one.
    //
    // The lookup above goes through `config_service`, which is hardcoded to
    // `CONFIG_BRANCH`; this used to call `config_branch()`, which resolves the
    // repo's real `default_branch`. On a repo where those differ the lookup
    // succeeds and the write targets a branch the mount does not exist on, so
    // `record_push_delivery` gives up — and every delivery counter stays at zero
    // while notifications are demonstrably arriving and being acked with 200.
    //
    // Observed in production on `studio`: 200s in the access log, "No delivery
    // yet · ACCEPTED 0" in the console, and (once this release started logging
    // it) `skipping push-delivery counter write: mount node not found on this
    // branch`. Two branch resolutions for one node is the bug; using one is the
    // fix. The wider inconsistency of the integrations surface on a non-`main`
    // repo is noted on `config_branch` and is not addressed here.
    let branch = super::CONFIG_BRANCH;
    if let Err(e) =
        raisin_rocksdb::record_push_delivery(storage, tenant, repo, branch, mount_id, ok).await
    {
        tracing::warn!(mount_id = %mount_id, error = %e, "failed to record push delivery health");
    }
}

#[cfg(not(feature = "storage-rocksdb"))]
async fn record_delivery(_: &AppState, _: &str, _: &str, _: &str, _: bool) {}

#[cfg(feature = "storage-rocksdb")]
/// Enqueue a `VirtualMountSync { mode: "delta" }` for one mount, deduped on the in-flight
/// key `vmount-sync:{mount_id}`.
///
/// `branch` is a PARAMETER rather than something this function resolves. It used to call
/// `config_branch()` internally, which meant one request resolved the branch twice — and
/// the two resolutions could disagree, which is the bug documented at the call site
/// above. The caller already knows the branch it read the mount from; passing it is the
/// only way to guarantee the job runs against the same one.
///
/// Returns whether the job was newly registered. The idempotency key
/// `vmount-sync:{mount_id}` is shared with the scheduler's own enqueue, so a burst of
/// provider events collapses to one run per mount for free.
pub(crate) async fn enqueue_delta_sync(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
) -> Result<bool, ApiError> {
    enqueue_delta_sync_with_events(state, tenant_id, repo, branch, mount_id, None, None).await
}

/// As above, but carrying events the control plane already received.
///
/// # The idempotency key changes when a payload rides along
///
/// The plain key collapses a burst to one run per mount, which is right when the run
/// re-reads everything: whatever the collapsed deliveries carried is picked up anyway.
/// It is WRONG once the job carries the payload, because collapsing then discards the
/// second event's body — it would be dropped until the safety-net poll happened to
/// re-read that window. So a keyed push gets its own slot per event.
///
/// A burst is affordable here precisely because these runs are cheap: applying a pushed
/// event costs no provider request at all.
pub(crate) async fn enqueue_delta_sync_with_events(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
    events: Option<&serde_json::Value>,
    dedupe_suffix: Option<&str>,
) -> Result<bool, ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB backend required for mount sync"))?;

    let job_registry = rocksdb.job_registry();
    let job_data_store = rocksdb.job_data_store();

    let job_type = raisin_storage::jobs::JobType::VirtualMountSync {
        mount_id: mount_id.to_string(),
        mode: "delta".to_string(),
        trigger: "push".to_string(),
    };
    let job_id = raisin_storage::jobs::JobId::new();
    let mut metadata = std::collections::HashMap::new();
    if let Some(events) = events {
        metadata.insert("pushed_events".to_string(), events.clone());
    }
    let context = raisin_storage::jobs::JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
        workspace_id: "raisin:system".to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata,
    };
    job_data_store
        .put(&job_id, &context)
        .map_err(|e| ApiError::internal(format!("failed to store sync job context: {e}")))?;

    let registered = job_registry
        .register_job_with_id_idempotent(
            job_id.clone(),
            job_type,
            tenant_id.to_string(),
            match dedupe_suffix {
                Some(suffix) => format!("vmount-sync:{mount_id}:{suffix}"),
                None => format!("vmount-sync:{mount_id}"),
            },
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("failed to enqueue sync job: {e}")))?;

    Ok(registered)
}

/// Non-RocksDB stub. The route is only registered under `storage-rocksdb`, so
/// this is never reached; it exists so the module compiles for all feature sets.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn notify(
    State(_state): State<AppState>,
    Path((_repo, _mount_token)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    Err(ApiError::internal(
        "Push notifications require the RocksDB backend",
    ))
}

// --- Pure decision helpers (unit-tested below; no storage or feature deps) ---

/// The mount node's engine-managed `state` object as raw JSON (`Null` if unset).
fn mount_state(node: &raisin_models::nodes::Node) -> Value {
    node.properties
        .get("state")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
}

/// Read a non-empty string field from a JSON object.
fn string_field(obj: &Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extract a validation challenge from the query string or JSON body, checking
/// the known generic challenge parameter names in order. Data-driven, never
/// keyed on provider identity.
fn extract_challenge(query: &HashMap<String, String>, body: &Value) -> Option<String> {
    for key in CHALLENGE_KEYS {
        if let Some(v) = query.get(*key).filter(|s| !s.is_empty()) {
            return Some(v.clone());
        }
    }
    for key in CHALLENGE_KEYS {
        if let Some(v) = string_field(body, key) {
            return Some(v);
        }
    }
    None
}

/// Whether the request may proceed: allowed when no secret is stored (the
/// unguessable token is the only guard) or when the stored secret appears —
/// matched in constant time — in *any* request carrier: a query value, a header
/// value, or a top-level string field of the JSON body.
///
/// Carrier-agnostic by design: the mount is already pinned by the unguessable
/// URL token, so the endpoint need not know *which* field a given provider
/// echoes the secret in — only that the secret is present. This is exactly what
/// lets a user-authored connector implement push the same way the shipped ones
/// do, with no provider name baked into the engine or endpoint.
fn secret_ok(
    stored: Option<&str>,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
    body: &Value,
) -> bool {
    let stored = match stored {
        None => return true,
        Some(s) => s.as_bytes(),
    };
    for v in query.values() {
        if ct_eq(stored, v.as_bytes()) {
            return true;
        }
    }
    for v in headers.values() {
        if let Ok(s) = v.to_str() {
            if ct_eq(stored, s.as_bytes()) {
                return true;
            }
        }
    }
    // Walk the body RECURSIVELY, not just its top level.
    //
    // This scanned only top-level strings, which meant it never found the
    // secret any real provider sends. Microsoft Graph POSTs
    //
    //     {"value":[{"subscriptionId":"…","clientState":"<secret>", …}]}
    //
    // whose single top-level key holds an ARRAY, so `as_str()` was `None`, the
    // loop matched nothing, and EVERY genuine notification got 401. The
    // subscribe-time validation handshake still returned 200 because it is
    // answered before this check runs, so the mount looked healthy while no
    // ping ever reached the sync engine. Google's `message.attributes` nests
    // one level too. Depth-limited so a hostile payload cannot make us walk a
    // pathological tree.
    if body_contains_secret(stored, body, 0) {
        return true;
    }
    false
}

/// Maximum nesting the body scan descends. Comfortably deeper than any real
/// provider envelope (Graph is 2, Google Pub/Sub 2) and shallow enough that a
/// hostile payload cannot turn this into a lot of work.
const MAX_BODY_SCAN_DEPTH: usize = 6;

/// Whether `stored` appears as any string anywhere in `body`, constant-time per
/// comparison, bounded by [`MAX_BODY_SCAN_DEPTH`].
fn body_contains_secret(stored: &[u8], body: &Value, depth: usize) -> bool {
    if depth > MAX_BODY_SCAN_DEPTH {
        return false;
    }
    match body {
        Value::String(s) => ct_eq(stored, s.as_bytes()),
        Value::Array(items) => items
            .iter()
            .any(|v| body_contains_secret(stored, v, depth + 1)),
        Value::Object(obj) => obj
            .values()
            .any(|v| body_contains_secret(stored, v, depth + 1)),
        _ => false,
    }
}

/// Constant-time byte-slice equality (no `subtle` dependency in this crate).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether `mount_token` identifies this mount. The authoritative match is the
/// last path segment of the stored `push_notification_url` (that URL is exactly
/// what the provider calls back), with a `push_token` field and the mount id as
/// fallbacks. Comparison is constant-time on the token.
fn token_matches(mount_token: &str, state: &Value, mount_id: &str) -> bool {
    if let Some(url) = string_field(state, "push_notification_url") {
        let seg = url.rsplit('/').next().unwrap_or_default();
        if !seg.is_empty() && ct_eq(seg.as_bytes(), mount_token.as_bytes()) {
            return true;
        }
    }
    if let Some(tok) = string_field(state, "push_token") {
        if ct_eq(tok.as_bytes(), mount_token.as_bytes()) {
            return true;
        }
    }
    ct_eq(mount_id.as_bytes(), mount_token.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn qs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn challenge_echoed_from_query_validation_token() {
        let q = qs(&[("validationToken", "abc123")]);
        assert_eq!(
            extract_challenge(&q, &Value::Null).as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn challenge_echoed_from_body_and_hub_challenge() {
        let empty = HashMap::new();
        assert_eq!(
            extract_challenge(&empty, &json!({ "challenge": "xyz" })).as_deref(),
            Some("xyz")
        );
        let q = qs(&[("hub.challenge", "9988")]);
        assert_eq!(extract_challenge(&q, &Value::Null).as_deref(), Some("9988"));
    }

    #[test]
    fn no_challenge_returns_none() {
        assert!(extract_challenge(&HashMap::new(), &json!({ "resource": "x" })).is_none());
    }

    #[test]
    fn secret_mismatch_is_rejected() {
        let empty_q = HashMap::new();
        let empty_h = HeaderMap::new();
        // Stored secret + wrong provided secret => not ok (handler maps to 401).
        let wrong = json!({ "clientState": "wrong" });
        assert!(!secret_ok(Some("s3cr3t"), &empty_q, &empty_h, &wrong));
        // Stored secret + nothing provided => not ok.
        assert!(!secret_ok(Some("s3cr3t"), &empty_q, &empty_h, &Value::Null));
        // Correct secret in the body is accepted.
        let right = json!({ "clientState": "s3cr3t" });
        assert!(secret_ok(Some("s3cr3t"), &empty_q, &empty_h, &right));
        // No stored secret => the unguessable token is the only guard.
        assert!(secret_ok(None, &empty_q, &empty_h, &Value::Null));
    }

    #[test]
    fn secret_accepted_from_any_carrier() {
        let empty_q = HashMap::new();
        let empty_h = HeaderMap::new();

        // Body field (any key — the field name is not special-cased).
        let body = json!({ "anyField": "the-secret" });
        assert!(secret_ok(Some("the-secret"), &empty_q, &empty_h, &body));

        // Arbitrary header value.
        let mut headers = HeaderMap::new();
        headers.insert("x-any-header", "the-secret".parse().unwrap());
        assert!(secret_ok(
            Some("the-secret"),
            &empty_q,
            &headers,
            &Value::Null
        ));

        // Arbitrary query param.
        let q = qs(&[("anything", "the-secret")]);
        assert!(secret_ok(Some("the-secret"), &q, &empty_h, &Value::Null));
    }

    /// The payload REAL providers send, which the old top-level-only scan could
    /// never match.
    ///
    /// The pre-existing tests all put `clientState` at the top level — a shape
    /// Graph never sends — so they passed while production 401'd every single
    /// notification. Assert the wire format, not a convenient stand-in.
    #[test]
    fn secret_found_inside_provider_envelopes() {
        let empty_q = HashMap::new();
        let empty_h = HeaderMap::new();

        // Microsoft Graph: clientState lives in each element of `value[]`.
        let graph = json!({
            "value": [{
                "subscriptionId": "9e93b593-1dca-4092-8ece-f6941dc94759",
                "clientState": "the-secret",
                "changeType": "created",
                "resource": "Users/x/Messages/y"
            }]
        });
        assert!(secret_ok(Some("the-secret"), &empty_q, &empty_h, &graph));

        // Google Pub/Sub: nested under message.attributes.
        let pubsub = json!({
            "message": { "attributes": { "token": "the-secret" }, "data": "eyJ9" },
            "subscription": "projects/p/subscriptions/s"
        });
        assert!(secret_ok(Some("the-secret"), &empty_q, &empty_h, &pubsub));

        // A nested envelope carrying the WRONG secret is still rejected —
        // recursion must not turn into "accept anything".
        let wrong = json!({ "value": [{ "clientState": "not-it" }] });
        assert!(!secret_ok(Some("the-secret"), &empty_q, &empty_h, &wrong));
    }

    /// Recursion is depth-limited, so a deeply nested payload cannot be used to
    /// make the endpoint do unbounded work.
    #[test]
    fn body_scan_is_depth_limited() {
        let mut deep = json!("the-secret");
        for _ in 0..(MAX_BODY_SCAN_DEPTH + 3) {
            deep = json!({ "n": deep });
        }
        let empty_q = HashMap::new();
        let empty_h = HeaderMap::new();
        assert!(!secret_ok(Some("the-secret"), &empty_q, &empty_h, &deep));

        // Within the limit it is still found.
        let mut shallow = json!("the-secret");
        for _ in 0..3 {
            shallow = json!({ "n": shallow });
        }
        assert!(secret_ok(Some("the-secret"), &empty_q, &empty_h, &shallow));
    }

    #[test]
    fn token_lookup_matches_url_segment_and_misses_otherwise() {
        let state = json!({
            "push_notification_url": "https://h/api/integrations/r/notifications/tok-abc",
            "push_secret": "s",
        });
        assert!(token_matches("tok-abc", &state, "mount-1"));
        // A miss: the token->mount lookup returns false (handler maps to 404).
        assert!(!token_matches("tok-zzz", &state, "mount-1"));
    }

    #[test]
    fn token_lookup_falls_back_to_push_token_and_mount_id() {
        let state = json!({ "push_token": "pt-1" });
        assert!(token_matches("pt-1", &state, "mount-9"));
        assert!(token_matches("mount-9", &Value::Null, "mount-9"));
        assert!(!token_matches("nope", &Value::Null, "mount-9"));
    }
}

/// Subscription ids named in a provider notification body, for the orphan log.
///
/// Best-effort and shape-tolerant: Graph sends `{"value":[{"subscriptionId":…}]}`,
/// others differ, and an orphan may arrive with no parseable body at all. The
/// point is to give an operator something to search for at the provider, so an
/// empty answer is fine — never an error.
fn orphan_subscription_ids(body: &serde_json::Value) -> String {
    let mut ids: Vec<String> = Vec::new();
    if let Some(items) = body.get("value").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(id) = item.get("subscriptionId").and_then(|v| v.as_str()) {
                if !ids.iter().any(|existing| existing == id) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    if let Some(id) = body.get("subscriptionId").and_then(|v| v.as_str()) {
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        "none in body".to_string()
    } else {
        ids.join(",")
    }
}
