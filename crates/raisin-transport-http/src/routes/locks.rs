// SPDX-License-Identifier: BSL-1.1

//! Routes for the atomic lock / inventory subsystem.

use axum::routing::post;
use axum::Router;

use crate::state::AppState;

/// Build routes for lease-locks (`/locks/*`) and counting reservations
/// (`/inventory/*`).
pub(crate) fn locks_routes(_state: &AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/{repo}/{branch}/locks/acquire",
            post(crate::handlers::locks::acquire_lock),
        )
        .route(
            "/api/{repo}/{branch}/locks/release",
            post(crate::handlers::locks::release_lock),
        )
        .route(
            "/api/{repo}/{branch}/locks/renew",
            post(crate::handlers::locks::renew_lock),
        )
        .route(
            "/api/{repo}/{branch}/inventory/claim",
            post(crate::handlers::locks::claim_inventory),
        )
        .route(
            "/api/{repo}/{branch}/inventory/release",
            post(crate::handlers::locks::release_inventory),
        )
}
