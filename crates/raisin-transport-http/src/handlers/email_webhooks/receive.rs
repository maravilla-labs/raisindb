// SPDX-License-Identifier: BSL-1.1

//! `POST /api/email/{repo}/webhooks/{name}/{token}` — provider delivery events.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use raisin_functions::runtime::email::feedback::{self, FeedbackEvent};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::secret_store::SecretScope;
use serde_json::{json, Value};

use super::{ct_eq, read_secret, secret_name, sender_entry, store, system_service};
use crate::error::ApiError;
use crate::handlers::integrations::config_branch;
use crate::middleware::TenantInfo;
use crate::state::AppState;

fn header<'a>(h: &'a HeaderMap, k: &str) -> &'a str {
    h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("")
}

fn prop(v: Value) -> Option<PropertyValue> {
    serde_json::from_value(v).ok()
}

pub async fn receive(
    State(state): State<AppState>,
    Path((repo, name, token)): Path<(String, String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let branch = config_branch(&state, &tenant.tenant_id, &repo).await;
    let (kind, _entry) = sender_entry(&state, &tenant.tenant_id, &repo, &branch, &name).await?;

    // 1. The unguessable path token. Same answer whether the sender has no token
    //    or the wrong one, so the route does not reveal which senders exist.
    let secrets = store(&state)?;
    let scope = SecretScope::new(&tenant.tenant_id, &repo, &branch);
    let stored = read_secret(&secrets, &scope, &secret_name(&name, "token"))?;
    if !stored.is_some_and(|t| ct_eq(t.as_bytes(), token.as_bytes())) {
        return Err(ApiError::unauthorized("invalid webhook token"));
    }

    let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let events: Vec<FeedbackEvent> = match kind.as_str() {
        "resend" => {
            // Fail CLOSED: a Resend sender with no signing secret stored is
            // refused rather than trusted on the token alone, because Resend
            // can sign and a missing secret is a setup gap worth surfacing.
            let secret = read_secret(&secrets, &scope, &secret_name(&name, "signing_secret"))?
                .ok_or_else(|| {
                    ApiError::unauthorized(
                        "no signing secret is stored for this Resend sender; run webhook setup",
                    )
                })?;
            feedback::verify_svix(
                &secret,
                header(&headers, "svix-id"),
                header(&headers, "svix-timestamp"),
                &body,
                header(&headers, "svix-signature"),
                chrono::Utc::now().timestamp(),
            )
            .map_err(|why| ApiError::unauthorized(format!("signature check failed: {why}")))?;
            feedback::parse_resend(&parsed, header(&headers, "svix-id"))
        }
        "brevo" => feedback::parse_brevo(&parsed),
        _ => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "NO_FEEDBACK",
                "this sender type does not report delivery events",
            ))
        }
    };

    let svc = system_service(&state, &tenant.tenant_id, &repo, &branch);
    ensure_folders(&svc).await;
    let mut stored_events = 0usize;
    for ev in &events {
        if record(&svc, &kind, ev).await? {
            stored_events += 1;
        }
    }
    Ok((
        StatusCode::OK,
        Json(json!({ "received": events.len(), "stored": stored_events })),
    ))
}

/// Existing tenants predate `/email/*`; create what is missing, once.
async fn ensure_folders(svc: &raisin_core::NodeService<crate::state::Store>) {
    for (parent, name) in [("", "email"), ("/email", "messages"), ("/email", "events")] {
        let path = format!("{parent}/{name}");
        if matches!(svc.get_by_path(&path).await, Ok(Some(_))) {
            continue;
        }
        let node = Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name: name.to_string(),
            path,
            parent: (!parent.is_empty()).then(|| parent.trim_start_matches('/').to_string()),
            ..Default::default()
        };
        let _ = svc.create(node).await;
    }
}

/// Store one event and advance its message. `Ok(false)` = already seen.
async fn record(
    svc: &raisin_core::NodeService<crate::state::Store>,
    provider: &str,
    ev: &FeedbackEvent,
) -> Result<bool, ApiError> {
    let event_path = format!("{}/{}", feedback::EVENTS_PATH, ev.dedupe_key);
    if svc.get_by_path(&event_path).await?.is_some() {
        return Ok(false);
    }

    let msg_path = format!(
        "{}/{}",
        feedback::MESSAGES_PATH,
        feedback::message_node_name(&ev.message_id)
    );
    let message = svc.get_by_path(&msg_path).await?;
    let now = chrono::Utc::now().to_rfc3339();

    let mut event = Node {
        id: nanoid::nanoid!(),
        node_type: "raisin:EmailEvent".to_string(),
        name: ev.dedupe_key.clone(),
        path: event_path,
        parent: Some("events".to_string()),
        ..Default::default()
    };
    let set = |n: &mut Node, k: &str, v: Option<&str>| {
        if let Some(pv) = v.and_then(|v| prop(json!(v))) {
            n.properties.insert(k.to_string(), pv);
        }
    };
    set(&mut event, "provider", Some(provider));
    set(&mut event, "message_id", Some(&ev.message_id));
    set(&mut event, "event", Some(ev.kind));
    set(&mut event, "email", ev.email.as_deref());
    set(&mut event, "occurred_at", ev.occurred_at.as_deref());
    set(&mut event, "received_at", Some(&now));
    set(&mut event, "detail", ev.detail.as_deref());

    if let Some(m) = &message {
        for k in ["correlation_kind", "correlation_id"] {
            if let Some(PropertyValue::String(v)) = m.properties.get(k) {
                set(&mut event, k, Some(v));
            }
        }
    }
    svc.create(event).await?;

    // Advance the message AFTER the event is durable: a crash in between leaves
    // an event with a stale message status (repairable), never the reverse.
    if let Some(mut m) = message {
        let current = match m.properties.get("status") {
            Some(PropertyValue::String(s)) => feedback::status_rank(s),
            _ => None,
        };
        let mut changed = false;
        if let Some(new_rank) = feedback::status_rank(ev.kind) {
            if current.map_or(true, |c| new_rank > c) {
                set(&mut m, "status", Some(ev.kind));
                if let Some(d) = ev.detail.as_deref().filter(|_| new_rank >= 2) {
                    set(&mut m, "last_error", Some(d));
                }
                changed = true;
            }
        }
        if changed {
            set(&mut m, "last_event_at", Some(&now));
            svc.update_node(m).await?;
        }
    }
    Ok(true)
}
