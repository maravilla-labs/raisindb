// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for RaisinFunctionApi

use super::RaisinFunctionApi;

#[test]
fn test_glob_match() {
    // Test simple wildcard
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/*",
        "https://api.example.com/users"
    ));

    // Test double wildcard
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/**",
        "https://api.example.com/users/123/profile"
    ));

    // Test exact match
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/users",
        "https://api.example.com/users"
    ));

    // Test no match
    assert!(!RaisinFunctionApi::glob_match(
        "https://api.example.com/*",
        "https://api.other.com/users"
    ));
}

// ========== IMAP network-policy + credential-redaction tests ==========

use crate::api::callbacks::RaisinFunctionApiCallbacks;
use crate::types::{ExecutionContext, NetworkPolicy};

const SECRET_PW: &str = "SUPER_SECRET_APP_PASSWORD";

fn imap_conn(host: &str) -> serde_json::Value {
    serde_json::json!({
        "host": host,
        "port": 993,
        "tls": true,
        "username": "user@example.org",
        "password": SECRET_PW,
    })
}

fn api_with_allowed(allowed: &[&str]) -> RaisinFunctionApi {
    let policy = NetworkPolicy {
        http_enabled: true,
        allowed_urls: allowed.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    RaisinFunctionApi::new(ctx, policy, RaisinFunctionApiCallbacks::default())
}

/// A host that is not on the allow-list must be refused with a policy error
/// BEFORE any socket is opened — so the failure is a permission error, never a
/// connection/timeout error.
#[tokio::test]
async fn imap_disallowed_host_refused_before_connect() {
    let api = api_with_allowed(&["imaps://allowed.example.org:993"]);
    let conn = imap_conn("blocked.example.org");

    let err = api
        .impl_imap_fetch_since(conn, 0, None)
        .await
        .expect_err("disallowed host must be refused");

    let msg = err.to_string();
    // Policy denial, not a protocol/timeout error → proves the check ran first.
    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {msg}"
    );
    assert!(msg.contains("policy_denied"), "unexpected message: {msg}");
    // The password must never leak into an error surface.
    assert!(
        !msg.contains(SECRET_PW),
        "error message leaked the password: {msg}"
    );
}

/// Same guard on the other two entry points.
#[tokio::test]
async fn imap_disallowed_host_refused_all_ops() {
    let api = api_with_allowed(&["imaps://allowed.example.org:993"]);

    for r in [
        api.impl_imap_list_mailboxes(imap_conn("blocked.example.org"))
            .await,
        api.impl_imap_fetch_message(imap_conn("blocked.example.org"), 1, None)
            .await,
    ] {
        let err = r.expect_err("disallowed host must be refused");
        assert!(matches!(err, raisin_error::Error::PermissionDenied(_)));
        assert!(!err.to_string().contains(SECRET_PW));
    }
}

// ========== Crypto (verifyJwt) network-policy tests ==========

/// A JWKS URL whose host is not on the allow-list must be refused with a policy
/// error BEFORE any socket is opened — so the failure is a permission error,
/// never a fetch/timeout error.
#[tokio::test]
async fn verify_jwt_disallowed_jwks_refused_before_fetch() {
    let api = api_with_allowed(&["https://allowed.example.org/**"]);
    let opts = serde_json::json!({ "jwks_url": "https://blocked.example.org/jwks" });

    let err = api
        .impl_crypto_verify_jwt("header.payload.sig", opts)
        .await
        .expect_err("disallowed JWKS host must be refused");

    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {err}"
    );
    assert!(
        err.to_string().contains("policy_denied"),
        "unexpected message: {err}"
    );
}

/// A missing `jwks_url` is a soft failure (`{ valid:false }`), not a hard error,
/// and never touches the network.
#[tokio::test]
async fn verify_jwt_missing_jwks_url_is_soft_invalid() {
    let api = api_with_allowed(&[]);
    let out = api
        .impl_crypto_verify_jwt("t", serde_json::json!({}))
        .await
        .expect("missing jwks_url must not be a hard error");
    assert_eq!(out["valid"], serde_json::json!(false));
}

/// The connection descriptor must redact the password from any Debug render.
#[test]
fn imap_conn_debug_redacts_password() {
    use crate::runtime::imap::ImapConn;
    let conn = ImapConn::from_value(&imap_conn("imap.example.org")).unwrap();
    let dbg = format!("{conn:?}");
    assert!(!dbg.contains(SECRET_PW), "Debug leaked password: {dbg}");
    assert!(dbg.contains("<redacted>"), "Debug should mark redaction");
    assert!(dbg.contains("imap.example.org"));
}

// ========== Secret policy tests ==========
//
// The gate must run BEFORE the store is consulted, so every test here uses a
// `RaisinFunctionApiCallbacks::default()` — i.e. NO secret callbacks at all. A
// denial therefore proves the policy refused; if the gate ran second, the same
// call would surface the "subsystem not configured" validation error instead.

use crate::types::SecretPolicy;

fn api_with_secret_policy(policy: SecretPolicy) -> RaisinFunctionApi {
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    RaisinFunctionApi::new(
        ctx,
        NetworkPolicy::default(),
        RaisinFunctionApiCallbacks::default(),
    )
    .with_secret_policy(policy)
}

/// A recorded callback set, so an ALLOWED call can be shown to reach the store.
fn api_with_recording_callbacks(policy: SecretPolicy) -> RaisinFunctionApi {
    use std::sync::Arc;
    let callbacks = RaisinFunctionApiCallbacks::default()
        .with_secret_get(Arc::new(|name: String, v: Option<u64>| {
            // Echoes back what the API actually asked the store for, so a test
            // can assert BOTH the parsed name and the resolved version.
            Box::pin(async move {
                Ok(match v {
                    Some(v) => format!("plaintext-of-{name}@{v}"),
                    None => format!("plaintext-of-{name}"),
                })
            })
        }))
        .with_secret_list(Arc::new(|| {
            Box::pin(async move {
                Ok(vec![
                    serde_json::json!({ "name": "stripe/live", "version": 1 }),
                    serde_json::json!({ "name": "twilio/token", "version": 1 }),
                    serde_json::json!({ "version": 1 }), // nameless: must be dropped
                ])
            })
        }));
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    RaisinFunctionApi::new(ctx, NetworkPolicy::default(), callbacks).with_secret_policy(policy)
}

fn assert_policy_denied(err: raisin_error::Error, name: &str) {
    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {err}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("secrets:policy_denied"),
        "denial must carry the stable code: {msg}"
    );
    assert!(
        msg.contains(name),
        "denial must name the secret so the author can see WHICH grant is missing: {msg}"
    );
}

/// No policy at all = no secret access. This is the default a function gets
/// when its `.node.yaml` says nothing about secrets.
#[tokio::test]
async fn secret_get_denied_without_policy() {
    let api = api_with_secret_policy(SecretPolicy::default());
    let err = api
        .impl_secret_get("stripe/live", None)
        .await
        .expect_err("no policy must deny");
    assert_policy_denied(err, "stripe/live");
}

/// Every mutating operation is gated too — a function that cannot read a
/// secret must not be able to overwrite, rotate or tombstone it either.
#[tokio::test]
async fn secret_writes_denied_without_policy() {
    let api = api_with_secret_policy(SecretPolicy::default());

    assert_policy_denied(
        api.impl_secret_put("stripe/live", "v").await.unwrap_err(),
        "stripe/live",
    );
    assert_policy_denied(
        api.impl_secret_rotate("stripe/live", "v")
            .await
            .unwrap_err(),
        "stripe/live",
    );
    assert_policy_denied(
        api.impl_secret_delete("stripe/live").await.unwrap_err(),
        "stripe/live",
    );

    let err = api.impl_secret_list().await.unwrap_err();
    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "list must be denied too, got: {err}"
    );
}

/// `enabled: true` with an empty allow-list must still deny: "opted in" can
/// never silently mean "unrestricted".
#[tokio::test]
async fn secret_get_denied_when_allowlist_empty() {
    let api = api_with_secret_policy(SecretPolicy {
        enabled: true,
        allowed_names: Vec::new(),
    });
    let err = api
        .impl_secret_get("stripe/live", None)
        .await
        .expect_err("empty allow-list must deny");
    assert_policy_denied(err, "stripe/live");
}

/// A name outside the allow-list is denied, and the error says so rather than
/// returning an empty/None result that reads like "no such secret".
#[tokio::test]
async fn secret_get_denied_for_unlisted_name() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["stripe/*".to_string()]));
    let err = api
        .impl_secret_get("twilio/token", None)
        .await
        .expect_err("unlisted name must be denied");
    assert_policy_denied(err, "twilio/token");
}

/// An allow-listed name reaches the store. Combined with the tests above, this
/// is what shows the gate is a gate and not a wall.
#[tokio::test]
async fn secret_get_allowed_for_exact_name() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec![
        "sendgrid_api_key".to_string()
    ]));
    let value = api
        .impl_secret_get("sendgrid_api_key", None)
        .await
        .expect("allow-listed name must be readable");
    assert_eq!(value, "plaintext-of-sendgrid_api_key");
}

/// A glob grant covers every name it matches — and nothing else.
#[tokio::test]
async fn secret_get_allowed_by_glob() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["stripe/*".to_string()]));

    assert_eq!(
        api.impl_secret_get("stripe/live", None).await.unwrap(),
        "plaintext-of-stripe/live"
    );
    assert_eq!(
        api.impl_secret_get("stripe/test", None).await.unwrap(),
        "plaintext-of-stripe/test"
    );

    // The glob does not reach a sibling prefix.
    assert_policy_denied(
        api.impl_secret_get("stripeX/live", None).await.unwrap_err(),
        "stripeX/live",
    );
}

/// `list` filters to the allowed names rather than denying, so a granted
/// function can discover what it may use — but it must NOT be able to
/// enumerate credentials it cannot read.
#[tokio::test]
async fn secret_list_filters_to_allowed_names() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["stripe/*".to_string()]));

    let listed = api.impl_secret_list().await.expect("list must be allowed");
    let names: Vec<&str> = listed
        .iter()
        .filter_map(|e| e.get("name").and_then(|n| n.as_str()))
        .collect();

    assert_eq!(
        names,
        vec!["stripe/live"],
        "list must drop both the unlisted name and the nameless entry"
    );
}

// ========== secret:// reference handling ==========
//
// The primary use case: a node property in an `encrypted: true` field holds
// `secret://node/{id}/{field}@N`, node reads never resolve it, so the function
// is handed the whole reference string and passes it straight to `get`.

/// Both spellings of ONE secret must get ONE answer. If the policy were matched
/// against the raw argument instead of the parsed name, a caller could dodge
/// an allow-list simply by re-spelling — deny a bare name, allow the reference,
/// or vice versa.
#[tokio::test]
async fn policy_matches_the_parsed_name_for_both_spellings() {
    let allowed = api_with_recording_callbacks(SecretPolicy::allow_names(vec![
        "node/01H8XY/api_key".to_string(),
    ]));

    // Allowed: identical outcome whichever way it is named.
    assert_eq!(
        allowed
            .impl_secret_get("node/01H8XY/api_key", None)
            .await
            .unwrap(),
        "plaintext-of-node/01H8XY/api_key"
    );
    assert_eq!(
        allowed
            .impl_secret_get("secret://node/01H8XY/api_key", None)
            .await
            .unwrap(),
        "plaintext-of-node/01H8XY/api_key"
    );

    // Denied: also identical whichever way it is named.
    let denied =
        api_with_recording_callbacks(SecretPolicy::allow_names(vec!["stripe/*".to_string()]));
    assert_policy_denied(
        denied
            .impl_secret_get("node/01H8XY/api_key", None)
            .await
            .unwrap_err(),
        "node/01H8XY/api_key",
    );
    assert_policy_denied(
        denied
            .impl_secret_get("secret://node/01H8XY/api_key@3", None)
            .await
            .unwrap_err(),
        // The denial names the PARSED name, not the raw reference.
        "node/01H8XY/api_key",
    );
}

/// A glob grant covers the reference spelling too — `node/*` must not require
/// the author to also write `secret://node/*`.
#[tokio::test]
async fn a_glob_grant_covers_the_reference_spelling() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["node/*".to_string()]));
    assert_eq!(
        api.impl_secret_get("secret://node/01H8XY/api_key", None)
            .await
            .unwrap(),
        "plaintext-of-node/01H8XY/api_key"
    );
}

/// A version pinned INSIDE the reference reaches the store. This is what keeps
/// a time-travel read honest: an older node revision holds an older version,
/// and reading it must give what the node actually held then, not the latest.
#[tokio::test]
async fn a_pinned_version_in_the_reference_is_honoured() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["k".to_string()]));

    assert_eq!(
        api.impl_secret_get("secret://k@1", None).await.unwrap(),
        "plaintext-of-k@1"
    );
    // Unpinned reads the newest.
    assert_eq!(
        api.impl_secret_get("secret://k", None).await.unwrap(),
        "plaintext-of-k"
    );
    // An explicit argument alongside an UNPINNED reference is unambiguous.
    assert_eq!(
        api.impl_secret_get("secret://k", Some(4)).await.unwrap(),
        "plaintext-of-k@4"
    );
    // ...but a pin AND an argument state two versions, which is refused rather
    // than silently resolved: the whole point of a pinned reference is that
    // reading an old node revision cannot quietly return a value it never held.
    let err = api
        .impl_secret_get("secret://k@1", Some(4))
        .await
        .expect_err("two stated versions must be refused");
    let msg = err.to_string();
    assert!(msg.contains('1') && msg.contains('4'), "name both: {msg}");
}

/// The `@` rule the API exists to keep out of function authors' hands: a name
/// may contain `@`, so `secret://ops@example.com` is a NAME, not a pin.
#[tokio::test]
async fn an_at_sign_in_the_name_is_not_read_as_a_version() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec![
        "ops@example.com".to_string()
    ]));
    assert_eq!(
        api.impl_secret_get("secret://ops@example.com", None)
            .await
            .unwrap(),
        "plaintext-of-ops@example.com"
    );
}

/// `resolve` passes a non-reference through untouched — and does so WITHOUT a
/// policy check, since nothing secret was touched. That is what lets an adapter
/// read a config field that is a literal on one deployment and a vaulted
/// reference on another with one line of code.
#[tokio::test]
async fn resolve_passes_a_literal_through_unchanged() {
    let api = api_with_recording_callbacks(SecretPolicy::default()); // no grants at all
    assert_eq!(
        api.impl_secret_resolve("a-literal-password").await.unwrap(),
        "a-literal-password"
    );
}

/// `resolve` on an actual reference resolves it — and is still gated.
#[tokio::test]
async fn resolve_resolves_a_reference_and_is_still_gated() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["k".to_string()]));
    assert_eq!(
        api.impl_secret_resolve("secret://k@2").await.unwrap(),
        "plaintext-of-k@2"
    );

    let denied = api_with_recording_callbacks(SecretPolicy::default());
    assert_policy_denied(
        denied.impl_secret_resolve("secret://k").await.unwrap_err(),
        "k",
    );
}

/// A write against a PINNED reference is refused rather than silently ignoring
/// the pin, which would let an author believe they had replaced version 1.
#[tokio::test]
async fn a_write_refuses_a_pinned_reference() {
    let api = api_with_recording_callbacks(SecretPolicy::allow_names(vec!["k".to_string()]));

    for err in [
        api.impl_secret_put("secret://k@1", "v").await.unwrap_err(),
        api.impl_secret_rotate("secret://k@1", "v")
            .await
            .unwrap_err(),
        api.impl_secret_delete("secret://k@1").await.unwrap_err(),
    ] {
        assert!(
            err.to_string().contains("append"),
            "refusal should explain that writes append: {err}"
        );
    }
}

// ========== Email policy tests ==========
//
// Same construction as the secret-policy tests: no email-capable callbacks and
// no `/config/email` node, so a denial can only have come from the policy. If
// the gate ran after `authorize_email`, these calls would surface the
// "outbound email is not configured for this tenant" state error instead —
// which is exactly what proves the recipient check runs first, before the
// config is read, before the credential is decrypted, and long before a socket.

use crate::types::EmailPolicy;

fn api_with_email_policy(policy: EmailPolicy) -> RaisinFunctionApi {
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    RaisinFunctionApi::new(
        ctx,
        NetworkPolicy::default(),
        RaisinFunctionApiCallbacks::default(),
    )
    .with_email_policy(policy)
}

fn message_to(to: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "to": to,
        "subject": "Sign in",
        "text": "https://app.example.com/verify?token=…",
    })
}

/// A function that never declared `email_policy` cannot send. The default is
/// the whole point of the type.
#[tokio::test]
async fn email_send_denied_without_policy() {
    let api = api_with_email_policy(EmailPolicy::default());
    let err = api
        .impl_email_send(message_to(&["user@example.com"]))
        .await
        .expect_err("no policy must deny");

    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("email:policy_denied"), "unexpected: {msg}");
    assert!(
        msg.contains("no email_policy"),
        "the denial must name what is missing: {msg}"
    );
}

/// `enabled: true` with nothing listed is still a denial — "enabled but
/// unconfigured" never means "send anywhere".
#[tokio::test]
async fn email_send_denied_when_allowlist_empty() {
    let api = api_with_email_policy(EmailPolicy {
        enabled: true,
        allowed_recipients: Vec::new(),
    });
    let err = api
        .impl_email_send(message_to(&["user@example.com"]))
        .await
        .expect_err("an empty allow-list must deny");
    assert!(matches!(err, raisin_error::Error::PermissionDenied(_)));
}

/// One forbidden recipient refuses the WHOLE message. Dropping it silently
/// would let the caller believe mail went somewhere it did not.
#[tokio::test]
async fn email_send_denied_when_any_recipient_is_disallowed() {
    let api = api_with_email_policy(EmailPolicy::allow_recipients(vec![
        "example.com".to_string()
    ]));
    let err = api
        .impl_email_send(message_to(&["ok@example.com", "nope@elsewhere.test"]))
        .await
        .expect_err("a mixed list must be refused whole");

    let msg = err.to_string();
    assert!(matches!(err, raisin_error::Error::PermissionDenied(_)));
    assert!(
        msg.contains("nope@elsewhere.test") && !msg.contains("ok@example.com"),
        "the denial should name only what was refused: {msg}"
    );
}

/// Naming a provider does not skip the policy. The recipient check runs before
/// the config is read, so a function without a policy cannot even learn whether
/// the provider it named exists.
#[tokio::test]
async fn naming_a_provider_does_not_bypass_the_recipient_policy() {
    let api = api_with_email_policy(EmailPolicy::allow_recipients(vec![
        "example.com".to_string()
    ]));
    let mut message = message_to(&["nope@elsewhere.test"]);
    message["provider"] = serde_json::json!("marketing");

    let err = api
        .impl_email_send(message)
        .await
        .expect_err("a forbidden recipient must be refused whatever the provider");
    assert!(matches!(err, raisin_error::Error::PermissionDenied(_)));
    assert!(err.to_string().contains("email:policy_denied"));
}

/// Listing providers is gated on the same policy as sending: a function that
/// may not send has no use for the names, and should not learn them.
#[tokio::test]
async fn listing_providers_needs_the_email_policy() {
    let denied = api_with_email_policy(EmailPolicy::default())
        .impl_email_providers()
        .await
        .expect_err("no policy, no listing");
    assert!(matches!(denied, raisin_error::Error::PermissionDenied(_)));
    assert!(denied.to_string().contains("email:policy_denied"));

    // And with a policy it gets past the gate, failing at the config lookup
    // instead — without this the assertion above would also pass if listing
    // were denied unconditionally.
    let allowed = api_with_email_policy(EmailPolicy::allow_recipients(vec![
        "example.com".to_string()
    ]))
    .impl_email_providers()
    .await
    .expect_err("there is no /config/email node in this test");
    assert!(
        !matches!(allowed, raisin_error::Error::PermissionDenied(_)),
        "a granted function must not be refused by the policy: {allowed}"
    );
}

/// The complement: an ALLOWED recipient gets past the policy and fails at the
/// next gate instead. Without this the tests above would also pass if the gate
/// denied everything unconditionally.
#[tokio::test]
async fn an_allowed_recipient_reaches_the_config_gate() {
    let api = api_with_email_policy(EmailPolicy::allow_recipients(vec![
        "example.com".to_string()
    ]));
    let err = api
        .impl_email_send(message_to(&["user@example.com"]))
        .await
        .expect_err("there is no /config/email node in this test");

    assert!(
        !matches!(err, raisin_error::Error::PermissionDenied(_)),
        "an allowed recipient must not be refused by the email policy: {err}"
    );
    // With no node callback wired up in this test, the next gate is the
    // `/config/email` LOOKUP failing — which is still proof the call got past
    // the policy and into `authorize_email`.
    assert!(
        err.to_string().contains("Node get"),
        "expected the config lookup to be what refused: {err}"
    );
}

// ========== is_url_allowed divergence (issue #13) ==========

/// The PERMISSIVE half of the divergence, pinned deliberately: on this path
/// `http_enabled: true` with an empty `allowed_urls` allows everything. The
/// strict half is pinned by `types::config::tests::network_policy_denies_an_empty_allowlist`.
/// Unifying the two is a follow-up needing a deprecation window — until then a
/// change to either behaviour must break one of these two tests, not slip
/// through.
#[test]
fn an_empty_allowlist_still_allows_everything_on_this_path() {
    let api = api_with_allowed(&[]);
    assert!(api.is_url_allowed("https://anything.example.com/whatever"));
}

/// ...but only when HTTP was enabled at all.
#[test]
fn an_empty_allowlist_allows_nothing_when_http_is_disabled() {
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    let api = RaisinFunctionApi::new(
        ctx,
        NetworkPolicy::default(),
        RaisinFunctionApiCallbacks::default(),
    );
    assert!(!api.is_url_allowed("https://anything.example.com/whatever"));
}

/// Both paths now run the SAME matcher, so `*` stops at `/` here too. This is
/// the half of issue #13 that HAS been unified.
#[test]
fn both_paths_agree_on_what_a_single_star_means() {
    let api = api_with_allowed(&["https://api.example.com/*"]);
    assert!(api.is_url_allowed("https://api.example.com/users"));
    assert!(!api.is_url_allowed("https://api.example.com/users/123"));

    let policy = NetworkPolicy::allow_urls(vec!["https://api.example.com/*".to_string()]);
    assert_eq!(
        policy.is_url_allowed("https://api.example.com/users/123"),
        api.is_url_allowed("https://api.example.com/users/123"),
    );
}

// ============================================================================
// raisin.identities.* — the IdentityPolicy gate and edge normalisation
// ============================================================================
//
// Every test here runs with NO identity callbacks wired. A PermissionDenied
// therefore proves the policy refused before the seam was reached; a recording
// callback shows what an ALLOWED call actually hands over.

use crate::api::IdentityPatch;
use crate::types::IdentityPolicy;

fn api_with_identity_policy(policy: IdentityPolicy) -> RaisinFunctionApi {
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    RaisinFunctionApi::new(
        ctx,
        NetworkPolicy::default(),
        RaisinFunctionApiCallbacks::default(),
    )
    .with_identity_policy(policy)
}

/// Records what reached the callbacks, so a test can assert on the normalised
/// email and the parsed patch rather than on a return value.
fn api_with_recording_identity_callbacks() -> (
    RaisinFunctionApi,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    std::sync::Arc<std::sync::Mutex<Vec<(String, IdentityPatch)>>>,
) {
    use std::sync::{Arc, Mutex};
    let lookups: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let updates: Arc<Mutex<Vec<(String, IdentityPatch)>>> = Arc::new(Mutex::new(Vec::new()));
    let l = lookups.clone();
    let u = updates.clone();
    let callbacks = RaisinFunctionApiCallbacks::default()
        .with_identity_find_by_email(Arc::new(move |email: String| {
            l.lock().unwrap().push(email.clone());
            Box::pin(async move {
                Ok(if email == "ada@example.com" {
                    Some(serde_json::json!({ "id": "id-ada", "email": email }))
                } else {
                    None
                })
            })
        }))
        .with_identity_update(Arc::new(move |id: String, patch: IdentityPatch| {
            u.lock().unwrap().push((id.clone(), patch));
            Box::pin(async move { Ok(serde_json::json!({ "id": id })) })
        }));
    let ctx = ExecutionContext::new("tenant1", "repo1", "main", "test-user");
    let api = RaisinFunctionApi::new(ctx, NetworkPolicy::default(), callbacks)
        .with_identity_policy(IdentityPolicy::allow());
    (api, lookups, updates)
}

fn assert_identity_policy_denied(err: raisin_error::Error) {
    assert!(
        matches!(err, raisin_error::Error::PermissionDenied(_)),
        "expected PermissionDenied, got: {err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("[identities:policy_denied]"), "{msg}");
    assert!(
        msg.contains("identity_policy"),
        "the denial must name the block that grants it: {msg}"
    );
}

/// A function that never declared `identity_policy` can neither look up nor
/// update — and the lookup denial is an ERROR, never `None`.
#[tokio::test]
async fn identities_denied_without_policy() {
    let api = api_with_identity_policy(IdentityPolicy::default());
    let err = api
        .impl_identity_find_by_email("ada@example.com")
        .await
        .expect_err("no policy must deny findByEmail");
    assert_identity_policy_denied(err);

    let err = api
        .impl_identity_update("id-1", serde_json::json!({ "display_name": "Ada" }))
        .await
        .expect_err("no policy must deny update");
    assert_identity_policy_denied(err);
}

/// `enabled: false` spelled out is the same as absent.
#[tokio::test]
async fn identities_denied_when_explicitly_disabled() {
    let api = api_with_identity_policy(IdentityPolicy { enabled: false });
    let err = api
        .impl_identity_find_by_email("ada@example.com")
        .await
        .unwrap_err();
    assert_identity_policy_denied(err);
}

/// The gate runs BEFORE the patch is parsed: an invalid patch under a denying
/// policy is still reported as a denial, so the error names the missing grant
/// rather than a field the author cannot use anyway.
#[tokio::test]
async fn identities_policy_runs_before_patch_validation() {
    let api = api_with_identity_policy(IdentityPolicy::default());
    let err = api
        .impl_identity_update("id-1", serde_json::json!({ "email_verified": true }))
        .await
        .unwrap_err();
    assert_identity_policy_denied(err);
}

/// Allowed, but no repository on this execution path: a clear error, not a
/// silent null.
#[tokio::test]
async fn identities_allowed_without_callbacks_reports_unavailable() {
    let api = api_with_identity_policy(IdentityPolicy::allow());
    let err = api
        .impl_identity_find_by_email("ada@example.com")
        .await
        .unwrap_err();
    assert!(
        matches!(err, raisin_error::Error::Validation(_)),
        "expected Validation, got: {err}"
    );
    assert!(err.to_string().contains("not available"), "{err}");
}

/// The email is trimmed and lowercased at the edge, so the store is only ever
/// asked for one spelling.
#[tokio::test]
async fn identities_find_by_email_normalises_the_address() {
    let (api, lookups, _) = api_with_recording_identity_callbacks();
    let found = api
        .impl_identity_find_by_email("  Ada@Example.COM \n")
        .await
        .unwrap();
    assert_eq!(found.unwrap()["id"], "id-ada");
    assert_eq!(lookups.lock().unwrap().as_slice(), ["ada@example.com"]);

    let missing = api
        .impl_identity_find_by_email("nobody@example.com")
        .await
        .unwrap();
    assert!(missing.is_none(), "unknown address is None, not an error");
}

#[tokio::test]
async fn identities_find_by_email_rejects_an_empty_address() {
    let (api, lookups, _) = api_with_recording_identity_callbacks();
    let err = api.impl_identity_find_by_email("   ").await.unwrap_err();
    assert!(
        err.to_string().contains("[identities:invalid_email]"),
        "{err}"
    );
    assert!(
        lookups.lock().unwrap().is_empty(),
        "nothing may reach the store"
    );
}

#[tokio::test]
async fn identities_update_normalises_the_patch() {
    let (api, _, updates) = api_with_recording_identity_callbacks();
    api.impl_identity_update(
        " id-1 ",
        serde_json::json!({
            "email": " New@Example.COM ",
            "password": "hunter2-but-longer",
            "display_name": "  Ada  ",
        }),
    )
    .await
    .unwrap();

    let recorded = updates.lock().unwrap();
    let (id, patch) = &recorded[0];
    assert_eq!(id, "id-1");
    assert_eq!(patch.email.as_deref(), Some("new@example.com"));
    assert_eq!(patch.password.as_deref(), Some("hunter2-but-longer"));
    assert_eq!(patch.display_name.as_deref(), Some("Ada"));
}

/// `email_verified` is refused — not ignored — so an author learns that the
/// binding cannot do it instead of believing it silently worked.
#[tokio::test]
async fn identities_update_refuses_email_verified() {
    let (api, _, updates) = api_with_recording_identity_callbacks();
    let err = api
        .impl_identity_update(
            "id-1",
            serde_json::json!({ "email": "a@example.com", "email_verified": true }),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("[identities:invalid_patch]"),
        "{err}"
    );
    assert!(err.to_string().contains("email_verified"), "{err}");
    assert!(
        updates.lock().unwrap().is_empty(),
        "nothing may reach the store"
    );
}

#[tokio::test]
async fn identities_update_refuses_malformed_patches() {
    let (api, _, updates) = api_with_recording_identity_callbacks();
    for bad in [
        serde_json::json!("not an object"),
        serde_json::json!({}),
        serde_json::json!({ "email": "no-at-sign" }),
        serde_json::json!({ "email": 42 }),
        serde_json::json!({ "password": "" }),
        serde_json::json!({ "avatar_url": "https://x" }),
    ] {
        let err = api
            .impl_identity_update("id-1", bad.clone())
            .await
            .expect_err(&format!("must refuse {bad}"));
        assert!(
            err.to_string().contains("[identities:invalid_patch]"),
            "{bad}: {err}"
        );
    }
    let err = api
        .impl_identity_update("", serde_json::json!({ "display_name": "x" }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("[identities:invalid_id]"), "{err}");
    assert!(updates.lock().unwrap().is_empty());
}

/// The patch's Debug output carries a password across the seam, so it must
/// never render it — a `{:?}` in a log line is the obvious leak.
#[test]
fn identity_patch_debug_redacts_the_password() {
    let patch = IdentityPatch {
        email: Some("a@example.com".to_string()),
        password: Some("correct horse battery staple".to_string()),
        display_name: None,
    };
    let rendered = format!("{patch:?}");
    assert!(!rendered.contains("correct horse"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}
