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

/// Re-exported so every `crate::protocol::ContentBlock` path keeps resolving;
/// the type moved to [`crate::content`] when its codec became hand-written.
pub use crate::content::ContentBlock;

/// JSON-RPC protocol version string carried by every message.
pub const JSONRPC_VERSION: &str = "2.0";

/// MCP protocol revision implemented by this server.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// Every revision this server accepts, newest first.
///
/// Reported verbatim as `DiscoverResult::supported_versions`, and any version
/// outside it is refused with `-32022` naming this list so the client can pick
/// a mutually supported one and retry.
///
/// The older revisions are not decoration. 2026-07-28 removed `initialize` and
/// moved negotiation into per-request `_meta`, but shipping clients have not
/// followed: Claude Desktop 1.24012.9 compiles in
/// `["2025-11-25","2025-06-18","2025-03-26","2024-11-05","2024-10-07"]` and
/// contains no mention of 2026-07-28 at all. Serving only the new revision
/// would refuse every client that exists today, at the first message.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    PROTOCOL_VERSION,
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

/// Revisions that still use the `initialize` handshake, where per-request
/// `_meta` negotiation does not exist and must not be demanded.
pub const LEGACY_HANDSHAKE_VERSIONS: &[&str] = &[
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

/// Whether `version` predates the 2026-07-28 negotiation model.
pub fn is_legacy_handshake(version: &str) -> bool {
    LEGACY_HANDSHAKE_VERSIONS.contains(&version)
}

/// `_meta` key carrying the revision a request is written against (REQUIRED).
///
/// On the HTTP transport this MUST equal the `MCP-Protocol-Version` header, or
/// the server answers `400` with [`HEADER_MISMATCH`].
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

/// `_meta` key carrying the client's capabilities for THIS request (REQUIRED).
///
/// Capabilities are per-request in 2026-07-28: "Servers MUST NOT infer
/// capabilities from prior requests." That suits this server, whose Streamable
/// HTTP binding is one JSON-RPC message per POST with no session store.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// `_meta` key identifying the client software (optional, display/logging only).
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";

/// `_meta` key identifying the server software on a result (optional).
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// `_meta` key correlating a notification with its `subscriptions/listen` stream.
pub const META_SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";

/// Extension identifier for MCP Apps (SEP-1865) interactive views.
pub const UI_EXTENSION_ID: &str = "io.modelcontextprotocol/ui";

/// Mime type of an MCP Apps view resource.
pub const UI_MIME_TYPE: &str = "text/html;profile=mcp-app";

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
                data: err.data(),
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
// Per-request `_meta` (2026-07-28 negotiation)
// ---------------------------------------------------------------------------

/// The negotiation fields every request carries in `params._meta`.
///
/// This REPLACES the `initialize` handshake, which 2026-07-28 removes: there is
/// no `initialize` method in the revision's `ClientRequest` union at all.
/// Version and capabilities now ride on each request, and a server must not
/// remember them between requests.
#[derive(Debug, Clone, Default)]
pub struct RequestMeta {
    /// Revision this request is written against (REQUIRED).
    pub protocol_version: Option<String>,
    /// The client's capabilities for this request (REQUIRED).
    pub client_capabilities: Option<Value>,
    /// Self-reported client identity. Display and logging only — the spec says
    /// servers SHOULD NOT change behaviour on it or use it for security.
    pub client_info: Option<Value>,
}

impl RequestMeta {
    /// Extract the negotiation fields from a request's `params._meta`.
    pub fn from_request(request: &JsonRpcRequest) -> Self {
        let meta = request
            .params
            .as_ref()
            .and_then(|p| p.get("_meta"))
            .and_then(|m| m.as_object());
        let Some(meta) = meta else {
            return Self::default();
        };
        Self {
            protocol_version: meta
                .get(META_PROTOCOL_VERSION)
                .and_then(|v| v.as_str())
                .map(str::to_string),
            client_capabilities: meta.get(META_CLIENT_CAPABILITIES).cloned(),
            client_info: meta.get(META_CLIENT_INFO).cloned(),
        }
    }

    /// Settings the client declared for `extension_id`, when it declared it.
    ///
    /// Servers "SHOULD check client capabilities before registering UI-enabled
    /// tools" (SEP-1865); this is how, now that the declaration is per-request.
    pub fn extension(&self, extension_id: &str) -> Option<&Value> {
        self.client_capabilities
            .as_ref()?
            .get("extensions")?
            .get(extension_id)
    }

    /// Whether the client declared MCP Apps support for the Apps mime type.
    pub fn supports_ui(&self) -> bool {
        self.extension(UI_EXTENSION_ID)
            .and_then(|ext| ext.get("mimeTypes"))
            .and_then(|m| m.as_array())
            .is_some_and(|types| types.iter().any(|t| t.as_str() == Some(UI_MIME_TYPE)))
    }
}

// ---------------------------------------------------------------------------
// `server/discover`
// ---------------------------------------------------------------------------

/// Result of `server/discover`, the replacement for `initialize`.
///
/// Servers MUST implement it; clients MAY call it, since version negotiation
/// can also happen inline via per-request `_meta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverResult {
    /// Discriminates the result shape; always `"complete"` here.
    #[serde(rename = "resultType")]
    pub result_type: String,
    /// Revisions this server supports; the client picks one for later requests.
    #[serde(rename = "supportedVersions")]
    pub supported_versions: Vec<String>,
    /// Capabilities the server offers.
    pub capabilities: ServerCapabilities,
    /// Natural-language guidance describing the server and its features.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// How long (ms) the client MAY cache this response.
    #[serde(rename = "ttlMs")]
    pub ttl_ms: u64,
    /// Whether the response may be cached across authorization contexts.
    #[serde(rename = "cacheScope")]
    pub cache_scope: String,
}

/// Parameters of the legacy `initialize` request (2025-11-25 and earlier).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    /// MCP revision the client speaks.
    #[serde(rename = "protocolVersion", default)]
    pub protocol_version: String,
    /// Capabilities advertised by the client.
    #[serde(default)]
    pub capabilities: Value,
    /// Identifying information about the client.
    #[serde(rename = "clientInfo", default)]
    pub client_info: Value,
}

/// Result of the legacy `initialize` request.
///
/// Kept for clients that predate 2026-07-28, which removed the method. The
/// modern equivalent is [`DiscoverResult`], returned by `server/discover`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    /// The revision the server agreed to speak for this connection.
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

/// Server identity reported in a result's `_meta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Stable server name.
    pub name: String,
    /// Server version string.
    pub version: String,
}

/// Cache scope for a [`CacheableResult`]-shaped payload.
///
/// Everything this server returns is scoped by the caller's identity — tool and
/// resource listings are filtered by scope, and reads are RLS-filtered — so
/// results are `private`. Marking them `public` would let a shared gateway
/// serve one tenant's listing to another.
pub const CACHE_SCOPE_PRIVATE: &str = "private";

/// `resultType` for a completed request.
pub const RESULT_TYPE_COMPLETE: &str = "complete";

/// Server capabilities advertised by `server/discover`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Present when the server exposes callable tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Present when the server exposes readable resources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    /// Optional MCP extensions, keyed by reverse-DNS extension identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Value>,
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

/// `serde` default for a `resultType` field a peer omitted.
///
/// The cacheable-result fields below are 2026-07-28 additions. Every shipping
/// server predates them, so a client that requires them cannot parse a single
/// real `tools/list` response. Defaulting is what makes this struct usable in
/// the client direction as well as the server one.
fn default_result_type() -> String {
    RESULT_TYPE_COMPLETE.to_string()
}

/// Parameters of `tools/list`.
///
/// Sent by the client. `cursor` carries the previous page's `nextCursor`;
/// absent on the first page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListToolsParams {
    /// Opaque pagination cursor from the previous page's `nextCursor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Result of `tools/list`: the advertised tool set.
///
/// Generic over the descriptor type so one envelope serves both directions:
/// the server fills it with RaisinDB's extended `ToolDescriptor` (which lives
/// in `raisin-mcp`, since it carries a UI binding the client has no use for),
/// while the client parses a remote reply into
/// [`crate::client::RemoteToolDescriptor`], which carries only spec fields.
///
/// There is deliberately no default type parameter: defaulting it to the
/// server's descriptor is what tied this module to the server half.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListToolsResult<T> {
    /// Discriminates the result shape; always `"complete"` here.
    #[serde(rename = "resultType", default = "default_result_type")]
    pub result_type: String,
    /// All tools the caller is allowed to see.
    pub tools: Vec<T>,
    /// Pagination cursor for the next page; `None` on the last page.
    ///
    /// Dropping this silently truncates a paginated server's tool set to its
    /// first page, so every client-side listing MUST loop until it is `None`.
    #[serde(
        rename = "nextCursor",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub next_cursor: Option<String>,
    /// How long (ms) the client MAY cache this listing.
    #[serde(rename = "ttlMs", default)]
    pub ttl_ms: u64,
    /// Cache scope — `private`, since the listing is filtered by caller scope.
    #[serde(rename = "cacheScope", default)]
    pub cache_scope: String,
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
    /// Discriminates the result shape; always `"complete"` here. A tool that
    /// needs more input would answer `input_required` with an
    /// `InputRequiredResult` instead — not yet implemented.
    #[serde(rename = "resultType", default = "default_result_type")]
    pub result_type: String,
    /// Result content blocks. Absent on a structured-content-only result.
    #[serde(default)]
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
            result_type: RESULT_TYPE_COMPLETE.to_string(),
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
            result_type: RESULT_TYPE_COMPLETE.to_string(),
            content: vec![ContentBlock::json_text(&value)],
            is_error: false,
            structured_content: Some(value),
        }
    }

    /// An error result carrying a single text content block.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            result_type: RESULT_TYPE_COMPLETE.to_string(),
            content: vec![ContentBlock::text(message)],
            is_error: true,
            structured_content: None,
        }
    }
}

// ---------------------------------------------------------------------------
// `resources/list`, `resources/read`, `resources/subscribe`
// ---------------------------------------------------------------------------

/// Result of `resources/list`: advertised resource descriptors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResourcesResult {
    /// Discriminates the result shape; always `"complete"` here.
    #[serde(rename = "resultType")]
    pub result_type: String,
    /// All readable resources visible to the caller.
    pub resources: Vec<crate::resource_types::ResourceDescriptor>,
    /// How long (ms) the client MAY cache this listing.
    #[serde(rename = "ttlMs")]
    pub ttl_ms: u64,
    /// Cache scope — `private`, since the listing is filtered by caller scope.
    #[serde(rename = "cacheScope")]
    pub cache_scope: String,
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
    /// Discriminates the result shape; always `"complete"` here.
    #[serde(rename = "resultType")]
    pub result_type: String,
    /// One content entry per URI read.
    pub contents: Vec<crate::resource_types::ResourceContents>,
    /// How long (ms) the client MAY cache these contents.
    #[serde(rename = "ttlMs")]
    pub ttl_ms: u64,
    /// Cache scope — `private`: reads are RLS-filtered per caller.
    #[serde(rename = "cacheScope")]
    pub cache_scope: String,
}

/// Parameters of `resources/subscribe` and `resources/unsubscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeResourceParams {
    /// `raisin://` URI (or URI prefix) to watch for change notifications.
    pub uri: String,
}

/// Parameters of `subscriptions/listen`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubscriptionsListenParams {
    /// The notification types the client opts in to on this stream.
    #[serde(default)]
    pub notifications: SubscriptionFilter,
}

/// Which notification types a `subscriptions/listen` stream carries.
///
/// Every type is opt-in and the server MUST NOT send one the client did not
/// request, which is why the acknowledgement echoes the granted subset rather
/// than the requested one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    /// Deliver `notifications/tools/list_changed`.
    #[serde(default, rename = "toolsListChanged")]
    pub tools_list_changed: bool,
    /// Deliver `notifications/prompts/list_changed`.
    #[serde(default, rename = "promptsListChanged")]
    pub prompts_list_changed: bool,
    /// Deliver `notifications/resources/list_changed`.
    #[serde(default, rename = "resourcesListChanged")]
    pub resources_list_changed: bool,
    /// Deliver `notifications/resources/updated` for these URIs.
    #[serde(default, rename = "resourceSubscriptions")]
    pub resource_subscriptions: Vec<String>,
}

/// Notification payload pushed for a subscribed resource that changed.
///
/// Mirrors MCP's `notifications/resources/updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUpdatedNotification {
    /// URI of the resource that changed.
    pub uri: String,
}
