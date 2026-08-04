// SPDX-License-Identifier: BSL-1.1

//! Routes for OUTBOUND MCP connections (RaisinDB as an MCP client).
//!
//! Namespaced `/api/mcp-connections/...` and entirely separate from
//! [`crate::routes::mcp`], which serves RaisinDB's own tools to external
//! clients, and from [`crate::routes::oauth`], the inbound authorization server.
//!
//! Every endpoint here is admin-gated: creating a connection tells the server to
//! dial an operator-chosen URL, and attaching its tools to an agent hands that
//! agent an external side effect. The public OAuth callback that the
//! service-account flow needs is added in the OAuth phase; until then there is
//! deliberately no unauthenticated route in this group.

use axum::Router;

use crate::state::AppState;

/// Build the outbound MCP-connection route group.
///
/// Empty without `storage-rocksdb`: connection config and (later) the discovery
/// job both require that backend, matching `integration_routes`.
pub(crate) fn mcp_client_routes(state: &AppState) -> Router<AppState> {
    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::handlers::mcp_client;
        use crate::middleware::require_auth_middleware;
        use axum::middleware::from_fn_with_state;
        use axum::routing::{delete, get, post, put};

        // PUBLIC: a browser redirect from the authorization server cannot carry
        // a bearer token. Authenticated instead by the single-use, TTL'd
        // `state` the authenticated `oauth/start` call minted.
        let public = Router::new().route(
            "/api/mcp-connections/{repo}/oauth/callback",
            get(mcp_client::callback),
        );

        let guarded = Router::new()
            .route(
                "/api/mcp-connections/{repo}",
                get(mcp_client::list_connections).post(mcp_client::create_connection),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}",
                get(mcp_client::get_connection)
                    .patch(mcp_client::update_connection)
                    .delete(mcp_client::delete_connection),
            )
            // Write-only: there is no GET here by design.
            .route(
                "/api/mcp-connections/{repo}/{slug}/credential",
                put(mcp_client::set_credential).delete(mcp_client::clear_credential),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/test",
                post(mcp_client::test_connection),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/refresh-tools",
                post(mcp_client::refresh_tools),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/tools",
                get(mcp_client::list_tools),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/tools/{remote_name}",
                axum::routing::patch(mcp_client::set_tool_enabled),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/oauth/discover",
                post(mcp_client::discover),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/oauth/start",
                post(mcp_client::start),
            )
            .route(
                "/api/mcp-connections/{repo}/{slug}/oauth/disconnect",
                post(mcp_client::disconnect),
            )
            .layer(from_fn_with_state(state.clone(), require_auth_middleware));

        public.merge(guarded)
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = state;
        Router::new()
    }
}
