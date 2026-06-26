// SPDX-License-Identifier: BSL-1.1

//! MCP (Model Context Protocol) Streamable HTTP transport.
//!
//! Exposes content-declared `raisin:McpServer` endpoints over the MCP Streamable
//! HTTP binding: a single `POST /mcp/{repo}/{branch}/{slug}` carrying one
//! JSON-RPC 2.0 message in its body.
//!
//! The handler authenticates the request through the existing auth middleware
//! (the resolved [`AuthContext`] arrives in the request extensions), projects it
//! onto an [`McpIdentity`], assembles the server's live tool set from the
//! `raisin:McpServer` node, and dispatches the JSON-RPC method against the real
//! RaisinDB services held in [`AppState`]:
//!
//! - node read / write / query / SQL via a branch-scoped `RaisinFunctionApi`,
//! - custom function tools via the real [`FunctionExecutor`](raisin_functions::FunctionExecutor)
//!   pipeline ([`services::HttpFunctionInvoker`]),
//! - `search_nodes` via the Tantivy / HNSW engines ([`services::HttpSearchProvider`]),
//! - `resources/subscribe` via the in-process event bus ([`services::BusEventSource`]),
//!   streamed back as Server-Sent Events.
//!
//! A JSON-RPC notification (a message with no `id`) receives an empty `202`
//! response, per the JSON-RPC and MCP specs.

mod api_factory;
mod auth;
mod identity;
mod services;

#[cfg(feature = "storage-rocksdb")]
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "storage-rocksdb")]
use axum::http::HeaderMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Extension, Json,
};
use futures::StreamExt;
use raisin_mcp::{JsonRpcRequest, JsonRpcResponse, McpError, McpIdentity};
use raisin_models::auth::AuthContext;
use serde_json::Value;

use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Workspace that holds the `raisin:McpServer` declaration nodes.
///
/// The `{slug}` path segment resolves against this workspace; the server's own
/// `dataPolicy.workspaces` then govern which content workspaces its tools touch.
const MCP_DISCOVERY_WORKSPACE: &str = "mcp";

/// Handle one MCP Streamable HTTP request: `POST /mcp/{repo}/{branch}/{slug}`.
///
/// The body is a single JSON-RPC 2.0 message. Requests (with an `id`) return a
/// JSON-RPC response; `resources/subscribe` returns an SSE stream of
/// `notifications/resources/updated`; notifications (no `id`) return `202`.
#[cfg(feature = "storage-rocksdb")]
pub async fn handle_mcp(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, branch, slug)): Path<(String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    let tenant_id = tenant_info.tenant_id.clone();
    let ext_auth = auth.map(|Extension(ctx)| ctx);

    // The shared middleware rejects audience-bound OAuth tokens; resolve such a
    // token here against this endpoint's audience (or keep the middleware's
    // result for login tokens / API keys / anonymous).
    let (auth_context, consented_scopes) =
        auth::resolve_mcp_auth(&state, &headers, &repo, &branch, &slug, ext_auth).await;

    match dispatch(
        &state,
        &tenant_id,
        &repo,
        &branch,
        &slug,
        auth_context.as_ref(),
        consented_scopes.as_deref(),
        request,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => jsonrpc_error_response(Value::Null, &err),
    }
}

/// Stub when the RocksDB backend (and thus the function / search services) is
/// not compiled in.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_mcp(
    State(_state): State<AppState>,
    Path((_repo, _branch, _slug)): Path<(String, String, String)>,
    Json(_request): Json<Value>,
) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "MCP transport requires the RocksDB backend",
    )
        .into_response()
}

/// Assemble the server, build the dispatcher, and route the JSON-RPC method.
#[cfg(feature = "storage-rocksdb")]
#[allow(clippy::too_many_arguments)]
async fn dispatch(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    slug: &str,
    auth: Option<&AuthContext>,
    consented_scopes: Option<&[String]>,
    request: JsonRpcRequest,
) -> Result<Response, McpError> {
    use raisin_mcp::{
        assemble_for_slug, AssemblyServices, Dispatcher, NodeResourceProvider,
    };

    use api_factory::build_mcp_function_api;
    use identity::mcp_identity_from_auth;
    use services::{BusEventSource, HttpFunctionInvoker, HttpSearchProvider};

    // Caller-scoped backend: data tools read/write as the authenticated identity
    // (or anonymous), so RLS governs what their calls can touch.
    let backend = build_mcp_function_api(
        state,
        tenant_id,
        repo,
        branch,
        MCP_DISCOVERY_WORKSPACE,
        Default::default(),
        auth.cloned(),
    );

    // Discovery backend: resolves the `raisin:McpServer` declaration (and any
    // function-side tool declarations) with a system context. Reading a server's
    // own declaration is routing metadata, not user data — without this a
    // `public` server could not be resolved by an anonymous caller who lacks RLS
    // read on the discovery workspace. Access is still gated post-resolution by
    // `Dispatcher::authorize_session` (a non-public server rejects anonymous).
    let discovery_backend: Arc<dyn raisin_functions::FunctionApi> = build_mcp_function_api(
        state,
        tenant_id,
        repo,
        branch,
        MCP_DISCOVERY_WORKSPACE,
        Default::default(),
        Some(AuthContext::system()),
    );

    let invoker: Arc<dyn raisin_mcp::FunctionInvoker> = Arc::new(HttpFunctionInvoker::new(
        state.clone(),
        tenant_id,
        repo,
        branch,
    ));
    let search: Arc<dyn raisin_mcp::SearchProvider> =
        Arc::new(HttpSearchProvider::new(state.clone(), tenant_id, repo));

    let services = AssemblyServices {
        backend: backend.clone(),
        search: Some(search),
        functions: Some(invoker),
    };

    let (descriptor, registry) =
        assemble_for_slug(&services, &discovery_backend, MCP_DISCOVERY_WORKSPACE, slug).await?;

    // The session's active workspace defaults to the server's first declared
    // content workspace, falling back to the discovery workspace.
    let active_workspace = descriptor
        .data_policy
        .workspaces
        .first()
        .cloned()
        .unwrap_or_else(|| MCP_DISCOVERY_WORKSPACE.to_string());

    let identity = mcp_identity_from_auth(
        auth,
        tenant_id,
        repo,
        branch,
        &active_workspace,
        consented_scopes,
    );

    // Wire resource reads + live subscriptions onto the event bus.
    use raisin_storage::Storage;
    let events: Arc<dyn raisin_mcp::EventSource> = Arc::new(BusEventSource::new(
        state.storage().event_bus(),
        tenant_id,
        repo,
    ));
    let resources = NodeResourceProvider::new(backend).with_events(events);

    let dispatcher = Dispatcher::new(descriptor, registry).with_resources(resources);

    // `resources/subscribe` upgrades to an SSE stream of update notifications.
    if request.method == "resources/subscribe" && request.id.is_some() {
        return subscribe_sse(dispatcher, identity, request);
    }

    let is_notification = request.id.is_none();
    let id = request.id.clone().unwrap_or(Value::Null);

    match dispatcher.handle(&identity, &request).await {
        Ok(result) if is_notification => {
            // Notifications expect no JSON-RPC reply.
            let _ = result;
            Ok(StatusCode::ACCEPTED.into_response())
        }
        Ok(result) => Ok(Json(JsonRpcResponse::success(id, result)).into_response()),
        Err(_) if is_notification => Ok(StatusCode::ACCEPTED.into_response()),
        Err(err) => Ok(jsonrpc_error_response(id, &err)),
    }
}

/// Open an SSE stream that confirms the subscription, then forwards every
/// `notifications/resources/updated` frame for the subscribed URI.
#[cfg(feature = "storage-rocksdb")]
fn subscribe_sse(
    dispatcher: raisin_mcp::Dispatcher,
    identity: McpIdentity,
    request: JsonRpcRequest,
) -> Result<Response, McpError> {
    use raisin_mcp::SubscribeResourceParams;

    let id = request.id.clone().unwrap_or(Value::Null);
    let params: SubscribeResourceParams = request.decode_params()?;
    let uri = params.uri.clone();

    // Confirm the subscription up front, then stream updates.
    let stream = dispatcher.subscribe_resource(&identity, &uri)?;

    let ack = JsonRpcResponse::success(id, serde_json::json!({ "subscribed": true, "uri": uri }));

    let sse_stream = async_stream::stream! {
        // First frame: the JSON-RPC subscribe acknowledgement.
        if let Ok(data) = serde_json::to_string(&ack) {
            yield Ok::<SseEvent, std::convert::Infallible>(
                SseEvent::default().event("message").data(data),
            );
        }

        futures::pin_mut!(stream);
        while let Some(notification) = stream.next().await {
            let frame = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/resources/updated",
                "params": notification,
            });
            if let Ok(data) = serde_json::to_string(&frame) {
                yield Ok(SseEvent::default().event("message").data(data));
            }
        }
    };

    Ok(Sse::new(sse_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response())
}

/// Build a JSON-RPC error response.
///
/// Errors are carried in the JSON-RPC envelope (HTTP `200`), so a client always
/// receives a parseable JSON-RPC body whether the call succeeded or failed.
fn jsonrpc_error_response(id: Value, err: &McpError) -> Response {
    Json(JsonRpcResponse::failure(id, err)).into_response()
}
