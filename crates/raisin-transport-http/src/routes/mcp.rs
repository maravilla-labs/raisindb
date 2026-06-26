// SPDX-License-Identifier: BSL-1.1

//! Routes for the MCP (Model Context Protocol) Streamable HTTP transport.
//!
//! A single `POST /mcp/{repo}/{branch}/{slug}` endpoint serves the JSON-RPC
//! message exchange for a content-declared `raisin:McpServer`. Optional auth is
//! attached so public servers/tools work anonymously while authenticated callers
//! get their resolved permissions; repo resolution for that auth depends on the
//! `mcp` arm added to `middleware::path_helpers::extract_repo_from_path`.

use axum::routing::post;
use axum::Router;

#[cfg(feature = "storage-rocksdb")]
use crate::middleware::optional_auth_middleware;
use crate::state::AppState;

/// Build the MCP transport route group.
pub(crate) fn mcp_routes(state: &AppState) -> Router<AppState> {
    let router = Router::new().route(
        "/mcp/{repo}/{branch}/{slug}",
        post(crate::handlers::mcp::handle_mcp),
    );

    #[cfg(feature = "storage-rocksdb")]
    let router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        optional_auth_middleware,
    ));

    // `state` only feeds the auth layer above, which is RocksDB-gated.
    #[cfg(not(feature = "storage-rocksdb"))]
    let _ = state;

    router
}
