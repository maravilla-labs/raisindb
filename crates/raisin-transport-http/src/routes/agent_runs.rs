// SPDX-License-Identifier: BSL-1.1

//! Routes for durable agent runs (see `handlers::agent_runs`).

use axum::routing::{get, post};
use axum::Router;

use crate::middleware::optional_auth_middleware;
use crate::state::AppState;

/// Build the `/api/agent-runs/*` routes. Every handler requires an
/// authenticated caller; `optional_auth_middleware` populates it.
pub(crate) fn agent_run_routes(state: &AppState) -> Router<AppState> {
    use crate::handlers::agent_runs as h;
    use crate::handlers::agent_runs_children as c;
    use crate::handlers::agent_runs_stream as s;
    Router::new()
        .route(
            "/api/agent-runs/{repo}",
            post(h::create_run).get(h::list_runs),
        )
        .route("/api/agent-runs/{repo}/{run}", get(h::get_run))
        .route("/api/agent-runs/{repo}/{run}/events", get(h::run_events))
        .route("/api/agent-runs/{repo}/{run}/stream", get(s::stream_run))
        .route("/api/agent-runs/{repo}/{run}/control", post(h::control_run))
        .route(
            "/api/agent-runs/{repo}/{run}/operations",
            post(h::begin_operation),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/operations/finish",
            post(h::finish_operation),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/lease/{action}",
            post(h::run_lease),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/children",
            post(c::spawn_child).get(c::list_children),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/children/{child}",
            get(c::inspect_child),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/children/{child}/{action}",
            post(c::child_action),
        )
        .route("/api/agent-runs/{repo}/{run}/mailbox", get(c::mailbox))
        .route(
            "/api/agent-runs/{repo}/{run}/mailbox/ack",
            post(c::ack_mailbox),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/post-to-parent",
            post(c::post_to_parent),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/checkpoints",
            post(c::write_checkpoint),
        )
        .route(
            "/api/agent-runs/{repo}/{run}/checkpoints/{n}",
            get(c::read_checkpoint),
        )
        .route("/api/agent-runs/{repo}/{run}/usage", get(c::usage))
        .route("/api/agent-runs/{repo}/{run}/{action}", post(h::run_action))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            optional_auth_middleware,
        ))
}
