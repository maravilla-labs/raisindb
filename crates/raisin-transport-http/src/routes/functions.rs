// SPDX-License-Identifier: BSL-1.1

//! Routes for serverless functions, flows, webhooks, and HTTP triggers.

use axum::routing::{delete, get, post};
use axum::Router;

#[cfg(feature = "storage-rocksdb")]
use crate::middleware::optional_auth_middleware;
use crate::state::AppState;

/// Build routes for serverless functions, flows, webhooks, and HTTP triggers.
pub(crate) fn function_routes(state: &AppState) -> Router<AppState> {
    Router::new()
        // ----------------------------------------------------------------
        // Serverless Functions API
        // ----------------------------------------------------------------
        // List all functions in a repository
        .route(
            "/api/functions/{repo}",
            get(crate::handlers::functions::list_functions),
        )
        // Get function details
        .route(
            "/api/functions/{repo}/{name}",
            get(crate::handlers::functions::get_function),
        )
        // Invoke a function (sync or async)
        .route(
            "/api/functions/{repo}/{name}/invoke",
            post(crate::handlers::functions::invoke_function),
        )
        // List function executions
        .route(
            "/api/functions/{repo}/{name}/executions",
            get(crate::handlers::functions::list_executions),
        )
        // Get specific execution details
        .route(
            "/api/functions/{repo}/{name}/executions/{execution_id}",
            get(crate::handlers::functions::get_execution),
        )
        // Direct file execution (standalone JS files without parent Function)
        .route(
            "/api/files/{repo}/run",
            post(crate::handlers::functions::run_file).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                optional_auth_middleware,
            )),
        )
        // ----------------------------------------------------------------
        // Flow execution
        // ----------------------------------------------------------------
        // Execute a raisin:Flow by path
        .route(
            "/api/flows/{repo}/run",
            post(crate::handlers::functions::run_flow).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                optional_auth_middleware,
            )),
        )
        // Test flow execution (with mock configuration)
        .route(
            "/api/flows/{repo}/test",
            post(crate::handlers::functions::run_flow_test).layer(
                axum::middleware::from_fn_with_state(state.clone(), optional_auth_middleware),
            ),
        )
        // Get flow instance status / Delete a flow instance
        .route(
            "/api/flows/{repo}/instances/{instance_id}",
            get(crate::handlers::functions::get_flow_instance)
                .delete(crate::handlers::functions::delete_flow_instance)
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    optional_auth_middleware,
                )),
        )
        // Resume a paused flow instance
        .route(
            "/api/flows/{repo}/instances/{instance_id}/resume",
            post(crate::handlers::functions::resume_flow).layer(
                axum::middleware::from_fn_with_state(state.clone(), optional_auth_middleware),
            ),
        )
        // Cancel a running/waiting flow instance
        .route(
            "/api/flows/{repo}/instances/{instance_id}/cancel",
            post(crate::handlers::functions::cancel_flow_instance).layer(
                axum::middleware::from_fn_with_state(state.clone(), optional_auth_middleware),
            ),
        )
        // Flow instance events SSE (real-time step-level events)
        .route(
            "/api/flows/{repo}/instances/{instance_id}/events",
            get(crate::handlers::functions::stream_flow_events).layer(
                axum::middleware::from_fn_with_state(state.clone(), optional_auth_middleware),
            ),
        )
        // ----------------------------------------------------------------
        // Conversation events SSE (real-time AI conversation streaming)
        // ----------------------------------------------------------------
        .route(
            "/api/conversations/{repo}/events",
            get(crate::handlers::conversations::stream_conversation_events)
                .post(crate::handlers::conversations::stream_conversation_events_post)
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    optional_auth_middleware,
                )),
        )
        // ----------------------------------------------------------------
        // HTTP Webhooks (nanoid-based secure URLs)
        // ----------------------------------------------------------------
        .route(
            "/api/webhooks/{repo}/{webhook_id}",
            get(crate::handlers::webhooks::invoke_webhook)
                .post(crate::handlers::webhooks::invoke_webhook)
                .put(crate::handlers::webhooks::invoke_webhook)
                .delete(crate::handlers::webhooks::invoke_webhook),
        )
        .route(
            "/api/webhooks/{repo}/{webhook_id}/{*path_suffix}",
            get(crate::handlers::webhooks::invoke_webhook_with_path)
                .post(crate::handlers::webhooks::invoke_webhook_with_path)
                .put(crate::handlers::webhooks::invoke_webhook_with_path)
                .delete(crate::handlers::webhooks::invoke_webhook_with_path),
        )
        // ----------------------------------------------------------------
        // HTTP Triggers (name-based URLs with unique trigger names)
        // ----------------------------------------------------------------
        .route(
            "/api/triggers/{repo}/{trigger_name}",
            get(crate::handlers::webhooks::invoke_trigger)
                .post(crate::handlers::webhooks::invoke_trigger)
                .put(crate::handlers::webhooks::invoke_trigger)
                .delete(crate::handlers::webhooks::invoke_trigger),
        )
        .route(
            "/api/triggers/{repo}/{trigger_name}/{*path_suffix}",
            get(crate::handlers::webhooks::invoke_trigger_with_path)
                .post(crate::handlers::webhooks::invoke_trigger_with_path)
                .put(crate::handlers::webhooks::invoke_trigger_with_path)
                .delete(crate::handlers::webhooks::invoke_trigger_with_path),
        )
}
