// SPDX-License-Identifier: BSL-1.1

//! Routes for the OAuth 2.1 authorization server.
//!
//! Exposes the discovery, registration, authorization, and token endpoints that
//! let an interactive MCP client authenticate against RaisinDB:
//!
//! - `GET  /.well-known/oauth-authorization-server` (RFC 8414)
//! - `GET  /.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}` (RFC 9728)
//! - `POST /register` (RFC 7591 Dynamic Client Registration)
//! - `GET  /authorize` / `POST /authorize` (PKCE authorization-code grant)
//! - `POST /token` (authorization-code exchange)
//!
//! All endpoints depend on the `storage-rocksdb` feature, which brings in the
//! authorization server, identity store, and JWT signer. They go through the
//! global tenant middleware (applied in [`crate::routes::routes`]) so the
//! `TenantInfo` extension the handlers read is populated.

use axum::Router;

use crate::state::AppState;

/// Build the OAuth 2.1 authorization-server route group.
///
/// When the `storage-rocksdb` feature is disabled the server cannot mint tokens,
/// so the group is empty.
pub(crate) fn oauth_routes(state: &AppState) -> Router<AppState> {
    let _ = state;

    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::handlers::oauth_as;
        use axum::routing::{get, post};

        Router::new()
            .route(
                "/.well-known/oauth-authorization-server",
                get(oauth_as::metadata::authorization_server_metadata),
            )
            .route(
                "/.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}",
                get(oauth_as::metadata::protected_resource_metadata),
            )
            .route("/register", post(oauth_as::register::register_client))
            .route(
                "/authorize",
                get(oauth_as::authorize::authorize_get).post(oauth_as::authorize::authorize_post),
            )
            .route("/token", post(oauth_as::token::token))
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        Router::new()
    }
}
