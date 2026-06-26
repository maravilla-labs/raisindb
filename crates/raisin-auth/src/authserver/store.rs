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
//! The server keeps two kinds of mutable state: registered [`OAuthClient`]s and
//! short-lived [`AuthorizationCode`]s. Both are expressed as traits so a managed
//! deployment can back them with RocksDB while the default
//! [`InMemoryAuthServerStore`] keeps everything in process — a real, complete
//! implementation suitable for single-node servers, tests, and the CLI dev
//! server.
//!
//! Traits use native `async fn` (Rust 1.89), so no `async-trait` shim is
//! required. They are `Send + Sync` so the HTTP layer can hold them behind an
//! `Arc` shared across request tasks.

use std::collections::HashMap;
use std::sync::RwLock;

use super::error::{AuthServerError, AuthServerResult};
use super::model::{AuthorizationCode, OAuthClient};

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

/// Storage for single-use authorization codes, scoped by tenant.
///
/// [`take_code`](AuthCodeStore::take_code) MUST remove the code as it returns
/// it so a replayed code cannot be exchanged twice (RFC 6749 §4.1.2).
pub trait AuthCodeStore: Send + Sync {
    /// Persist a freshly issued authorization code.
    fn put_code(
        &self,
        code: AuthorizationCode,
    ) -> impl std::future::Future<Output = AuthServerResult<()>> + Send;

    /// Atomically remove and return a code by value, if present.
    fn take_code(
        &self,
        tenant_id: &str,
        code: &str,
    ) -> impl std::future::Future<Output = AuthServerResult<Option<AuthorizationCode>>> + Send;
}

/// In-process, thread-safe implementation of both store traits.
///
/// Backed by `RwLock<HashMap>` maps keyed by `{tenant}\0{id}`. Expired codes
/// are swept lazily on access and can be purged eagerly via
/// [`InMemoryAuthServerStore::sweep_expired`]. This is the default store used
/// when no persistent backend is wired in.
#[derive(Default)]
pub struct InMemoryAuthServerStore {
    clients: RwLock<HashMap<String, OAuthClient>>,
    codes: RwLock<HashMap<String, AuthorizationCode>>,
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

    /// Remove every authorization code that has passed `now` (Unix seconds).
    ///
    /// Returns the number of codes purged. Callers may invoke this on a timer;
    /// correctness does not depend on it because [`AuthCodeStore::take_code`]
    /// also rejects expired codes.
    pub fn sweep_expired(&self, now: i64) -> usize {
        let mut codes = self
            .codes
            .write()
            .expect("authorization code store lock poisoned");
        let before = codes.len();
        codes.retain(|_, c| !c.is_expired(now));
        before - codes.len()
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

impl AuthCodeStore for InMemoryAuthServerStore {
    async fn put_code(&self, code: AuthorizationCode) -> AuthServerResult<()> {
        let key = Self::key(&code.tenant_id, &code.code);
        self.codes
            .write()
            .map_err(|_| AuthServerError::ServerError("code store lock poisoned".to_string()))?
            .insert(key, code);
        Ok(())
    }

    async fn take_code(
        &self,
        tenant_id: &str,
        code: &str,
    ) -> AuthServerResult<Option<AuthorizationCode>> {
        let key = Self::key(tenant_id, code);
        let removed = self
            .codes
            .write()
            .map_err(|_| AuthServerError::ServerError("code store lock poisoned".to_string()))?
            .remove(&key);

        // Reject (and drop) an expired code rather than returning it: a code
        // past its lifetime is no longer a valid grant.
        match removed {
            Some(c) if c.is_expired(chrono::Utc::now().timestamp()) => Ok(None),
            other => Ok(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authserver::model::TokenEndpointAuthMethod;
    use crate::authserver::pkce::CodeChallengeMethod;

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

    fn sample_code(value: &str, expires_at: i64) -> AuthorizationCode {
        AuthorizationCode {
            code: value.to_string(),
            client_id: "client-1".to_string(),
            tenant_id: "tenant-a".to_string(),
            redirect_uri: "http://localhost/cb".to_string(),
            code_challenge: "challenge".to_string(),
            code_challenge_method: CodeChallengeMethod::S256,
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            resource: "https://h/mcp/repo/main/srv".to_string(),
            scope: "reader".to_string(),
            expires_at,
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

    #[tokio::test]
    async fn code_is_single_use() {
        let store = InMemoryAuthServerStore::new();
        let future = chrono::Utc::now().timestamp() + 600;
        store.put_code(sample_code("abc", future)).await.unwrap();

        let first = store.take_code("tenant-a", "abc").await.unwrap();
        assert!(first.is_some());
        // Second take must be empty — the code was consumed.
        let second = store.take_code("tenant-a", "abc").await.unwrap();
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn expired_code_is_not_returned() {
        let store = InMemoryAuthServerStore::new();
        let past = chrono::Utc::now().timestamp() - 1;
        store.put_code(sample_code("old", past)).await.unwrap();

        let taken = store.take_code("tenant-a", "old").await.unwrap();
        assert!(taken.is_none());
    }

    #[test]
    fn sweep_removes_expired_only() {
        let store = InMemoryAuthServerStore::new();
        let now = chrono::Utc::now().timestamp();
        {
            // Insert directly to avoid the async take path.
            let mut codes = store.codes.write().unwrap();
            codes.insert(
                InMemoryAuthServerStore::key("tenant-a", "live"),
                sample_code("live", now + 600),
            );
            codes.insert(
                InMemoryAuthServerStore::key("tenant-a", "dead"),
                sample_code("dead", now - 1),
            );
        }
        let purged = store.sweep_expired(now);
        assert_eq!(purged, 1);
    }
}
