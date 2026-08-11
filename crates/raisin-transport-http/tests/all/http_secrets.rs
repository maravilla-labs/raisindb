//! `/api/secrets/{repo}/{branch}` — the admin-gated secret-store surface.
//!
//! The load-bearing assertion in this file is [`no_endpoint_ever_emits_the_plaintext`]:
//! the whole design is that values go in and never come out, and the only way
//! that stays true is a test that reads every response body and greps it.

#![cfg(all(feature = "storage-rocksdb", not(feature = "s3")))]

use std::sync::{Arc, Once};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use raisin_crypto::{Keyring, SecretBox};
use raisin_rocksdb::secret_store::{SecretOwner, SecretScope, SecretStore};
use raisin_rocksdb::RocksDBStorage;

/// The bearer the tests present. `optional_auth_middleware` matches it against
/// `RAISIN_SUPERADMIN_TOKEN` and installs `AuthContext::system()`, which is
/// what `require_admin` accepts.
const ADMIN_TOKEN: &str = "secrets-test-superadmin-token";

/// Env vars are process-global. Both of these are only ever SET, never cleared,
/// so tests running in parallel in this binary cannot race on them.
///
/// `RAISIN_CRYPTO_EMIT_V2` is not optional: the node-secret crypto family is
/// `V1Policy::Reject`, so without it every write fails with
/// `V2EmissionRequired` rather than storing bytes it could never read back.
static ENV: Once = Once::new();

fn init_env() {
    ENV.call_once(|| {
        std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1");
        std::env::set_var("RAISIN_SUPERADMIN_TOKEN", ADMIN_TOKEN);
    });
}

/// A router over a fresh RocksDB, with a secret store installed from an
/// explicit keyring rather than the environment — so these tests do not depend
/// on `RAISIN_MASTER_KEY(S)` and cannot disturb any other test that does.
fn app(name: &str) -> axum::Router {
    init_env();
    let path = format!("/tmp/raisin-secrets-test-{name}");
    let _ = std::fs::remove_dir_all(&path);
    let store = Arc::new(RocksDBStorage::new(&path).unwrap());

    let keys = Arc::new(Keyring::new(vec![(1, [9u8; 32])], 1).unwrap());
    let secrets = Arc::new(SecretStore::new(
        store.db().clone(),
        Arc::new(SecretBox::with_keyring(keys)),
        "test-node",
    ));
    assert!(store.set_secret_store(secrets));

    raisin_transport_http::router(store)
}

fn admin(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("authorization", format!("Bearer {ADMIN_TOKEN}"))
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn json_body(value: Value) -> Body {
    Body::from(serde_json::to_vec(&value).unwrap())
}

async fn put_secret(app: &axum::Router, name: &str, value: &str) -> (StatusCode, String) {
    let req = admin(
        Request::builder()
            .method("PUT")
            .uri(format!("/api/secrets/test/main/{name}"))
            .header("content-type", "application/json"),
    )
    .body(json_body(serde_json::json!({ "value": value })))
    .unwrap();
    send(app, req).await
}

// ---- round trip ---------------------------------------------------------

#[tokio::test]
async fn put_then_list_then_get_metadata() {
    let app = app("round-trip");

    let (status, body) = put_secret(&app, "stripe_key", "sk_live_abc").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let put: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(put["name"], "stripe_key");
    assert_eq!(put["version"], 1);
    assert_eq!(put["reference"], "secret://stripe_key");

    // A second write appends a version rather than overwriting.
    let (_, body) = put_secret(&app, "stripe_key", "sk_live_def").await;
    let put2: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(put2["version"], 2);

    let req = admin(Request::builder().uri("/api/secrets/test/main"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: Value = serde_json::from_str(&body).unwrap();
    let secrets = list["secrets"].as_array().unwrap();
    assert_eq!(
        secrets.len(),
        1,
        "list shows one entry per NAME, not per version"
    );
    assert_eq!(secrets[0]["name"], "stripe_key");
    assert_eq!(
        secrets[0]["version"], 2,
        "the newest version is what is listed"
    );
    assert!(
        secrets[0].get("ciphertext").is_none(),
        "SecretMetadata must have no field that can hold ciphertext"
    );

    let req = admin(Request::builder().uri("/api/secrets/test/main/stripe_key"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let meta: Value = serde_json::from_str(&body).unwrap();
    // The newest version is FLATTENED to the top level, so the body is itself a
    // `SecretMetadata` — a client that only wants "what is this secret" never
    // has to reach into `versions`.
    assert_eq!(meta["name"], "stripe_key");
    assert_eq!(meta["version"], 2);
    assert_eq!(meta["deleted"], false);
    assert!(meta["created_at"].is_string());
    assert!(meta.get("key_id").is_some());
    let versions = meta["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2, "every version is listed, newest first");
    assert_eq!(versions[0]["version"], 2);
    assert_eq!(versions[1]["version"], 1);
}

// ---- THE invariant ------------------------------------------------------

/// **No endpoint on this surface ever emits a plaintext value.**
///
/// Not "no endpoint is documented to"; no response body contains the bytes.
/// This is checked over every route, including the error shapes, because a
/// leak is most likely to arrive through a helpful error message quoting the
/// value it could not handle.
#[tokio::test]
async fn no_endpoint_ever_emits_the_plaintext() {
    let app = app("no-leak");
    const PLAINTEXT: &str = "pl4inT3xt-do-not-emit-me";

    let mut bodies: Vec<(String, String)> = Vec::new();

    let (_, body) = put_secret(&app, "leaky", PLAINTEXT).await;
    bodies.push(("PUT".into(), body));

    let req = admin(
        Request::builder()
            .method("POST")
            .uri("/api/secrets/test/main/rotate/leaky")
            .header("content-type", "application/json"),
    )
    .body(json_body(serde_json::json!({ "value": PLAINTEXT })))
    .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    bodies.push(("POST rotate".into(), body));

    let req = admin(Request::builder().uri("/api/secrets/test/main"))
        .body(Body::empty())
        .unwrap();
    bodies.push(("GET list".into(), send(&app, req).await.1));

    let req = admin(Request::builder().uri("/api/secrets/test/main/leaky"))
        .body(Body::empty())
        .unwrap();
    bodies.push(("GET metadata".into(), send(&app, req).await.1));

    let req = admin(
        Request::builder()
            .method("DELETE")
            .uri("/api/secrets/test/main/leaky"),
    )
    .body(Body::empty())
    .unwrap();
    bodies.push(("DELETE".into(), send(&app, req).await.1));

    // And after the tombstone, the metadata read again — the error/`deleted`
    // shape is a response too.
    let req = admin(Request::builder().uri("/api/secrets/test/main/leaky"))
        .body(Body::empty())
        .unwrap();
    bodies.push(("GET metadata after delete".into(), send(&app, req).await.1));

    for (route, body) in &bodies {
        assert!(
            !body.contains(PLAINTEXT),
            "{route} response leaked the plaintext: {body}"
        );
    }

    // Sanity: the assertion above would pass on empty bodies too, so prove the
    // responses actually carried content.
    assert!(bodies.iter().all(|(_, b)| b.contains("leaky")));
}

// ---- a name with slashes ------------------------------------------------

/// The auto-vault convention is `node/{node_id}/{field.path}`, so the route
/// parameter must be a wildcard capture. A single-segment `{name}` 404s here.
#[tokio::test]
async fn a_name_containing_slashes_round_trips() {
    let app = app("slashy-name");
    let name = "node/01H8XY/venue.token";

    let (status, body) = put_secret(&app, name, "tok").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let put: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(put["name"], name);
    assert_eq!(put["reference"], format!("secret://{name}"));

    let req = admin(Request::builder().uri(format!("/api/secrets/test/main/{name}")))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let meta: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(meta["name"], name);
    assert_eq!(meta["versions"].as_array().unwrap().len(), 1);

    // And rotation, whose route puts the wildcard after a literal segment.
    let req = admin(
        Request::builder()
            .method("POST")
            .uri(format!("/api/secrets/test/main/rotate/{name}"))
            .header("content-type", "application/json"),
    )
    .body(json_body(serde_json::json!({ "value": "tok2" })))
    .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["version"],
        2,
        "rotate appends"
    );
}

/// **Literal `/` is the wire form; `%2F` is NOT an escape here.**
///
/// The route is a wildcard precisely because names contain `/`, so a client
/// must send the separators literally. A percent-encoded `%2F` does not round
/// trip: axum decodes it while capturing, so it arrives as a literal `/` and
/// addresses the *same* secret — it is a redundant spelling, never a distinct
/// name. This test pins that, because the alternative reading ("`%2F` escapes a
/// slash so the name has one segment") would have a client and the server
/// disagreeing about which secret is which, silently.
#[tokio::test]
async fn percent_encoded_slashes_decode_to_the_same_name() {
    let app = app("percent-encoding");
    let name = "node/01H8XY/api_key";

    let (status, body) = put_secret(&app, name, "tok").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Address the SAME secret with `%2F` in place of every `/`.
    let encoded = name.replace('/', "%2F");
    let req = admin(Request::builder().uri(format!("/api/secrets/test/main/{encoded}")))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "%2F must reach the same secret, not 404: {body}"
    );
    let meta: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        meta["name"], name,
        "the captured name is decoded, so %2F and / name one secret"
    );
    assert_eq!(
        meta["versions"].as_array().unwrap().len(),
        1,
        "one secret, not two — an encoded name must not mint a second entry"
    );

    // And the store agrees: exactly one name exists in the branch.
    let req = admin(Request::builder().uri("/api/secrets/test/main"))
        .body(Body::empty())
        .unwrap();
    let (_, body) = send(&app, req).await;
    let list: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(list["secrets"].as_array().unwrap().len(), 1);
}

// ---- delete -------------------------------------------------------------

/// A delete is a tombstone, not an erasure: the name stays visible, flagged,
/// so an operator can tell "retired" from "never existed", and prior versions
/// stay readable through a pinned reference.
#[tokio::test]
async fn delete_tombstones_and_metadata_shows_it() {
    let app = app("delete-tombstone");
    put_secret(&app, "doomed", "v1").await;

    let req = admin(
        Request::builder()
            .method("DELETE")
            .uri("/api/secrets/test/main/doomed"),
    )
    .body(Body::empty())
    .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let deleted: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(deleted["deleted"], true);
    assert_eq!(deleted["version"], 2, "the tombstone is itself a version");

    let req = admin(Request::builder().uri("/api/secrets/test/main/doomed"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let meta: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(meta["deleted"], true, "newest version is the tombstone");
    let versions = meta["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["deleted"], true);
    assert_eq!(versions[1]["deleted"], false, "v1 survives the tombstone");

    // The listing keeps it, flagged — absent from the list would read as
    // "never existed".
    let req = admin(Request::builder().uri("/api/secrets/test/main"))
        .body(Body::empty())
        .unwrap();
    let (_, body) = send(&app, req).await;
    let list: Value = serde_json::from_str(&body).unwrap();
    let secrets = list["secrets"].as_array().unwrap();
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0]["deleted"], true);
}

#[tokio::test]
async fn metadata_for_an_unknown_name_is_404() {
    let app = app("unknown-name");
    let req = admin(Request::builder().uri("/api/secrets/test/main/never_written"))
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- the admin gate -----------------------------------------------------

/// Every route refuses a non-admin caller. An unauthenticated request is the
/// weakest possible principal, so if any route let this through, the store
/// would be world-writable.
#[tokio::test]
async fn a_non_admin_is_refused_on_every_route() {
    let app = app("non-admin");
    // Seed through the admin path so the refusals below are about the gate,
    // not about missing data.
    put_secret(&app, "guarded", "v1").await;

    let attempts: Vec<(&str, &str, Body)> = vec![
        (
            "PUT",
            "/api/secrets/test/main/guarded",
            json_body(serde_json::json!({ "value": "x" })),
        ),
        ("GET", "/api/secrets/test/main", Body::empty()),
        ("GET", "/api/secrets/test/main/guarded", Body::empty()),
        ("DELETE", "/api/secrets/test/main/guarded", Body::empty()),
        (
            "POST",
            "/api/secrets/test/main/rotate/guarded",
            json_body(serde_json::json!({ "value": "x" })),
        ),
    ];

    for (method, uri, body) in attempts {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body)
            .unwrap();
        let (status, response) = send(&app, req).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {uri} should be refused, got {status}: {response}"
        );
    }

    // A wrong bearer is no better than none.
    let req = Request::builder()
        .uri("/api/secrets/test/main")
        .header("authorization", "Bearer not-the-superadmin-token")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---- resolution ---------------------------------------------------------

/// Resolution is the server-side USE path, deliberately unreachable over HTTP.
/// Exercised here against the same store the router holds, to prove the two
/// absences stay distinguishable end to end: a caller that retries on
/// `Pending` and gives up on `Gone` is correct only if they never merge.
#[tokio::test]
async fn resolution_distinguishes_pending_from_gone() {
    use raisin_models::secret_ref::SecretRef;
    use raisin_rocksdb::secret_store::SecretError;

    init_env();
    let path = "/tmp/raisin-secrets-test-resolve";
    let _ = std::fs::remove_dir_all(path);
    let storage = Arc::new(RocksDBStorage::new(path).unwrap());
    let keys = Arc::new(Keyring::new(vec![(1, [9u8; 32])], 1).unwrap());
    let store = SecretStore::new(
        storage.db().clone(),
        Arc::new(SecretBox::with_keyring(keys)),
        "test-node",
    );

    let scope = SecretScope::new("default", "test", "main");
    let owner = SecretOwner::actor("admin");
    store.put(&scope, "live", b"v1", &owner).unwrap();
    store.put(&scope, "retired", b"v1", &owner).unwrap();
    store.delete(&scope, "retired", &owner).unwrap();

    assert_eq!(
        store
            .resolve_string(&scope, &SecretRef::parse("secret://live").unwrap())
            .unwrap(),
        "v1"
    );

    match store.resolve(&scope, &SecretRef::parse("secret://retired").unwrap()) {
        Err(SecretError::Gone { name, .. }) => assert_eq!(name, "retired"),
        other => panic!("a tombstoned secret must be Gone, got {other:?}"),
    }

    match store.resolve(&scope, &SecretRef::parse("secret://absent").unwrap()) {
        Err(SecretError::Pending { name, .. }) => assert_eq!(name, "absent"),
        other => panic!("an absent secret must be Pending, got {other:?}"),
    }

    // The uniform convenience: a literal is `Ok(None)`, a broken reference is
    // an error — never the same answer.
    assert_eq!(
        store.resolve_if_reference(&scope, "a-literal").unwrap(),
        None
    );
    assert!(matches!(
        store.resolve_if_reference(&scope, "secret://retired"),
        Err(SecretError::Gone { .. })
    ));
}
