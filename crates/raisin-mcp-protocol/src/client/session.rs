// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! A negotiated conversation with one remote MCP server.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

use super::error::{RemoteToolError, Result};
use super::transport::{ExtraHeaders, StreamableHttpTransport};
use crate::protocol::{
    CallToolParams, CallToolResult, InitializeResult, ListToolsParams, ListToolsResult,
    ServerCapabilities, ServerInfo, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};

/// Client identity reported in the handshake (display/logging only).
const CLIENT_NAME: &str = "raisindb";

/// Hard cap on `tools/list` pages, so a server that returns a cursor forever
/// cannot spin the discovery job indefinitely.
const MAX_TOOL_PAGES: usize = 50;

/// A remote server's tool as IT declares it — spec fields only.
///
/// Deliberately NOT [`crate::registry::ToolDescriptor`], which carries RaisinDB
/// extensions (`kind`, `scopes`, `ui`) and requires a `description` that remote
/// servers routinely omit. Parsing a foreign server into the local type would
/// fail on the first real response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteToolDescriptor {
    /// Tool name as the remote server knows it. Sent verbatim in `tools/call`.
    pub name: String,
    /// Optional human-friendly display title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional natural-language description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema for the tool's arguments.
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
    /// Optional JSON Schema for a structured result.
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    /// Optional behavioural hints (read-only, destructive, idempotent…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

/// What the handshake established about the peer.
#[derive(Debug, Clone)]
pub struct ServerHandshake {
    /// The revision both sides agreed to speak.
    pub protocol_version: String,
    /// Server identity, when it reported one.
    pub server_info: Option<ServerInfo>,
    /// Capabilities the server advertises.
    pub capabilities: ServerCapabilities,
    /// Usage guidance the server supplied.
    pub instructions: Option<String>,
}

impl ServerHandshake {
    /// Whether the server promises to announce tool-list changes.
    ///
    /// A legacy peer that says `false` will never push
    /// `notifications/tools/list_changed`, so holding a stream open for it burns
    /// a connection to hear nothing. Absent capabilities mean `false`: a server
    /// that did not claim the guarantee has not made it.
    pub fn announces_tool_changes(&self) -> bool {
        self.capabilities
            .tools
            .as_ref()
            .is_some_and(|tools| tools.list_changed)
    }

    /// Whether the peer speaks the 2026-07-28 subscription model.
    ///
    /// Decides which way a listener opens its stream: `subscriptions/listen` for
    /// a modern peer, the long-lived GET for everyone else.
    pub fn uses_listen_subscriptions(&self) -> bool {
        !crate::protocol::is_legacy_handshake(&self.protocol_version)
    }
}

/// A negotiated conversation with one remote MCP server.
pub struct McpClientSession {
    transport: StreamableHttpTransport,
    next_id: AtomicU64,
    handshake: Mutex<Option<ServerHandshake>>,
}

impl McpClientSession {
    /// Wrap a transport in a session. No I/O happens until the first call.
    pub fn new(transport: StreamableHttpTransport) -> Self {
        Self {
            transport,
            next_id: AtomicU64::new(1),
            handshake: Mutex::new(None),
        }
    }

    /// The transport underneath, for callers that need the endpoint.
    pub fn transport(&self) -> &StreamableHttpTransport {
        &self.transport
    }

    /// Negotiate if we have not already, and report what was agreed.
    pub async fn handshake(&self, auth: &ExtraHeaders) -> Result<ServerHandshake> {
        let mut guard = self.handshake.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.clone());
        }
        let negotiated = self.negotiate(auth).await?;
        self.transport
            .set_protocol_version(negotiated.protocol_version.clone());
        *guard = Some(negotiated.clone());
        Ok(negotiated)
    }

    /// Perform the handshake, `initialize` first.
    ///
    /// Order matters and is the opposite of what the newest spec implies.
    /// 2026-07-28 replaced `initialize` with `server/discover`, but essentially
    /// every server deployed today predates it — leading with `server/discover`
    /// would fail against real servers on the very first message. So we try
    /// `initialize`, and fall back to `server/discover` only when the peer says
    /// it has no such method.
    async fn negotiate(&self, auth: &ExtraHeaders) -> Result<ServerHandshake> {
        let params = json!({
            "protocolVersion": SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v != PROTOCOL_VERSION)
                .copied()
                .unwrap_or(PROTOCOL_VERSION),
            "capabilities": {},
            "clientInfo": { "name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION") },
        });

        match self.send("initialize", params, auth).await {
            Ok(value) => {
                let result: InitializeResult = serde_json::from_value(value).map_err(|e| {
                    RemoteToolError::Protocol(format!("malformed initialize result: {e}"))
                })?;
                // Per spec the client confirms before issuing other requests.
                // Best-effort: a server that rejects the notification is still
                // usable, and failing the whole connection over it would be worse.
                let _ = self
                    .transport
                    .notify("notifications/initialized", json!({}), auth)
                    .await;
                Ok(ServerHandshake {
                    protocol_version: result.protocol_version,
                    server_info: Some(result.server_info),
                    capabilities: result.capabilities,
                    instructions: result.instructions,
                })
            }
            Err(RemoteToolError::Config(message)) if message.contains("method not found") => {
                self.discover(auth).await
            }
            Err(err) => Err(err),
        }
    }

    /// 2026-07-28 handshake, used only when `initialize` is absent.
    async fn discover(&self, auth: &ExtraHeaders) -> Result<ServerHandshake> {
        let value = self.send("server/discover", json!({}), auth).await?;
        let supported: Vec<String> = value
            .get("supportedVersions")
            .and_then(Value::as_array)
            .map(|versions| {
                versions
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let agreed = SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .find(|ours| supported.iter().any(|theirs| theirs == *ours))
            .map(|v| v.to_string())
            .ok_or_else(|| {
                RemoteToolError::Protocol(format!(
                    "no mutually supported protocol revision (server offers: {})",
                    supported.join(", ")
                ))
            })?;
        Ok(ServerHandshake {
            protocol_version: agreed,
            server_info: value
                .get("serverInfo")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            capabilities: value
                .get("capabilities")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            instructions: value
                .get("instructions")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// List every tool the server exposes, following pagination to the end.
    pub async fn list_tools(&self, auth: &ExtraHeaders) -> Result<Vec<RemoteToolDescriptor>> {
        self.handshake(auth).await?;

        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors: Vec<String> = Vec::new();

        for _ in 0..MAX_TOOL_PAGES {
            let params = serde_json::to_value(ListToolsParams {
                cursor: cursor.clone(),
            })
            .unwrap_or_else(|_| json!({}));
            let value = self.call_with_replay("tools/list", params, auth).await?;
            let page: ListToolsResult<RemoteToolDescriptor> = serde_json::from_value(value)
                .map_err(|e| {
                    RemoteToolError::Protocol(format!("malformed tools/list result: {e}"))
                })?;
            tools.extend(page.tools);

            match page.next_cursor {
                None => return Ok(tools),
                // A server that repeats a cursor would otherwise loop until the
                // page cap, re-appending the same tools each time.
                Some(next) if seen_cursors.contains(&next) => {
                    return Err(RemoteToolError::Protocol(
                        "tools/list repeated a pagination cursor".into(),
                    ))
                }
                Some(next) => {
                    seen_cursors.push(next.clone());
                    cursor = Some(next);
                }
            }
        }
        Err(RemoteToolError::Protocol(format!(
            "tools/list exceeded {MAX_TOOL_PAGES} pages"
        )))
    }

    /// Invoke one remote tool.
    ///
    /// `name` MUST be the remote server's own tool name, never a locally
    /// sanitized one.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        auth: &ExtraHeaders,
    ) -> Result<CallToolResult> {
        self.handshake(auth).await?;
        let params = serde_json::to_value(CallToolParams {
            name: name.to_string(),
            arguments,
        })
        .map_err(|e| RemoteToolError::Protocol(format!("cannot encode tool arguments: {e}")))?;

        let value = self.call_with_replay("tools/call", params, auth).await?;
        serde_json::from_value(value)
            .map_err(|e| RemoteToolError::Protocol(format!("malformed tools/call result: {e}")))
    }

    /// Send a request, re-initializing and replaying ONCE if the server dropped
    /// our session.
    ///
    /// Bounded to a single replay on [`RemoteToolError::SessionExpired`] alone —
    /// this is session recovery, not a general retry. A `tools/call` that failed
    /// for any other reason is never re-sent: MCP has no idempotency key, so a
    /// blind retry can charge a card or file a ticket twice.
    async fn call_with_replay(
        &self,
        method: &str,
        params: Value,
        auth: &ExtraHeaders,
    ) -> Result<Value> {
        match self.send(method, params.clone(), auth).await {
            Err(RemoteToolError::SessionExpired) => {
                *self.handshake.lock().await = None;
                self.handshake(auth).await?;
                self.send(method, params, auth).await
            }
            other => other,
        }
    }

    /// Issue one JSON-RPC request with a fresh id.
    async fn send(&self, method: &str, params: Value, auth: &ExtraHeaders) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.transport.request(id, method, params, auth).await
    }
}

/// Flatten a [`CallToolResult`]'s text blocks into one string, for error
/// reporting and for the single-text-block result shape.
pub fn concat_text(result: &CallToolResult) -> String {
    let parts: Vec<&str> = result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .collect();
    parts.join("\n")
}

/// Merge tool arguments into a JSON object, rejecting non-object input.
///
/// MCP requires `arguments` to be an object; passing a bare scalar produces a
/// confusing server-side schema error instead of a clear local one.
pub fn ensure_object(arguments: Value) -> Result<Value> {
    match arguments {
        Value::Object(map) => Ok(Value::Object(map)),
        Value::Null => Ok(Value::Object(Map::new())),
        other => Err(RemoteToolError::Config(format!(
            "tool arguments must be a JSON object, got {}",
            match other {
                Value::Array(_) => "an array",
                Value::String(_) => "a string",
                Value::Number(_) => "a number",
                Value::Bool(_) => "a boolean",
                _ => "a scalar",
            }
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_remote_tool() {
        // The shape a real 2025-06-18 server sends: no title, no outputSchema,
        // no annotations, and none of RaisinDB's own descriptor extensions.
        let tool: RemoteToolDescriptor = serde_json::from_value(json!({
            "name": "search_issues",
            "description": "Search Linear issues",
            "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } },
        }))
        .expect("a spec-shaped tool must parse");

        assert_eq!(tool.name, "search_issues");
        assert!(tool.output_schema.is_none());
    }

    #[test]
    fn parses_a_tool_with_no_description() {
        let tool: RemoteToolDescriptor =
            serde_json::from_value(json!({ "name": "ping", "inputSchema": {} }))
                .expect("description is optional on the wire");
        assert_eq!(tool.description, None);
    }

    #[test]
    fn parses_a_legacy_tools_list_page() {
        // No resultType, no ttlMs, no cacheScope, no nextCursor — i.e. nothing
        // the 2026-07-28 cacheable-result shape adds. This is the regression
        // test for the client being unable to parse any real server.
        let page: ListToolsResult<RemoteToolDescriptor> = serde_json::from_value(json!({
            "tools": [ { "name": "a", "inputSchema": {} } ]
        }))
        .expect("a legacy tools/list page must parse");

        assert_eq!(page.tools.len(), 1);
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.result_type, "complete");
    }

    #[test]
    fn parses_a_paginated_tools_list_page() {
        let page: ListToolsResult<RemoteToolDescriptor> = serde_json::from_value(json!({
            "tools": [ { "name": "a", "inputSchema": {} } ],
            "nextCursor": "page-2",
        }))
        .unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
    }

    #[test]
    fn parses_a_legacy_call_tool_result() {
        let result: CallToolResult = serde_json::from_value(json!({
            "content": [ { "type": "text", "text": "hello" } ]
        }))
        .expect("a legacy tools/call result must parse");

        assert!(!result.is_error);
        assert_eq!(concat_text(&result), "hello");
    }

    #[test]
    fn parses_a_structured_only_call_tool_result() {
        let result: CallToolResult =
            serde_json::from_value(json!({ "structuredContent": { "count": 2 } }))
                .expect("content is optional when structuredContent is present");
        assert!(result.content.is_empty());
        assert_eq!(result.structured_content, Some(json!({ "count": 2 })));
    }

    #[test]
    fn rejects_non_object_arguments() {
        assert!(ensure_object(json!("nope")).is_err());
        assert_eq!(ensure_object(Value::Null).unwrap(), json!({}));
        assert_eq!(ensure_object(json!({ "a": 1 })).unwrap(), json!({ "a": 1 }));
    }
}
