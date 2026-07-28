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
//!
//! Errors ride in the JSON-RPC envelope on HTTP `200`, **except** authorization
//! failures: an unauthenticated caller gets `401` with an RFC 9728
//! `WWW-Authenticate: Bearer resource_metadata="…"` challenge so the client can
//! discover the authorization server and run the OAuth flow, and an
//! authenticated caller missing a scope gets `403`.

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
    // Taken as raw bytes rather than `Json<JsonRpcRequest>` on purpose: the
    // `Json` extractor rejects a request whose `Content-Type` is missing or not
    // `application/json` with a bare `415`, before this handler ever runs. MCP
    // clients probe the endpoint with exactly such a request, and a `415`
    // carries no `WWW-Authenticate`, so the probe teaches them nothing. Parsing
    // here lets an unauthenticated probe get the OAuth challenge it needs.
    body: axum::body::Bytes,
) -> Response {
    let tenant_id = tenant_info.tenant_id.clone();
    let ext_auth = auth.map(|Extension(ctx)| ctx);

    // The shared middleware rejects audience-bound OAuth tokens; resolve such a
    // token here against this endpoint's audience (or keep the middleware's
    // result for login tokens / API keys / anonymous).
    let (auth_context, consented_scopes) = auth::resolve_mcp_auth(
        &state, &headers, &tenant_id, &repo, &branch, &slug, ext_auth,
    )
    .await;

    // An unauthenticated caller gets an RFC 9728 challenge pointing at this
    // endpoint's protected-resource metadata, so the client can find the
    // authorization server and start (or, when the 1h token lapses, restart) the
    // OAuth flow instead of surfacing a generic failure.
    let authenticated = is_authenticated(auth_context.as_ref());
    let metadata_url = if authenticated {
        None
    } else {
        auth::resource_metadata_challenge(&state, &headers, &tenant_id, &repo, &branch, &slug).await
    };
    let challenge = AuthChallenge {
        authenticated,
        metadata_url: metadata_url.as_deref(),
    };

    // An unparseable body from an unauthenticated caller is treated as a probe:
    // answer with the challenge, not a parse error, so the client discovers the
    // authorization server. Authenticated callers get a real JSON-RPC parse
    // error, which is the actionable answer for them.
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            tracing::debug!(
                %repo, %branch, %slug, authenticated,
                error = %e,
                "MCP request body is not a JSON-RPC message"
            );
            let err = if authenticated {
                McpError::Parse(format!("invalid JSON-RPC request body: {e}"))
            } else {
                McpError::unauthorized("authentication required for this MCP server")
            };
            return jsonrpc_error_response(Value::Null, &err, &challenge);
        }
    };

    // Externally-reachable base for `mode: uri-list` widget URLs: explicit
    // RAISINDB_BASE_URL wins (same override the signed-URL builder honors),
    // else derive from the request's forwarded/Host headers.
    let public_base = std::env::var("RAISINDB_BASE_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            let host = headers
                .get("x-forwarded-host")
                .or_else(|| headers.get(axum::http::header::HOST))
                .and_then(|v| v.to_str().ok())?;
            let proto = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("http");
            Some(format!("{proto}://{host}"))
        });

    match dispatch(
        &state,
        &tenant_id,
        &repo,
        &branch,
        &slug,
        auth_context.as_ref(),
        consented_scopes.as_deref(),
        &challenge,
        public_base,
        request,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => jsonrpc_error_response(Value::Null, &err, &challenge),
    }
}

/// Whether the resolved context represents a real, non-anonymous principal.
///
/// Mirrors the branching in [`identity::mcp_identity_from_auth`]: a missing
/// context, an anonymous one, or one with no user id all count as
/// unauthenticated, and only those are worth challenging for credentials.
#[cfg(feature = "storage-rocksdb")]
fn is_authenticated(auth: Option<&AuthContext>) -> bool {
    auth.is_some_and(|ctx| ctx.is_system || (!ctx.is_anonymous && ctx.user_id.is_some()))
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
    challenge: &AuthChallenge<'_>,
    public_base: Option<String>,
    request: JsonRpcRequest,
) -> Result<Response, McpError> {
    use raisin_mcp::{assemble_for_slug, AssemblyServices, Dispatcher, NodeResourceProvider};

    use api_factory::build_mcp_function_api;
    use identity::mcp_identity_from_auth;
    use services::{BusEventSource, HttpAssetReader, HttpFunctionInvoker, HttpSearchProvider};

    // Provenance marker for everything written through this MCP server. It rides
    // on the `AuthContext` because that is what reaches the transaction commit,
    // where it is stamped onto every `NodeEvent` and from there into the audit
    // log — so any write a tool makes is attributable to BOTH the human and the
    // surface they came through, on every write path, with no per-tool plumbing.
    //
    // The server slug is the honest, always-present fact the server can vouch
    // for. MCP `clientInfo` ("Claude Code 2.1") would be more specific, but it is
    // client-asserted and the Streamable-HTTP binding is one JSON-RPC message per
    // POST with no session store to carry it from `initialize` to `tools/call`.
    // The value is namespaced so such detail can be appended later without any
    // signature change.
    let agent = format!("mcp:{slug}");
    let caller_auth = auth.map(|ctx| ctx.clone().with_agent(agent.clone()));

    // Caller-scoped backend: data tools read/write as the authenticated identity
    // (or anonymous), so RLS governs what their calls can touch.
    let backend = build_mcp_function_api(
        state,
        tenant_id,
        repo,
        branch,
        MCP_DISCOVERY_WORKSPACE,
        Default::default(),
        caller_auth.clone(),
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

    // Custom function tools execute as the authenticated caller — with the SAME
    // resolved context the data tools got above. Passing `auth` here rather than
    // letting the invoker rebuild it from the identity is load-bearing: a rebuilt
    // context carries no resolved permissions, which RLS treats as deny-all.
    let invoker: Arc<dyn raisin_mcp::FunctionInvoker> = Arc::new(HttpFunctionInvoker::new(
        state.clone(),
        tenant_id,
        repo,
        branch,
        caller_auth,
        agent,
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

    // Scope gating is the most common reason a *correctly authenticated* session
    // is still refused, and until now the reason existed only inside the
    // JSON-RPC error body — which every client swallows into "connection
    // failed". Log both sides of the comparison so a 403 names itself.
    if !descriptor.public {
        let missing = identity.missing_scopes(&descriptor.scopes);
        if !missing.is_empty() {
            tracing::warn!(
                subject = ?identity.subject,
                %repo, %branch, %slug,
                required_scopes = ?descriptor.scopes,
                granted_scopes = ?identity.scopes,
                consented_scopes = ?consented_scopes,
                missing_scopes = ?missing,
                anonymous = identity.is_anonymous(),
                "MCP session refused: caller lacks the scopes this server requires. \
                 Granted scopes come from the caller's roles + groups, intersected \
                 with the OAuth token's consented scopes when present."
            );
        }
    }

    // Wire resource reads + live subscriptions onto the event bus.
    use raisin_storage::Storage;
    let events: Arc<dyn raisin_mcp::EventSource> = Arc::new(BusEventSource::new(
        state.storage().event_bus(),
        tenant_id,
        repo,
    ));
    // Asset reader: serves raw `raisin:Asset` bytes (RLS-scoped) for blob
    // resource reads and `mode: html` MCP-UI widgets.
    let assets: Arc<dyn raisin_mcp::AssetReader> =
        Arc::new(HttpAssetReader::new(state.clone(), tenant_id, repo));

    let resources = NodeResourceProvider::new(backend)
        .with_events(events)
        .with_asset_reader(assets.clone());

    let dispatcher = Dispatcher::new(descriptor, registry)
        .with_resources(resources)
        .with_asset_reader(assets)
        .with_public_base(public_base);

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
        Err(err) => Ok(jsonrpc_error_response(id, &err, challenge)),
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
/// The error is always carried in the JSON-RPC envelope, so a client always
/// receives a parseable body. Most failures ride on HTTP `200`; authorization
/// failures do not, because an MCP client keys its OAuth handling off the HTTP
/// status:
///
/// - **`401`** when the caller is unauthenticated, carrying the RFC 9728
///   `WWW-Authenticate: Bearer resource_metadata="…"` challenge (when `challenge`
///   is `Some`) that points at this endpoint's protected-resource metadata.
/// - **`403`** when the caller *is* authenticated but lacks a required scope —
///   re-authenticating would not help, so no challenge is offered.
fn jsonrpc_error_response(id: Value, err: &McpError, challenge: &AuthChallenge<'_>) -> Response {
    let body = Json(JsonRpcResponse::failure(id, err));

    if !matches!(err, McpError::Unauthorized(_)) {
        return body.into_response();
    }

    // Authenticated but refused: a fresh token would not change the outcome.
    if challenge.authenticated {
        return (StatusCode::FORBIDDEN, body).into_response();
    }

    let mut response = (StatusCode::UNAUTHORIZED, body).into_response();
    if let Some(url) = challenge.metadata_url {
        // RFC 9728 §5.1: the `resource_metadata` parameter tells the client which
        // document describes how to authenticate for this resource.
        let value = format!(r#"Bearer error="invalid_token", resource_metadata="{url}""#);
        if let Ok(header) = axum::http::HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .insert(axum::http::header::WWW_AUTHENTICATE, header);
        }
    }
    response
}

/// How to answer an authorization failure for this request.
///
/// Carries both facts because they are independent: whether the caller was
/// authenticated decides `401` vs `403`, while the metadata URL may be missing
/// even for an unauthenticated caller (no trusted issuer could be derived) — in
/// which case the `401` is still correct, just without a discovery pointer.
#[derive(Debug, Default, Clone, Copy)]
struct AuthChallenge<'a> {
    /// Whether a real, non-anonymous principal was resolved.
    authenticated: bool,
    /// The RFC 9728 protected-resource metadata URL to advertise, if derivable.
    metadata_url: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const METADATA: &str =
        "https://db.example.com/.well-known/oauth-protected-resource/mcp/r/main/s";

    fn challenge_header(response: &Response) -> Option<&str> {
        response
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
    }

    /// An unauthenticated caller must get a `401` carrying the discovery
    /// pointer — this is the whole reason an MCP client knows to start OAuth
    /// rather than surfacing a generic connection error.
    #[test]
    fn unauthenticated_gets_401_with_resource_metadata() {
        let challenge = AuthChallenge {
            authenticated: false,
            metadata_url: Some(METADATA),
        };
        let err = McpError::unauthorized("authentication required for this MCP server");
        let response = jsonrpc_error_response(Value::from(1), &err, &challenge);

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            challenge_header(&response),
            Some(
                format!(r#"Bearer error="invalid_token", resource_metadata="{METADATA}""#).as_str()
            )
        );
    }

    /// No trusted issuer could be derived, so there is nothing to point at — but
    /// the caller is still unauthenticated, so the status must stay `401`.
    #[test]
    fn unauthenticated_without_metadata_url_is_still_401() {
        let challenge = AuthChallenge {
            authenticated: false,
            metadata_url: None,
        };
        let response =
            jsonrpc_error_response(Value::from(1), &McpError::unauthorized("nope"), &challenge);

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(challenge_header(&response), None);
    }

    /// A logged-in caller missing a scope gets `403` and no challenge: another
    /// round of OAuth would hand back the same scopes.
    #[test]
    fn authenticated_but_unscoped_gets_403_without_challenge() {
        let challenge = AuthChallenge {
            authenticated: true,
            metadata_url: None,
        };
        let response = jsonrpc_error_response(
            Value::from(1),
            &McpError::unauthorized("session is missing required scopes: admin"),
            &challenge,
        );

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(challenge_header(&response), None);
    }

    /// A bodyless / wrong-content-type probe from an unauthenticated client used
    /// to get a bare `415` from the `Json` extractor, which carries no
    /// `WWW-Authenticate` and so tells the client nothing. It must now produce
    /// the same challenge any other unauthenticated request gets.
    #[test]
    fn unparseable_body_from_unauthenticated_caller_challenges() {
        let challenge = AuthChallenge {
            authenticated: false,
            metadata_url: Some(METADATA),
        };
        let response = jsonrpc_error_response(
            Value::Null,
            &McpError::unauthorized("authentication required for this MCP server"),
            &challenge,
        );

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(challenge_header(&response).is_some_and(|h| h.contains(METADATA)));
    }

    /// An authenticated caller sending malformed JSON gets the actionable
    /// answer instead — a JSON-RPC parse error, not an auth challenge.
    #[test]
    fn unparseable_body_from_authenticated_caller_is_a_parse_error() {
        let challenge = AuthChallenge {
            authenticated: true,
            metadata_url: None,
        };
        let response = jsonrpc_error_response(
            Value::Null,
            &McpError::Parse("invalid JSON-RPC request body".to_string()),
            &challenge,
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(challenge_header(&response), None);
    }

    /// Every non-authorization failure keeps riding on `200` in the JSON-RPC
    /// envelope, as MCP clients expect.
    #[test]
    fn other_errors_stay_on_http_200() {
        let challenge = AuthChallenge {
            authenticated: false,
            metadata_url: Some(METADATA),
        };
        for err in [
            McpError::not_found("unknown method: nope"),
            McpError::invalid_params("bad params"),
        ] {
            let response = jsonrpc_error_response(Value::from(1), &err, &challenge);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(challenge_header(&response), None);
        }
    }
}
