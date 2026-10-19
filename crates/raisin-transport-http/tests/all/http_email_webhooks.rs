//! `/api/email/{repo}/webhooks/{name}/{token}` — Brevo / Resend delivery feedback.
//!
//! Drives the whole loop a customer relies on: admin setup mints the URL, the
//! provider posts to it WITHOUT credentials, the event is stored once, and the
//! accepted message's status advances — and never goes backwards.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{body::Body, http::Request};
use serde_json::{json, Value};

use raisin_crypto::{Keyring, SecretBox};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::secret_store::SecretStore;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::support::{admin, send, Fixture, TENANT};

const REPO: &str = "mailrepo";
const WS: &str = "raisin:system";

fn node(path: &str, node_type: &str, props: Value) -> Node {
    let props: HashMap<String, PropertyValue> = serde_json::from_value(props).unwrap();
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = path[..path.rfind('/').unwrap()]
        .rsplit('/')
        .next()
        .unwrap_or("");
    Node {
        id: nanoid::nanoid!(),
        name,
        path: path.to_string(),
        node_type: node_type.to_string(),
        parent: Some(if parent.is_empty() {
            "/".into()
        } else {
            parent.into()
        }),
        properties: props,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WS.to_string()),
        ..Default::default()
    }
}

async fn fixture() -> Fixture {
    let f = Fixture::new(
        "email-webhooks",
        REPO,
        "main",
        &[WS],
        &[
            "raisin:Folder",
            "raisin:EmailConfig",
            "raisin:EmailMessage",
            "raisin:EmailEvent",
        ],
    )
    .await;
    let keys = Arc::new(Keyring::new(vec![(1, [9u8; 32])], 1).unwrap());
    let secrets = Arc::new(SecretStore::new(
        f.storage.db().clone(),
        Arc::new(SecretBox::with_keyring(keys)),
        "test-node",
    ));
    assert!(f.storage.set_secret_store(secrets));

    let nodes = f.storage.nodes_impl();
    for (path, ty, props) in [
        ("/config", "raisin:Folder", json!({})),
        (
            "/config/email",
            "raisin:EmailConfig",
            json!({"enabled": true, "providers": [
                {"name": "news", "provider": "brevo", "credential_ref": "secret://email/k"}
            ]}),
        ),
        ("/email", "raisin:Folder", json!({})),
        ("/email/events", "raisin:Folder", json!({})),
        ("/email/messages", "raisin:Folder", json!({})),
        (
            "/email/messages/_m1_brevo_",
            "raisin:EmailMessage",
            json!({"provider": "brevo", "sender": "news", "message_id": "<m1@brevo>", "status": "accepted"}),
        ),
    ] {
        nodes
            .add(TENANT, REPO, "main", WS, node(path, ty, props))
            .await
            .unwrap();
    }
    f
}

fn post(uri: &str, body: Value, authed: bool) -> Request<Body> {
    let b = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    let b = if authed { admin(b) } else { b };
    b.body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn brevo(event: &str, ts: u64) -> Value {
    json!({"event": event, "email": "a@b.c", "message-id": "<m1@brevo>", "ts_event": ts})
}

async fn status_of(f: &Fixture) -> String {
    let scope = StorageScope::new(TENANT, REPO, "main", WS);
    let n = f
        .storage
        .nodes()
        .get_by_path(scope, "/email/messages/_m1_brevo_", None)
        .await
        .unwrap()
        .unwrap();
    match n.properties.get("status") {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("no status: {other:?}"),
    }
}

#[tokio::test]
async fn brevo_events_advance_the_message_monotonically_and_dedupe() {
    let f = fixture().await;

    let (code, body) = send(
        &f.raw,
        post(
            &format!("/api/email/{REPO}/webhook-setup/news"),
            json!({"public_base": "https://db.example.com"}),
            true,
        ),
    )
    .await;
    assert_eq!(code, 200, "{body}");
    let url = serde_json::from_str::<Value>(&body).unwrap()["url"]
        .as_str()
        .unwrap()
        .to_string();
    let path = url
        .strip_prefix("https://db.example.com")
        .unwrap()
        .to_string();

    // The provider presents no credentials: the token in the path is the guard.
    let (code, body) = send(&f.raw, post(&path, brevo("delivered", 1), false)).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(status_of(&f).await, "delivered");

    // Same event redelivered: recognised, not stored again.
    let (_, body) = send(&f.raw, post(&path, brevo("delivered", 1), false)).await;
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["stored"], 0);

    // A late, lower-ranked event is recorded but cannot move the status back.
    send(&f.raw, post(&path, brevo("deferred", 2), false)).await;
    assert_eq!(status_of(&f).await, "delivered");

    // An asynchronous hard bounce outranks a delivery.
    send(&f.raw, post(&path, brevo("hard_bounce", 3), false)).await;
    assert_eq!(status_of(&f).await, "hard_bounce");
}

#[tokio::test]
async fn wrong_token_and_non_admin_setup_are_refused() {
    let f = fixture().await;
    send(
        &f.raw,
        post(
            &format!("/api/email/{REPO}/webhook-setup/news"),
            json!({"public_base": "https://x"}),
            true,
        ),
    )
    .await;

    let (code, _) = send(
        &f.raw,
        post(
            &format!("/api/email/{REPO}/webhooks/news/not-the-token"),
            brevo("delivered", 1),
            false,
        ),
    )
    .await;
    assert_eq!(code, 401);
    assert_eq!(status_of(&f).await, "accepted");

    let (code, _) = send(
        &f.raw,
        post(
            &format!("/api/email/{REPO}/webhook-setup/news"),
            json!({"public_base": "https://x"}),
            false,
        ),
    )
    .await;
    assert!(
        code == 401 || code == 403,
        "setup must be admin-only, got {code}"
    );
}

#[tokio::test]
async fn unknown_sender_looks_like_a_bad_token_and_senders_cannot_cross() {
    let f = fixture().await;
    let (code, _) = send(
        &f.raw,
        post(
            &format!("/api/email/{REPO}/webhooks/nope/whatever"),
            brevo("delivered", 1),
            false,
        ),
    )
    .await;
    assert_eq!(
        code, 401,
        "an unknown sender must not be distinguishable from a bad token"
    );

    // The message was sent through `news`; a different sender with its own valid
    // token must not be able to move it.
    let scope = StorageScope::new(TENANT, REPO, "main", WS);
    let mut cfg = f
        .storage
        .nodes()
        .get_by_path(scope.clone(), "/config/email", None)
        .await
        .unwrap()
        .unwrap();
    cfg.properties = serde_json::from_value(json!({"enabled": true, "providers": [
        {"name": "news", "provider": "brevo", "credential_ref": "secret://email/k"},
        {"name": "other", "provider": "brevo", "credential_ref": "secret://email/k2"}
    ]}))
    .unwrap();
    f.storage
        .nodes()
        .update(scope.clone(), cfg, Default::default())
        .await
        .unwrap();
    // The fixture's message predates `sender`; stamp it as the news sender's.
    let mut m = f
        .storage
        .nodes()
        .get_by_path(scope.clone(), "/email/messages/_m1_brevo_", None)
        .await
        .unwrap()
        .unwrap();
    m.properties
        .insert("sender".into(), PropertyValue::String("news".into()));
    f.storage
        .nodes()
        .update(scope, m, Default::default())
        .await
        .unwrap();

    let (_, body) = send(
        &f.raw,
        post(
            &format!("/api/email/{REPO}/webhook-setup/other"),
            json!({"public_base": "https://x"}),
            true,
        ),
    )
    .await;
    let url = serde_json::from_str::<Value>(&body).unwrap()["url"]
        .as_str()
        .unwrap()
        .to_string();
    let path = url.strip_prefix("https://x").unwrap().to_string();
    let (code, body) = send(&f.raw, post(&path, brevo("hard_bounce", 9), false)).await;
    assert_eq!(code, 200, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["stored"], 0);
    assert_eq!(status_of(&f).await, "accepted");
}
