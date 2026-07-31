// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for the durable authorization-server store.
//!
//! The one that matters most is [`clients_survive_a_reopen`]: it is the
//! regression test for connectors breaking on every server restart.

use std::sync::Arc;

use raisin_auth::authserver::{
    AuthCodeStore, AuthorizationCode, ClientStore, CodeChallengeMethod, OAuthClient, RefreshToken,
    RefreshTokenStore, TokenEndpointAuthMethod,
};

use super::RocksDbOAuthStore;

/// Open a database with the column families the store needs, at `path`.
fn open(path: &std::path::Path) -> Arc<rocksdb::DB> {
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db =
        rocksdb::DB::open_cf(&opts, path, [crate::cf::ADMIN_USERS]).expect("open test database");
    Arc::new(db)
}

fn client(id: &str) -> OAuthClient {
    OAuthClient {
        client_id: id.to_string(),
        client_secret_hash: None,
        tenant_id: "tenant-a".to_string(),
        client_name: Some("ChatGPT".to_string()),
        redirect_uris: vec!["https://chatgpt.com/connector/oauth/abc".to_string()],
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        response_types: vec!["code".to_string()],
        scope: Some("reader".to_string()),
        token_endpoint_auth_method: TokenEndpointAuthMethod::None,
        created_at: 1_700_000_000,
    }
}

fn code(value: &str, expires_at: i64) -> AuthorizationCode {
    AuthorizationCode {
        code: value.to_string(),
        client_id: "client-1".to_string(),
        tenant_id: "tenant-a".to_string(),
        redirect_uri: "https://chatgpt.com/connector/oauth/abc".to_string(),
        code_challenge: "challenge".to_string(),
        code_challenge_method: CodeChallengeMethod::S256,
        identity_id: "id-1".to_string(),
        email: "u@example.com".to_string(),
        repository: "studio".to_string(),
        branch: "main".to_string(),
        resource: "https://h/mcp/studio/main/studio".to_string(),
        scope: "reader".to_string(),
        expires_at,
    }
}

fn refresh(hash: &str, family: &str) -> RefreshToken {
    RefreshToken {
        token_hash: hash.to_string(),
        family_id: family.to_string(),
        client_id: "client-1".to_string(),
        tenant_id: "tenant-a".to_string(),
        identity_id: "id-1".to_string(),
        email: "u@example.com".to_string(),
        repository: "studio".to_string(),
        branch: "main".to_string(),
        resource: "https://h/mcp/studio/main/studio".to_string(),
        scope: "reader".to_string(),
        issued_at: 0,
        expires_at: i64::MAX,
        consumed_at: None,
    }
}

/// The regression test for the ChatGPT `unknown client_id` failure: a client
/// registered before a restart must still resolve after it.
#[tokio::test]
async fn clients_survive_a_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");

    {
        let store = RocksDbOAuthStore::new(open(dir.path()));
        store.put_client(client("client-1")).await.unwrap();
    } // database closed — simulates the server process exiting

    let store = RocksDbOAuthStore::new(open(dir.path()));
    let loaded = store
        .get_client("tenant-a", "client-1")
        .await
        .unwrap()
        .expect("client must outlive the process");
    assert_eq!(loaded.client_name.as_deref(), Some("ChatGPT"));
    assert_eq!(
        loaded.redirect_uris,
        vec!["https://chatgpt.com/connector/oauth/abc"]
    );
    assert!(loaded.allows_grant_type("refresh_token"));
}

#[tokio::test]
async fn clients_are_tenant_scoped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RocksDbOAuthStore::new(open(dir.path()));
    store.put_client(client("client-1")).await.unwrap();

    assert!(store
        .get_client("tenant-b", "client-1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn codes_are_single_use_and_reject_expiry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RocksDbOAuthStore::new(open(dir.path()));
    let now = chrono::Utc::now().timestamp();

    store.put_code(code("live", now + 600)).await.unwrap();
    assert!(store.take_code("tenant-a", "live").await.unwrap().is_some());
    assert!(
        store.take_code("tenant-a", "live").await.unwrap().is_none(),
        "a code must not be redeemable twice"
    );

    store.put_code(code("stale", now - 1)).await.unwrap();
    assert!(store
        .take_code("tenant-a", "stale")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn refresh_consume_reports_prior_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RocksDbOAuthStore::new(open(dir.path()));
    store
        .put_refresh_token(refresh("h1", "fam-1"))
        .await
        .unwrap();

    let first = store
        .consume_refresh_token("tenant-a", "h1", 100)
        .await
        .unwrap()
        .expect("token exists");
    assert!(!first.is_consumed());

    let replay = store
        .consume_refresh_token("tenant-a", "h1", 200)
        .await
        .unwrap()
        .expect("record retained for replay detection");
    assert_eq!(replay.consumed_at, Some(100));
}

#[tokio::test]
async fn revoking_a_family_removes_members_and_their_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RocksDbOAuthStore::new(open(dir.path()));

    store
        .put_refresh_token(refresh("h1", "fam-1"))
        .await
        .unwrap();
    store
        .put_refresh_token(refresh("h2", "fam-1"))
        .await
        .unwrap();
    store
        .put_refresh_token(refresh("h3", "fam-2"))
        .await
        .unwrap();

    let revoked = store
        .revoke_refresh_family("tenant-a", "fam-1")
        .await
        .unwrap();
    assert_eq!(revoked, 2);

    assert!(store
        .consume_refresh_token("tenant-a", "h1", 1)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .consume_refresh_token("tenant-a", "h2", 1)
        .await
        .unwrap()
        .is_none());
    // An unrelated family is untouched.
    assert!(store
        .consume_refresh_token("tenant-a", "h3", 1)
        .await
        .unwrap()
        .is_some());

    // A second revoke finds nothing: the index entries went too.
    assert_eq!(
        store
            .revoke_refresh_family("tenant-a", "fam-1")
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn refresh_tokens_survive_a_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let store = RocksDbOAuthStore::new(open(dir.path()));
        store
            .put_refresh_token(refresh("h1", "fam-1"))
            .await
            .unwrap();
    }
    let store = RocksDbOAuthStore::new(open(dir.path()));
    assert!(store
        .consume_refresh_token("tenant-a", "h1", 1)
        .await
        .unwrap()
        .is_some());
}
