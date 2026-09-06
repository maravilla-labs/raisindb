// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Persistence abstractions for authorization-server state.
//!
//! The server persists two kinds of state: registered [`OAuthClient`]s and
//! [`RefreshToken`]s. Both are expressed as traits so the server can back them
//! with RocksDB (and replicate them) while [`InMemoryAuthServerStore`] keeps
//! everything in process for tests.
//!
//! Authorization codes are deliberately absent: they carry their own grant and
//! are never written down. See [`super::code_codec`].
//!
//! Traits use native `async fn` (Rust 1.89), so no `async-trait` shim is
//! required. They are `Send + Sync` so the HTTP layer can hold them behind an
//! `Arc` shared across request tasks.

use std::collections::HashMap;
use std::sync::RwLock;

use super::error::{AuthServerError, AuthServerResult};
use super::model::{OAuthClient, RefreshToken};

/// Storage for registered OAuth clients, scoped by tenant.
pub trait ClientStore: Send + Sync {
    /// Persist a newly registered client.
    fn put_client(
        &self,
        client: OAuthClient,
    ) -> impl std::future::Future<Output = AuthServerResult<()>> + Send;

    /// Fetch a client by id within a tenant. Returns `None` if unknown.
    fn get_client(
        &self,
        tenant_id: &str,
        client_id: &str,
    ) -> impl std::future::Future<Output = AuthServerResult<Option<OAuthClient>>> + Send;
}

/// Storage for refresh tokens, scoped by tenant.
///
/// Records are keyed by the **hash** of the token value, never the value
/// itself. [`consume_refresh_token`](RefreshTokenStore::consume_refresh_token)
/// marks rather than deletes, because replay detection needs to distinguish a
/// token that was already redeemed from one that never existed — see
/// [`super::refresh`].
pub trait RefreshTokenStore: Send + Sync {
    /// Persist a freshly issued refresh token.
    fn put_refresh_token(
        &self,
        token: RefreshToken,
    ) -> impl std::future::Future<Output = AuthServerResult<()>> + Send;

    /// Atomically mark a token consumed and return it **as it was before the
    /// mark**.
    ///
    /// Returning the prior state is what makes replay detectable: a record
    /// whose `consumed_at` was already set means the caller is the second
    /// presenter. Implementations MUST make the read-and-mark atomic against
    /// concurrent callers, or two racing redemptions both see an unconsumed
    /// token and neither is flagged.
    fn consume_refresh_token(
        &self,
        tenant_id: &str,
        token_hash: &str,
        now: i64,
    ) -> impl std::future::Future<Output = AuthServerResult<Option<RefreshToken>>> + Send;

    /// Revoke every token in a rotation family, returning how many were
    /// removed. Called when a replay is detected.
    fn revoke_refresh_family(
        &self,
        tenant_id: &str,
        family_id: &str,
    ) -> impl std::future::Future<Output = AuthServerResult<usize>> + Send;
}

/// In-process, thread-safe implementation of all three store traits.
///
/// Backed by `RwLock<HashMap>` maps keyed by `{tenant}\0{id}`. Expired codes
/// are swept lazily on access and can be purged eagerly via
/// [`InMemoryAuthServerStore::sweep_expired`].
///
/// **This store does not survive a restart.** Registered clients live only as
/// long as the process, so a client that registered via RFC 7591 and cached its
/// `client_id` — which is what ChatGPT, Claude and other MCP hosts do — gets
/// `invalid_client` after any redeploy or crash, with no way to recover except
/// removing and re-adding the connector. Use it for tests, the CLI dev server,
/// and single-node throwaway instances; any deployment a real MCP client
/// connects to wants a persistent implementation of the same traits.
#[derive(Default)]
pub struct InMemoryAuthServerStore {
    clients: RwLock<HashMap<String, OAuthClient>>,
    refresh_tokens: RwLock<HashMap<String, RefreshToken>>,
}

impl InMemoryAuthServerStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Compose the composite map key used for tenant isolation.
    fn key(tenant_id: &str, id: &str) -> String {
        format!("{tenant_id}\0{id}")
    }

    /// Number of currently registered clients (test/observability helper).
    pub fn client_count(&self) -> usize {
        self.clients
            .read()
            .expect("client store lock poisoned")
            .len()
    }
}

impl ClientStore for InMemoryAuthServerStore {
    async fn put_client(&self, client: OAuthClient) -> AuthServerResult<()> {
        let key = Self::key(&client.tenant_id, &client.client_id);
        self.clients
            .write()
            .map_err(|_| AuthServerError::ServerError("client store lock poisoned".to_string()))?
            .insert(key, client);
        Ok(())
    }

    async fn get_client(
        &self,
        tenant_id: &str,
        client_id: &str,
    ) -> AuthServerResult<Option<OAuthClient>> {
        let key = Self::key(tenant_id, client_id);
        Ok(self
            .clients
            .read()
            .map_err(|_| AuthServerError::ServerError("client store lock poisoned".to_string()))?
            .get(&key)
            .cloned())
    }
}

impl RefreshTokenStore for InMemoryAuthServerStore {
    async fn put_refresh_token(&self, token: RefreshToken) -> AuthServerResult<()> {
        let key = Self::key(&token.tenant_id, &token.token_hash);
        self.refresh_tokens
            .write()
            .map_err(|_| {
                AuthServerError::ServerError("refresh token store lock poisoned".to_string())
            })?
            .insert(key, token);
        Ok(())
    }

    async fn consume_refresh_token(
        &self,
        tenant_id: &str,
        token_hash: &str,
        now: i64,
    ) -> AuthServerResult<Option<RefreshToken>> {
        let key = Self::key(tenant_id, token_hash);
        // A write lock across the read and the mark makes the pair atomic.
        let mut tokens = self.refresh_tokens.write().map_err(|_| {
            AuthServerError::ServerError("refresh token store lock poisoned".to_string())
        })?;
        let Some(entry) = tokens.get_mut(&key) else {
            return Ok(None);
        };
        let before = entry.clone();
        if entry.consumed_at.is_none() {
            entry.consumed_at = Some(now);
        }
        Ok(Some(before))
    }

    async fn revoke_refresh_family(
        &self,
        tenant_id: &str,
        family_id: &str,
    ) -> AuthServerResult<usize> {
        let mut tokens = self.refresh_tokens.write().map_err(|_| {
            AuthServerError::ServerError("refresh token store lock poisoned".to_string())
        })?;
        let before = tokens.len();
        tokens.retain(|_, t| !(t.tenant_id == tenant_id && t.family_id == family_id));
        Ok(before - tokens.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authserver::model::TokenEndpointAuthMethod;

    fn sample_client() -> OAuthClient {
        OAuthClient {
            client_id: "client-1".to_string(),
            client_secret_hash: None,
            tenant_id: "tenant-a".to_string(),
            client_name: Some("Test".to_string()),
            redirect_uris: vec!["http://localhost/cb".to_string()],
            grant_types: vec!["authorization_code".to_string()],
            response_types: vec!["code".to_string()],
            scope: Some("reader".to_string()),
            token_endpoint_auth_method: TokenEndpointAuthMethod::None,
            created_at: 0,
        }
    }

    #[tokio::test]
    async fn client_round_trip_is_tenant_scoped() {
        let store = InMemoryAuthServerStore::new();
        store.put_client(sample_client()).await.unwrap();

        assert!(store
            .get_client("tenant-a", "client-1")
            .await
            .unwrap()
            .is_some());
        // Wrong tenant must not see it.
        assert!(store
            .get_client("tenant-b", "client-1")
            .await
            .unwrap()
            .is_none());
    }

    fn sample_refresh(hash: &str, family: &str) -> RefreshToken {
        RefreshToken {
            token_hash: hash.to_string(),
            family_id: family.to_string(),
            client_id: "client-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            resource: "https://h/mcp/repo/main/srv".to_string(),
            scope: "reader".to_string(),
            issued_at: 0,
            expires_at: i64::MAX,
            consumed_at: None,
        }
    }

    #[tokio::test]
    async fn consume_returns_prior_state_so_replay_is_visible() {
        let store = InMemoryAuthServerStore::new();
        store
            .put_refresh_token(sample_refresh("h1", "fam-1"))
            .await
            .unwrap();

        let first = store
            .consume_refresh_token("tenant-a", "h1", 100)
            .await
            .unwrap()
            .expect("token exists");
        assert!(!first.is_consumed(), "first redemption sees it unconsumed");

        let second = store
            .consume_refresh_token("tenant-a", "h1", 200)
            .await
            .unwrap()
            .expect("record is retained, not deleted");
        assert_eq!(
            second.consumed_at,
            Some(100),
            "replay must observe the original consumption"
        );
    }

    #[tokio::test]
    async fn revoking_a_family_drops_every_member() {
        let store = InMemoryAuthServerStore::new();
        store
            .put_refresh_token(sample_refresh("h1", "fam-1"))
            .await
            .unwrap();
        store
            .put_refresh_token(sample_refresh("h2", "fam-1"))
            .await
            .unwrap();
        store
            .put_refresh_token(sample_refresh("h3", "fam-2"))
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
        // A different family is untouched.
        assert!(store
            .consume_refresh_token("tenant-a", "h3", 1)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn refresh_tokens_are_tenant_scoped() {
        let store = InMemoryAuthServerStore::new();
        store
            .put_refresh_token(sample_refresh("h1", "fam-1"))
            .await
            .unwrap();
        assert!(store
            .consume_refresh_token("tenant-b", "h1", 1)
            .await
            .unwrap()
            .is_none());
    }
}
