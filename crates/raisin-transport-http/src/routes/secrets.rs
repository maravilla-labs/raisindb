// SPDX-License-Identifier: BSL-1.1

//! Routes for the secret store (`/api/secrets/{repo}/{branch}`).
//!
//! | Method   | Path                                          | Behaviour                                   |
//! |----------|-----------------------------------------------|---------------------------------------------|
//! | `PUT`    | `/api/secrets/{repo}/{branch}/{*name}`        | Create or append a version                  |
//! | `GET`    | `/api/secrets/{repo}/{branch}`                | List metadata (newest version of each)      |
//! | `GET`    | `/api/secrets/{repo}/{branch}/{*name}`        | Metadata for one secret, every version      |
//! | `DELETE` | `/api/secrets/{repo}/{branch}/{*name}`        | Tombstone                                   |
//! | `POST`   | `/api/secrets/{repo}/{branch}/rotate/{*name}` | New version, stamped `rotated_at`           |
//!
//! **There is no route that returns a plaintext value**, by design — see the
//! handler module docs.
//!
//! `{*name}` is a wildcard because a secret name may contain `/`
//! (`node/{node_id}/{field.path}`), and an axum wildcard must be the last
//! segment — which is why rotate reads `rotate/{*name}` rather than
//! `{name}/rotate`.
//!
//! `optional_auth_middleware` is layered on so the `AuthContext` extension is
//! populated; every handler then gates on it with `require_admin`.

use axum::routing::{get, post, put};
use axum::Router;

use crate::state::AppState;

pub(crate) fn secrets_routes(state: &AppState) -> Router<AppState> {
    #[allow(unused_mut)]
    let mut router = Router::new()
        .route(
            "/api/secrets/{repo}/{branch}",
            get(crate::handlers::secrets::list_secrets),
        )
        .route(
            "/api/secrets/{repo}/{branch}/rotate/{*name}",
            post(crate::handlers::secrets::rotate_secret),
        )
        // One `.route()` carrying all three methods: registering the same path
        // twice panics at router-build time in axum 0.8.
        .route(
            "/api/secrets/{repo}/{branch}/{*name}",
            put(crate::handlers::secrets::put_secret)
                .get(crate::handlers::secrets::get_secret_metadata)
                .delete(crate::handlers::secrets::delete_secret),
        );

    {
        use crate::middleware::optional_auth_middleware;
        use axum::middleware::from_fn_with_state;

        router = router.layer(from_fn_with_state(state.clone(), optional_auth_middleware));
    }

    let _ = state;
    router
}
