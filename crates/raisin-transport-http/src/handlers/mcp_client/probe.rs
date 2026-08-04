// SPDX-License-Identifier: BSL-1.1

//! `POST /api/mcp-connections/{repo}/{slug}/test` — live reachability probe.
//!
//! Answers **HTTP 200 with a structured report**, even when the connection is
//! completely broken. This mirrors `integrations::test_connection` and is the
//! whole point of the endpoint: a 502 gives the console a spinner and an error
//! toast, whereas a 200 carrying `{ reachable: false, error_code: "auth_expired" }`
//! lets it render what is actually wrong and what to do about it. The transport
//! layer only returns an error here for faults on OUR side (no master key, an
//! unreadable node).
//!
//! The probe handshakes and lists tools. It never calls one: `tools/call` has
//! side effects, and "test the connection" must not file a ticket.

use std::time::Duration;

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_mcp::client::{
    McpClientSession, McpConnectionDescriptor, RemoteToolError, StreamableHttpTransport,
};
use raisin_models::auth::AuthContext;
use serde::Serialize;
use serde_json::Value;

use super::{decrypt_credential, egress_policy, load_connection, remote_error, require_admin};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-test";

/// How many tool names the report echoes back.
const SAMPLE_TOOLS: usize = 20;

/// Structured probe result.
#[derive(Debug, Serialize)]
pub struct TestReport {
    /// Whether the handshake and listing both succeeded.
    pub reachable: bool,
    /// Revision agreed with the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    /// Server identity, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_info: Option<Value>,
    /// Capabilities the server advertises.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
    /// Usage guidance the server supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Total tools the server exposes.
    pub tool_count: usize,
    /// First [`SAMPLE_TOOLS`] tool names — NAMES ONLY, never schemas or data.
    pub tools: Vec<String>,
    /// How many of those the current tool filter would expose.
    pub permitted_tool_count: usize,
    /// Stable failure code, shared vocabulary with the connector stack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Human-readable failure detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TestReport {
    fn failed(err: &RemoteToolError) -> Self {
        Self {
            reachable: false,
            protocol_version: None,
            server_info: None,
            capabilities: None,
            instructions: None,
            tool_count: 0,
            tools: Vec::new(),
            permitted_tool_count: 0,
            error_code: Some(err.code().to_string()),
            error: Some(err.to_string()),
        }
    }
}

/// Probe a connection.
pub async fn test_connection(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<TestReport>, ApiError> {
    require_admin(auth.as_deref())?;

    let (node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    // The URL is re-checked here, not just at save time: the stored value may
    // predate a tightened allowlist, and a hostname can be re-pointed at a
    // private address after it was saved.
    if let Err(err) = egress_policy(&state).guard(&descriptor.url).await {
        return Ok(Json(TestReport::failed(&err)));
    }

    let master_key = state.get_master_key().map_err(|_| {
        ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot read MCP credentials")
    })?;
    let secret = decrypt_credential(&node, &descriptor, &master_key);
    let headers = descriptor.auth_headers(secret.as_deref());

    Ok(Json(probe(&state, &descriptor, &headers).await))
}

/// Handshake, then list tools.
async fn probe(
    state: &AppState,
    descriptor: &McpConnectionDescriptor,
    headers: &raisin_mcp::client::ExtraHeaders,
) -> TestReport {
    let config = &state.mcp_client_config;
    let timeout = Duration::from_millis(
        descriptor
            .refresh_policy
            .call_timeout_ms
            .min(config.default_timeout_ms.max(1)),
    );
    // The process-wide client: a per-probe `reqwest::Client` would build a new
    // connection pool and TLS session cache each time.
    let transport = StreamableHttpTransport::new(
        raisin_functions::shared_http_client(),
        descriptor.url.clone(),
        timeout,
    )
    .with_max_response_bytes(config.max_response_bytes);
    let session = McpClientSession::new(transport);

    let handshake = match session.handshake(headers).await {
        Ok(handshake) => handshake,
        Err(err) => return TestReport::failed(&err),
    };

    let tools = match session.list_tools(headers).await {
        Ok(tools) => tools,
        Err(err) => {
            // The handshake succeeded, so report what we DID learn alongside
            // the listing failure — "reachable but cannot list" is a different
            // problem from "cannot connect", and the console should say which.
            let mut report = TestReport::failed(&err);
            report.protocol_version = Some(handshake.protocol_version);
            report.server_info = handshake
                .server_info
                .and_then(|info| serde_json::to_value(info).ok());
            return report;
        }
    };

    let permitted_tool_count = tools
        .iter()
        .filter(|tool| descriptor.tool_filter.permits(&tool.name))
        .count();

    TestReport {
        reachable: true,
        protocol_version: Some(handshake.protocol_version),
        server_info: handshake
            .server_info
            .and_then(|info| serde_json::to_value(info).ok()),
        capabilities: serde_json::to_value(handshake.capabilities).ok(),
        instructions: handshake.instructions,
        tool_count: tools.len(),
        tools: tools
            .iter()
            .take(SAMPLE_TOOLS)
            .map(|tool| tool.name.clone())
            .collect(),
        permitted_tool_count,
        error_code: None,
        error: None,
    }
}
