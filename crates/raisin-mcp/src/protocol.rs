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

//! JSON-RPC 2.0 envelopes and the typed Model Context Protocol payloads.
//!
//! MCP frames every message as a JSON-RPC request, response, or notification.
//! The envelopes ([`JsonRpcRequest`], [`JsonRpcResponse`], [`JsonRpcError`]) are
//! the generic transport; the method-specific payloads
//! (`initialize`, `tools/list`, `tools/call`, `resources/list`,
//! `resources/read`, `resources/subscribe`) are typed structs with the exact
//! field names the MCP wire format uses. Higher layers ([`crate::dispatch`])
//! decode `params` into these and re-encode the results.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::McpError;

/// JSON-RPC protocol version string carried by every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// MCP protocol revision implemented by this server.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Identifier of a JSON-RPC request.
///
/// The spec permits a string, a number, or null; we preserve the original
/// JSON so responses can echo the caller's id verbatim.
pub type RequestId = Value;

// ---------------------------------------------------------------------------
// Generic JSON-RPC envelopes
// ---------------------------------------------------------------------------

/// An inbound JSON-RPC request (or notification when `id` is absent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request correlation id; `None` marks a notification (no response).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    /// The method name, e.g. `"tools/list"` or `"tools/call"`.
    pub method: String,
    /// Method parameters, shape depends on `method`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    /// Returns `true` when this request is a notification (expects no reply).
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// Decode `params` into a typed payload, mapping failures to a parse error.
    pub fn decode_params<T: for<'de> Deserialize<'de>>(&self) -> Result<T, McpError> {
        let params = self
            .params
            .clone()
            .unwrap_or(Value::Object(Default::default()));
        serde_json::from_value(params)
            .map_err(|e| McpError::invalid_params(format!("invalid params: {e}")))
    }
}

/// An outbound JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Echoes the originating request id.
    pub id: RequestId,
    /// Success payload; mutually exclusive with `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure payload; mutually exclusive with `result`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Build a successful response carrying `result`.
    pub fn success(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Build a failure response from an [`McpError`].
    pub fn failure(id: RequestId, err: &McpError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: err.code(),
                message: err.to_string(),
                data: None,
            }),
        }
    }
}

/// The `error` member of a failed JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Numeric error code (see [`McpError::code`]).
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ---------------------------------------------------------------------------
// `initialize`
// ---------------------------------------------------------------------------

/// Parameters of the MCP `initialize` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    /// MCP revision the client speaks.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Capabilities advertised by the client.
    #[serde(default)]
    pub capabilities: Value,
    /// Identifying information about the client.
    #[serde(rename = "clientInfo", default)]
    pub client_info: Value,
}

/// Result of the MCP `initialize` request (server handshake reply).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    /// MCP revision the server speaks.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Capabilities the server offers.
    pub capabilities: ServerCapabilities,
    /// Server identity (name / version).
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
    /// Optional natural-language usage instructions for the client/agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Server identity returned in [`InitializeResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Stable server name.
    pub name: String,
    /// Server version string.
    pub version: String,
}

/// Server capabilities advertised during the MCP `initialize` handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Present when the server exposes callable tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Present when the server exposes readable resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
}

/// Marker capability indicating tool support.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// Whether the tool list can change at runtime.
    #[serde(default, rename = "listChanged")]
    pub list_changed: bool,
}

/// Marker capability indicating resource support.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourcesCapability {
    /// Whether the server emits resource subscription updates.
    #[serde(default)]
    pub subscribe: bool,
    /// Whether the resource list can change at runtime.
    #[serde(default, rename = "listChanged")]
    pub list_changed: bool,
}

// ---------------------------------------------------------------------------
// `tools/list` and `tools/call`
// ---------------------------------------------------------------------------

/// Result of `tools/list`: the advertised tool set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListToolsResult {
    /// All tools the caller is allowed to see.
    pub tools: Vec<crate::registry::ToolDescriptor>,
}

/// Parameters of `tools/call`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolParams {
    /// Name of the tool to invoke.
    pub name: String,
    /// Arguments object passed to the tool (defaults to `{}`).
    #[serde(default)]
    pub arguments: Value,
}

/// Result of `tools/call`: structured content plus an error flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolResult {
    /// Result content blocks.
    pub content: Vec<ContentBlock>,
    /// `true` when the tool reported a domain-level failure.
    #[serde(rename = "isError", default)]
    pub is_error: bool,
    /// Machine-readable result conforming to the tool's `outputSchema`, present
    /// only for tools that declare one.
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
}

impl CallToolResult {
    /// A successful result carrying a single JSON value.
    ///
    /// Per the MCP spec, `content` items must be one of the standard block types
    /// (`text`/`image`/`audio`/`resource`) — a `json` block is rejected by strict
    /// clients (e.g. Claude Desktop). So the value is serialized into a `text`
    /// block; machine-readable callers use `structuredContent` where present.
    pub fn json(value: Value) -> Self {
        Self {
            content: vec![ContentBlock::json_text(&value)],
            is_error: false,
            structured_content: None,
        }
    }

    /// A successful result whose JSON value also satisfies the tool's
    /// `outputSchema` — carried as a spec-compliant `text` content block AND as
    /// `structuredContent`.
    pub fn json_structured(value: Value) -> Self {
        Self {
            content: vec![ContentBlock::json_text(&value)],
            is_error: false,
            structured_content: Some(value),
        }
    }

    /// An error result carrying a single text content block.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(message)],
            is_error: true,
            structured_content: None,
        }
    }
}

/// A single content block in a [`CallToolResult`] or resource read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    /// Plain-text content.
    Text {
        /// The text body.
        text: String,
    },
    /// Structured JSON content.
    Json {
        /// The JSON value.
        json: Value,
    },
    /// An embedded resource (e.g. an MCP-UI widget delivered inline as
    /// `text/html`, or a `text/uri-list` pointer to an iframable page).
    Resource {
        /// The embedded resource contents.
        resource: crate::resources::ResourceContents,
    },
}

impl ContentBlock {
    /// Build a text content block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Build a JSON content block.
    pub fn json(json: Value) -> Self {
        Self::Json { json }
    }

    /// Build an embedded-resource content block.
    pub fn resource(resource: crate::resources::ResourceContents) -> Self {
        Self::Resource { resource }
    }

    /// Build a spec-compliant `text` content block holding a serialized JSON
    /// value (pretty-printed for readability; compact on serialize failure).
    pub fn json_text(value: &Value) -> Self {
        Self::Text {
            text: serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// `resources/list`, `resources/read`, `resources/subscribe`
// ---------------------------------------------------------------------------

/// Result of `resources/list`: advertised resource descriptors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResourcesResult {
    /// All readable resources visible to the caller.
    pub resources: Vec<crate::resources::ResourceDescriptor>,
}

/// Parameters of `resources/read`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResourceParams {
    /// `raisin://` URI of the resource to read.
    pub uri: String,
}

/// Result of `resources/read`: the decoded resource contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResourceResult {
    /// One content entry per URI read.
    pub contents: Vec<crate::resources::ResourceContents>,
}

/// Parameters of `resources/subscribe` and `resources/unsubscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeResourceParams {
    /// `raisin://` URI (or URI prefix) to watch for change notifications.
    pub uri: String,
}

/// Notification payload pushed for a subscribed resource that changed.
///
/// Mirrors MCP's `notifications/resources/updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUpdatedNotification {
    /// URI of the resource that changed.
    pub uri: String,
}
