// SPDX-License-Identifier: BSL-1.1

//! Routes for the node-development surface (see `handlers::node_dev`).

use axum::routing::{get, post};
use axum::Router;

use crate::middleware::optional_auth_middleware;
use crate::state::AppState;

/// Build the `/api/node-dev/*` routes. Every call requires an authenticated
/// caller; `optional_auth_middleware` resolves it against the path's repo.
pub(crate) fn node_dev_routes(state: &AppState) -> Router<AppState> {
    use crate::handlers::node_dev as h;
    Router::new()
        .route("/api/node-dev/{repo}", get(h::methods))
        .route("/api/node-dev/{repo}/{method}", post(h::call))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            optional_auth_middleware,
        ))
}
