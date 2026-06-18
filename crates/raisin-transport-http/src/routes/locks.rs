// SPDX-License-Identifier: BSL-1.1

//! Routes for the atomic lock / inventory subsystem.

use axum::routing::post;
use axum::Router;

use crate::state::AppState;

/// Build routes for lease-locks (`/locks/*`) and counting reservations
/// (`/inventory/*`).
///
/// `optional_auth_middleware` is layered on so the `AuthContext` extension is
/// populated for authenticated callers; the handlers ACL-gate on it (lock /
/// inventory operations reject anonymous/unauthenticated requests).
pub(crate) fn locks_routes(state: &AppState) -> Router<AppState> {
    #[allow(unused_mut)]
    let mut router = Router::new()
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
        );

    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::middleware::optional_auth_middleware;
        use axum::middleware::from_fn_with_state;

        router = router.layer(from_fn_with_state(state.clone(), optional_auth_middleware));
    }

    let _ = state;
    router
}
