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
    ClientStore, OAuthClient, RefreshToken, RefreshTokenStore, TokenEndpointAuthMethod,
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

// ---------------------------------------------------------------------------
// Replication behaviour
//
// The apply handlers in `replication/application/oauth_operations.rs` delegate
// to this store rather than re-deriving its key layout, so the behaviour that
// needs proving lives here.
// ---------------------------------------------------------------------------

/// A revocation and a rotation of the same family are different replication
/// targets, so nothing orders them. A successor minted just before the replay
/// was noticed must not resurrect the family when it arrives afterwards — this
/// refusal is what the apply handler relies on.
#[tokio::test]
async fn a_token_from_a_revoked_family_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RocksDbOAuthStore::new(open(dir.path()));

    store
        .put_refresh_token(refresh("h1", "fam-1"))
        .await
        .unwrap();
    store
        .revoke_refresh_family("tenant-a", "fam-1")
        .await
        .unwrap();

    // The straggler arrives after the revocation.
    let err = store
        .put_refresh_token(refresh("h-late", "fam-1"))
        .await
        .expect_err("a revoked family must stay revoked");
    assert_eq!(err.code(), "invalid_grant");

    assert!(store
        .consume_refresh_token("tenant-a", "h-late", 1)
        .await
        .unwrap()
        .is_none());

    // An unrelated family is unaffected.
    store
        .put_refresh_token(refresh("h2", "fam-2"))
        .await
        .expect("a different family still works");
}

/// A key replicated from another node must actually authenticate here, which
/// means the non-tenant-scoped hash index has to be written too — storing only
/// the record leaves `validate_api_key` finding nothing.
#[tokio::test]
async fn a_replicated_api_key_validates_on_the_receiving_node() {
    use crate::api_key_store::ApiKeyStore;

    let dir = tempfile::tempdir().expect("tempdir");
    let db = open(dir.path());

    // Node A mints a key.
    let (key, raw_token) = ApiKeyStore::new(db.clone())
        .create_api_key("tenant-a", "user-1", "CI")
        .expect("create");

    // Node B receives it as a replicated record, in a database of its own.
    let dir_b = tempfile::tempdir().expect("tempdir");
    let node_b = ApiKeyStore::new(open(dir_b.path()));
    node_b.put_replicated(&key).expect("apply replicated key");

    let validated = node_b
        .validate_api_key(&raw_token)
        .expect("validate")
        .expect("the replicated key must authenticate on node B");
    assert_eq!(validated.key_id, key.key_id);
    assert_eq!(validated.tenant_id, "tenant-a");
}
