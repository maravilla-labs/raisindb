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

//! A generic OAuth 2.1 Authorization Server.
//!
//! This module implements the server side of OAuth 2.1 so that interactive MCP
//! clients — which perform OAuth discovery, PKCE, and Dynamic Client
//! Registration — can authenticate against RaisinDB and obtain an access token
//! scoped to a specific MCP resource.
//!
//! # What it provides
//!
//! - **Discovery metadata** ([`metadata`]): RFC 8414 authorization-server
//!   metadata and RFC 9728 protected-resource metadata.
//! - **Authorization endpoint** ([`authorize`]): validates the request, requires
//!   PKCE `S256`, and (once the HTTP layer has authenticated the resource owner
//!   through the *existing* identity/login flow) issues a single-use code.
//! - **Token endpoint** ([`token`]): authenticates the client, verifies the PKCE
//!   `code_verifier`, and yields a [`token::TokenGrant`] the resource server signs
//!   into a JWT whose audience is the MCP resource and whose `scope` claim carries
//!   the consented scopes.
//! - **Refresh grant** ([`refresh`]): rotating refresh tokens with replay
//!   detection, so a client whose one-hour access token expires does not have to
//!   send the user back through consent.
//! - **Dynamic Client Registration** ([`registration`]): RFC 7591.
//! - **Scope mapping** ([`scope`]): consented OAuth scopes are role/group ids the
//!   identity already holds, so dispatch's scope gating and RLS stay consistent.
//!
//! # Storage
//!
//! Clients, authorization codes and refresh tokens are persisted through the
//! [`store::ClientStore`], [`store::AuthCodeStore`] and
//! [`store::RefreshTokenStore`] traits, so the protocol logic here is
//! independent of the backend.
//!
//! **Registered clients must be durable.** An MCP host performs Dynamic Client
//! Registration once and then caches the issued `client_id` indefinitely; a
//! store that empties on restart therefore breaks every connector that ever
//! registered, permanently, with `invalid_client` at `/authorize`. The server
//! ships a RocksDB implementation (`raisin_rocksdb::RocksDbOAuthStore`) and uses
//! it everywhere. [`store::InMemoryAuthServerStore`] implements the same traits
//! for this crate's tests and for embedders that genuinely want throwaway state.
//!
//! # Token verification
//!
//! The minted token reuses the resource server's existing JWT machinery. It sets
//! the `aud` claim (the MCP resource) and a `scope` claim; the resource server's
//! token validator accepts these, and the MCP transport derives the caller's
//! granted scopes from the same role/group set, so a token issued here gates
//! exactly the operations the identity is entitled to.

pub mod authorize;
pub mod error;
pub mod metadata;
pub mod model;
pub mod pkce;
pub mod refresh;
pub mod registration;
pub mod scope;
pub mod store;
pub mod token;

use std::sync::Arc;

pub use authorize::{
    issue_authorization_code, validate_authorization_request, AuthorizationRequest, ResourceOwner,
    ValidatedAuthorizationRequest, AUTH_CODE_TTL_SECONDS,
};
pub use error::{AuthServerError, AuthServerResult, OAuthErrorBody};
pub use metadata::{AuthorizationServerMetadata, ProtectedResourceMetadata};
pub use model::{
    AuthorizationCode, ClientRegistrationRequest, ClientRegistrationResponse, OAuthClient,
    RefreshToken, TokenEndpointAuthMethod, TokenResponse,
};
pub use pkce::{verify_pkce, CodeChallengeMethod};
pub use refresh::{
    exchange_refresh_token, hash_refresh_token, issue_refresh_token, IssuedRefreshToken,
    RefreshTokenRequest, REFRESH_TOKEN_TTL_SECONDS,
};
pub use registration::{hash_client_secret, register_client, RegistrationOutcome};
pub use scope::{grant_scopes, parse_scope, IdentityGrants};
pub use store::{AuthCodeStore, ClientStore, InMemoryAuthServerStore, RefreshTokenStore};
pub use token::{
    authenticate_client, exchange_authorization_code, AuthorizationCodeTokenRequest, IssuedGrant,
    TokenGrant,
};

/// The authorization server: a thin orchestrator over the stores and the
/// transport-agnostic protocol logic in the sibling modules.
///
/// It is generic over a single store type implementing all three storage traits
/// (client, auth-code, refresh-token), kept behind an `Arc` so it can be shared
/// across request tasks.
pub struct AuthorizationServer<S> {
    store: Arc<S>,
}

impl<S> Clone for AuthorizationServer<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
        }
    }
}

impl<S> AuthorizationServer<S>
where
    S: ClientStore + AuthCodeStore + RefreshTokenStore,
{
    /// Create a server backed by the given store.
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Borrow the underlying store (e.g. for sweeping expired codes).
    pub fn store(&self) -> &Arc<S> {
        &self.store
    }

    /// Build RFC 8414 authorization-server metadata for `issuer`.
    pub fn metadata(&self, issuer: &str) -> AuthorizationServerMetadata {
        AuthorizationServerMetadata::for_issuer(issuer)
    }

    /// Build RFC 9728 protected-resource metadata for `resource`, guarded by
    /// the authorization server at `issuer`.
    pub fn protected_resource_metadata(
        &self,
        resource: &str,
        issuer: &str,
    ) -> ProtectedResourceMetadata {
        ProtectedResourceMetadata::new(resource, issuer)
    }

    /// Register a new client (RFC 7591) and persist it.
    pub async fn register_client(
        &self,
        tenant_id: &str,
        req: ClientRegistrationRequest,
    ) -> AuthServerResult<ClientRegistrationResponse> {
        let now = chrono::Utc::now().timestamp();
        let outcome = register_client(tenant_id, req, now)?;
        self.store.put_client(outcome.client).await?;
        Ok(outcome.response)
    }

    /// Look up and validate the client named by an authorization request,
    /// returning the loaded client alongside the validated request so the HTTP
    /// layer can render a consent screen.
    pub async fn begin_authorization(
        &self,
        tenant_id: &str,
        req: &AuthorizationRequest,
    ) -> AuthServerResult<(OAuthClient, ValidatedAuthorizationRequest)> {
        let client = self
            .store
            .get_client(tenant_id, &req.client_id)
            .await?
            .ok_or_else(|| {
                AuthServerError::InvalidClient(format!("unknown client_id '{}'", req.client_id))
            })?;
        let validated = validate_authorization_request(req, &client)?;
        Ok((client, validated))
    }

    /// Issue and persist a single-use authorization code for a validated request
    /// and an authenticated resource owner. Returns the opaque code value.
    pub async fn complete_authorization(
        &self,
        tenant_id: &str,
        validated: &ValidatedAuthorizationRequest,
        owner: &ResourceOwner,
    ) -> AuthServerResult<AuthorizationCode> {
        let now = chrono::Utc::now().timestamp();
        let code = issue_authorization_code(validated, owner, tenant_id, now);
        self.store.put_code(code.clone()).await?;
        Ok(code)
    }

    /// Redeem an `authorization_code` token request: load + authenticate the
    /// client, atomically consume the code, and verify the grant + PKCE.
    ///
    /// Returns the [`TokenGrant`] the resource server signs into an access
    /// token, plus a fresh refresh token when the client registered for the
    /// `refresh_token` grant. This starts a new rotation family.
    pub async fn redeem_authorization_code(
        &self,
        tenant_id: &str,
        req: &AuthorizationCodeTokenRequest,
    ) -> AuthServerResult<IssuedGrant> {
        let client = self
            .store
            .get_client(tenant_id, &req.client_id)
            .await?
            .ok_or_else(|| {
                AuthServerError::InvalidClient(format!("unknown client_id '{}'", req.client_id))
            })?;
        authenticate_client(&client, &req.client_id, req.client_secret.as_deref())?;

        // Consume the code first so a concurrent replay cannot also redeem it.
        let code = self
            .store
            .take_code(tenant_id, &req.code)
            .await?
            .ok_or_else(|| {
                AuthServerError::InvalidGrant(
                    "authorization code is unknown, expired, or already used".to_string(),
                )
            })?;

        let now = chrono::Utc::now().timestamp();
        let grant = exchange_authorization_code(req, &code, now)?;
        let refresh_token = self
            .mint_refresh_token(&client, &grant, refresh::new_family_id(), now)
            .await?;

        Ok(IssuedGrant {
            grant,
            refresh_token,
        })
    }

    /// Redeem a `refresh_token` request, rotating the token.
    ///
    /// The presented token is consumed and a successor in the same family is
    /// issued. Presenting a token that was **already** consumed means the chain
    /// leaked, so the entire family is revoked — the legitimate client then has
    /// to re-authorize, which is the intended outcome of a detected replay
    /// (OAuth 2.1 §4.14.2).
    pub async fn redeem_refresh_token(
        &self,
        tenant_id: &str,
        req: &RefreshTokenRequest,
    ) -> AuthServerResult<IssuedGrant> {
        let client = self
            .store
            .get_client(tenant_id, &req.client_id)
            .await?
            .ok_or_else(|| {
                AuthServerError::InvalidClient(format!("unknown client_id '{}'", req.client_id))
            })?;
        authenticate_client(&client, &req.client_id, req.client_secret.as_deref())?;

        let now = chrono::Utc::now().timestamp();
        let hash = refresh::hash_refresh_token(&req.refresh_token);
        let record = self
            .store
            .consume_refresh_token(tenant_id, &hash, now)
            .await?
            .ok_or_else(|| {
                AuthServerError::InvalidGrant("refresh token is unknown or revoked".to_string())
            })?;

        // Replay: the token had already been redeemed. Burn the whole chain.
        if record.is_consumed() {
            let revoked = self
                .store
                .revoke_refresh_family(tenant_id, &record.family_id)
                .await?;
            tracing::warn!(
                tenant_id,
                client_id = %req.client_id,
                family_id = %record.family_id,
                revoked,
                "refresh token replay detected; rotation family revoked"
            );
            return Err(AuthServerError::InvalidGrant(
                "refresh token has already been used; the token family was revoked".to_string(),
            ));
        }

        let grant = match refresh::exchange_refresh_token(req, &record, &client, now) {
            Ok(grant) => grant,
            Err(err) => {
                // The token is already consumed at this point, so a failed
                // exchange must not leave a usable chain behind.
                let _ = self
                    .store
                    .revoke_refresh_family(tenant_id, &record.family_id)
                    .await;
                return Err(err);
            }
        };

        // Rotation stays within the original family so replay stays detectable
        // across the whole chain, not just the most recent hop.
        let refresh_token = self
            .mint_refresh_token(&client, &grant, record.family_id.clone(), now)
            .await?;

        Ok(IssuedGrant {
            grant,
            refresh_token,
        })
    }

    /// Issue and persist a refresh token, when the client is registered for the
    /// grant. Returns `None` for a client that is not.
    async fn mint_refresh_token(
        &self,
        client: &OAuthClient,
        grant: &TokenGrant,
        family_id: String,
        now: i64,
    ) -> AuthServerResult<Option<String>> {
        if !client.allows_grant_type("refresh_token") {
            return Ok(None);
        }
        let issued = refresh::issue_refresh_token(grant, family_id, now);
        self.store.put_refresh_token(issued.record).await?;
        Ok(Some(issued.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategies::OidcStrategy;

    fn server() -> AuthorizationServer<InMemoryAuthServerStore> {
        AuthorizationServer::new(Arc::new(InMemoryAuthServerStore::new()))
    }

    fn registration() -> ClientRegistrationRequest {
        ClientRegistrationRequest {
            redirect_uris: vec!["http://127.0.0.1:9000/cb".to_string()],
            scope: Some("reader".to_string()),
            ..Default::default()
        }
    }

    fn auth_request(client_id: &str, challenge: &str) -> AuthorizationRequest {
        AuthorizationRequest {
            response_type: "code".to_string(),
            client_id: client_id.to_string(),
            redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
            state: Some("st".to_string()),
            code_challenge: Some(challenge.to_string()),
            code_challenge_method: Some("S256".to_string()),
            scope: Some("reader".to_string()),
            resource: Some("https://db.example.com/mcp/repo/main/srv".to_string()),
        }
    }

    /// End-to-end: register → authorize → issue code → redeem with correct PKCE.
    #[tokio::test]
    async fn full_authorization_code_flow() {
        let server = server();
        let reg = server
            .register_client("tenant-a", registration())
            .await
            .unwrap();
        let client_id = reg.client_id;

        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);

        let (_client, validated) = server
            .begin_authorization("tenant-a", &auth_request(&client_id, &challenge))
            .await
            .unwrap();

        let owner = ResourceOwner {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            granted_scopes: vec!["reader".to_string()],
        };
        let code = server
            .complete_authorization("tenant-a", &validated, &owner)
            .await
            .unwrap();

        let token_req = AuthorizationCodeTokenRequest {
            grant_type: "authorization_code".to_string(),
            code: code.code.clone(),
            redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
            client_id: client_id.clone(),
            client_secret: None,
            code_verifier: Some(verifier),
        };
        let issued = server
            .redeem_authorization_code("tenant-a", &token_req)
            .await
            .unwrap();
        let grant = issued.grant;

        assert_eq!(grant.identity_id, "id-1");
        assert_eq!(grant.scope, "reader");
        assert_eq!(grant.audience, "https://db.example.com/mcp/repo/main/srv");
        assert!(
            issued.refresh_token.is_some(),
            "the default registration includes the refresh_token grant"
        );

        // The code is single-use: a replay must fail.
        let replay = server
            .redeem_authorization_code("tenant-a", &token_req)
            .await
            .expect_err("code replay must be rejected");
        assert_eq!(replay.code(), "invalid_grant");
    }

    #[tokio::test]
    async fn redeem_with_wrong_verifier_is_rejected() {
        let server = server();
        let reg = server
            .register_client("tenant-a", registration())
            .await
            .unwrap();
        let client_id = reg.client_id;

        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let (_c, validated) = server
            .begin_authorization("tenant-a", &auth_request(&client_id, &challenge))
            .await
            .unwrap();
        let owner = ResourceOwner {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            granted_scopes: vec!["reader".to_string()],
        };
        let code = server
            .complete_authorization("tenant-a", &validated, &owner)
            .await
            .unwrap();

        let token_req = AuthorizationCodeTokenRequest {
            grant_type: "authorization_code".to_string(),
            code: code.code,
            redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
            client_id,
            client_secret: None,
            code_verifier: Some(OidcStrategy::generate_code_verifier()),
        };
        let err = server
            .redeem_authorization_code("tenant-a", &token_req)
            .await
            .expect_err("wrong verifier must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    /// Drive register → authorize → redeem and return `(client_id, refresh_token)`.
    async fn flow_to_refresh_token(
        server: &AuthorizationServer<InMemoryAuthServerStore>,
    ) -> (String, String) {
        let reg = server
            .register_client("tenant-a", registration())
            .await
            .unwrap();
        let client_id = reg.client_id;

        let verifier = OidcStrategy::generate_code_verifier();
        let challenge = OidcStrategy::generate_code_challenge(&verifier);
        let (_c, validated) = server
            .begin_authorization("tenant-a", &auth_request(&client_id, &challenge))
            .await
            .unwrap();
        let owner = ResourceOwner {
            identity_id: "id-1".to_string(),
            email: "u@example.com".to_string(),
            repository: "repo".to_string(),
            branch: "main".to_string(),
            granted_scopes: vec!["reader".to_string()],
        };
        let code = server
            .complete_authorization("tenant-a", &validated, &owner)
            .await
            .unwrap();

        let issued = server
            .redeem_authorization_code(
                "tenant-a",
                &AuthorizationCodeTokenRequest {
                    grant_type: "authorization_code".to_string(),
                    code: code.code,
                    redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
                    client_id: client_id.clone(),
                    client_secret: None,
                    code_verifier: Some(verifier),
                },
            )
            .await
            .unwrap();

        (client_id, issued.refresh_token.expect("refresh issued"))
    }

    fn refresh_request(client_id: &str, token: &str) -> RefreshTokenRequest {
        RefreshTokenRequest {
            grant_type: "refresh_token".to_string(),
            refresh_token: token.to_string(),
            client_id: client_id.to_string(),
            client_secret: None,
            scope: None,
        }
    }

    /// The whole point of the grant: refreshing yields a new access token
    /// without another trip through `/authorize`, and rotates the refresh token.
    #[tokio::test]
    async fn refresh_rotates_and_preserves_the_grant() {
        let server = server();
        let (client_id, refresh) = flow_to_refresh_token(&server).await;

        let issued = server
            .redeem_refresh_token("tenant-a", &refresh_request(&client_id, &refresh))
            .await
            .unwrap();

        assert_eq!(issued.grant.identity_id, "id-1");
        assert_eq!(
            issued.grant.audience,
            "https://db.example.com/mcp/repo/main/srv"
        );
        assert_eq!(issued.grant.scope, "reader");

        let rotated = issued.refresh_token.expect("rotation issues a successor");
        assert_ne!(rotated, refresh, "the token must not be reused verbatim");

        // The successor works in turn — a connector can refresh indefinitely.
        server
            .redeem_refresh_token("tenant-a", &refresh_request(&client_id, &rotated))
            .await
            .expect("the rotated token is usable");
    }

    /// Replaying a consumed token burns the family, including the successor the
    /// legitimate client is holding.
    #[tokio::test]
    async fn refresh_replay_revokes_the_whole_family() {
        let server = server();
        let (client_id, refresh) = flow_to_refresh_token(&server).await;

        let rotated = server
            .redeem_refresh_token("tenant-a", &refresh_request(&client_id, &refresh))
            .await
            .unwrap()
            .refresh_token
            .unwrap();

        // Replay the original.
        let err = server
            .redeem_refresh_token("tenant-a", &refresh_request(&client_id, &refresh))
            .await
            .expect_err("a consumed token must be rejected");
        assert_eq!(err.code(), "invalid_grant");

        // The successor is collateral damage, by design.
        let err = server
            .redeem_refresh_token("tenant-a", &refresh_request(&client_id, &rotated))
            .await
            .expect_err("the family was revoked");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[tokio::test]
    async fn unknown_refresh_token_is_rejected() {
        let server = server();
        let (client_id, _) = flow_to_refresh_token(&server).await;

        let err = server
            .redeem_refresh_token("tenant-a", &refresh_request(&client_id, "rt_nonsense"))
            .await
            .expect_err("unknown token must fail");
        assert_eq!(err.code(), "invalid_grant");
    }

    #[tokio::test]
    async fn unknown_client_is_rejected() {
        let server = server();
        let req = AuthorizationCodeTokenRequest {
            grant_type: "authorization_code".to_string(),
            code: "x".to_string(),
            redirect_uri: Some("http://127.0.0.1:9000/cb".to_string()),
            client_id: "nope".to_string(),
            client_secret: None,
            code_verifier: Some(OidcStrategy::generate_code_verifier()),
        };
        let err = server
            .redeem_authorization_code("tenant-a", &req)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid_client");
    }
}
