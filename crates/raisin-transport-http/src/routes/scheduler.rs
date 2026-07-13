// SPDX-License-Identifier: BSL-1.1

//! Routes for one-shot scheduled invocations (time-based function/flow runs).

use axum::routing::{get, post};
use axum::Router;

#[cfg(feature = "storage-rocksdb")]
use crate::middleware::optional_auth_middleware;
use crate::state::AppState;

/// Build routes for the repo-scoped scheduled invocation API.
#[allow(unused_variables)]
pub(crate) fn scheduler_routes(state: &AppState) -> Router<AppState> {
    let create_and_list = post(crate::handlers::scheduler::create_invocation)
        .get(crate::handlers::scheduler::list_invocations);
    let get_and_cancel = get(crate::handlers::scheduler::get_invocation)
        .delete(crate::handlers::scheduler::cancel_invocation);

    #[cfg(feature = "storage-rocksdb")]
    let (create_and_list, get_and_cancel) = (
        create_and_list.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            optional_auth_middleware,
        )),
        get_and_cancel.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            optional_auth_middleware,
        )),
    );

    Router::new()
        // Schedule / list one-shot invocations
        .route("/api/scheduler/{repo}/invocations", create_and_list)
        // Inspect / cancel a single invocation
        .route("/api/scheduler/{repo}/invocations/{job_id}", get_and_cancel)
}
