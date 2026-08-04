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

//! The dispatch engine: maps JSON-RPC methods onto the assembled tool set.
//!
//! The [`Dispatcher`] is the transport-agnostic core. It owns a resolved
//! [`McpServerDescriptor`], its assembled [`ToolRegistry`], and an optional
//! [`NodeResourceProvider`]. Given an [`McpIdentity`] and a decoded
//! [`JsonRpcRequest`] it enforces server- and tool-level scopes, routes the six
//! supported MCP methods, and returns the spec-correct result payload. The
//! transport ([`crate::transport`]) wraps that into a [`JsonRpcResponse`].

use serde_json::{json, Value};

use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::{
    is_legacy_handshake, CallToolParams, CallToolResult, ContentBlock, DiscoverResult,
    InitializeParams, InitializeResult, ListResourcesResult, ListToolsResult, ReadResourceParams,
    ReadResourceResult, RequestMeta, ResourcesCapability, ServerCapabilities, ServerInfo,
    SubscribeResourceParams, SubscriptionFilter, SubscriptionsListenParams, ToolsCapability,
    CACHE_SCOPE_PRIVATE, META_CLIENT_CAPABILITIES, META_SERVER_INFO, PROTOCOL_VERSION,
    RESULT_TYPE_COMPLETE, SUPPORTED_PROTOCOL_VERSIONS, UI_EXTENSION_ID, UI_MIME_TYPE,
};
use crate::registry::ToolRegistry;
use crate::resources::{NodeResourceProvider, ResourceContents};
use crate::server::{split_entry, McpServerDescriptor, UiBinding, UiMode};
use crate::services::SharedAssetReader;

mod resources;
mod tools;
mod ui;

#[cfg(test)]
mod tests;

/// URI scheme used to identify an MCP-UI widget resource carried in a tool
/// result. Distinct from [`crate::resources::RESOURCE_SCHEME`] (`raisin://`,
/// which addresses node content) — a `ui://` URI names a rendered widget, and
/// including the `#fragment` keeps two tools that share one SPA file but bind to
/// different routes on distinct, correctly-cached resource URIs.
const UI_RESOURCE_SCHEME: &str = "ui";

/// Cache TTL for `server/discover`, in ms.
///
/// Capabilities change only when the server node is edited, but a stale answer
/// hides a newly granted tool, so keep it short rather than clever.
const DISCOVER_TTL_MS: u64 = 60_000;

/// Cache TTL for `tools/list` and `resources/list`, in ms. Both are derived
/// from the caller's scopes and the server descriptor.
const LIST_TTL_MS: u64 = 60_000;

/// Cache TTL for `resources/read`, in ms.
///
/// Node content changes whenever anyone writes, and this server has no
/// revalidation channel, so a read is only briefly reusable.
const READ_TTL_MS: u64 = 5_000;

/// Routes decoded MCP methods to the tool registry and resource provider.
pub struct Dispatcher {
    descriptor: McpServerDescriptor,
    registry: ToolRegistry,
    resources: Option<NodeResourceProvider>,
    assets: Option<SharedAssetReader>,
    public_base: Option<String>,
}

impl Dispatcher {
    /// Build a dispatcher over a resolved descriptor and its tool registry.
    pub fn new(descriptor: McpServerDescriptor, registry: ToolRegistry) -> Self {
        Self {
            descriptor,
            registry,
            resources: None,
            assets: None,
            public_base: None,
        }
    }

    /// Set the server's externally-reachable base URL (`https://host[:port]`).
    ///
    /// A `mode: uri-list` widget resource is a URL the HOST iframes — a
    /// relative `/resources/...` path is meaningless outside the server's own
    /// origin, so the transport derives a base (config/env or the request's
    /// Host header) and the dispatcher prefixes it. Without one the URL stays
    /// relative (previous behavior).
    pub fn with_public_base(mut self, base: Option<String>) -> Self {
        self.public_base = base.map(|b| b.trim_end_matches('/').to_string());
        self
    }

    /// Attach a resource provider, enabling `resources/*` methods.
    pub fn with_resources(mut self, resources: NodeResourceProvider) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Attach an asset reader, enabling `mode: html` MCP-UI widget resources.
    ///
    /// Without one, a tool declaring `ui: { mode: html }` still returns its
    /// structured result — the widget resource is simply omitted (graceful
    /// degradation), since serving it requires reading the entry asset's bytes.
    pub fn with_asset_reader(mut self, assets: SharedAssetReader) -> Self {
        self.assets = Some(assets);
        self
    }

    /// The resolved server descriptor.
    pub fn descriptor(&self) -> &McpServerDescriptor {
        &self.descriptor
    }

    /// The assembled tool registry.
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The resource provider, if resources are enabled.
    pub fn resources(&self) -> Option<&NodeResourceProvider> {
        self.resources.as_ref()
    }

    /// Capabilities advertised by `server/discover`.
    ///
    /// The MCP Apps extension is declared only when the caller declared it too.
    /// `ServerCapabilities.extensions` is first-class in 2026-07-28, and
    /// mirroring the client's `mimeTypes` keeps the declaration honest: we
    /// never claim a content type it did not offer.
    pub fn capabilities(&self, meta: &RequestMeta) -> ServerCapabilities {
        let resources = self.resources.as_ref().map(|provider| ResourcesCapability {
            subscribe: provider.supports_subscribe(),
            list_changed: false,
        });
        let extensions = meta
            .supports_ui()
            .then(|| json!({ UI_EXTENSION_ID: { "mimeTypes": [UI_MIME_TYPE] } }));
        ServerCapabilities {
            tools: Some(ToolsCapability {
                // Honoured: a `subscriptions/listen` stream carrying
                // `toolsListChanged` is driven from `raisin:Function` node
                // events in the functions workspace, which the event bus
                // already delivers.
                list_changed: true,
            }),
            resources,
            extensions,
        }
    }

    /// Handle one decoded request for `identity`, returning the result payload
    /// (without the JSON-RPC envelope).
    ///
    /// Enforces the server-level scope gate first, then routes the method.
    pub async fn handle(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let meta = RequestMeta::from_request(request);
        // `initialize` IS the legacy negotiation, so it cannot be subject to
        // the negotiation it predates.
        if request.method != "initialize" {
            Self::negotiate(&meta)?;
        }
        self.authorize_session(identity)?;

        match request.method.as_str() {
            "initialize" => self.handle_initialize(request),
            "server/discover" => self.handle_discover(&meta),
            "tools/list" => self.handle_tools_list(identity),
            "tools/call" => self.handle_tools_call(identity, request).await,
            "resources/list" => self.handle_resources_list(identity),
            "resources/templates/list" => self.handle_resource_templates_list(),
            "resources/read" => self.handle_resources_read(identity, request).await,
            "subscriptions/listen" => self.handle_subscriptions_listen(identity, request),
            // Legacy per-URI subscription, replaced by `subscriptions/listen`.
            "resources/subscribe" => self.handle_resources_subscribe(identity, request),
            other => Err(McpError::not_found(format!("unknown method: {other}"))),
        }
    }

    /// Enforce the per-request negotiation `_meta` carries in 2026-07-28.
    ///
    /// The revision REQUIRES both `protocolVersion` and `clientCapabilities` on
    /// every request, and forbids inferring capabilities from earlier ones —
    /// there is no handshake left to remember them from, since `initialize` was
    /// removed.
    ///
    /// Applied uniformly, including to `server/discover`: `RequestParams._meta`
    /// is required on every request in the schema, and a client that names an
    /// unsupported version still recovers, because the `-32022` error's `data`
    /// carries the `supported` list it needs to retry with.
    fn negotiate(meta: &RequestMeta) -> Result<()> {
        match meta.protocol_version.as_deref() {
            Some(version) if SUPPORTED_PROTOCOL_VERSIONS.contains(&version) => {
                // A client that negotiated an older revision through
                // `initialize` has no per-request `_meta` to give; requiring it
                // would refuse the very clients the version list admits.
                if is_legacy_handshake(version) {
                    return Ok(());
                }
            }
            Some(version) => return Err(McpError::unsupported_protocol_version(version)),
            // Absent `_meta` means a pre-2026-07-28 client mid-session: it
            // negotiated at `initialize` and the revision had no such field.
            None => return Ok(()),
        }
        if meta.client_capabilities.is_none() {
            return Err(McpError::invalid_params(format!(
                "missing required `_meta.{META_CLIENT_CAPABILITIES}`"
            )));
        }
        Ok(())
    }

    /// The legacy `initialize` handshake, for clients predating 2026-07-28.
    ///
    /// The reply echoes the client's requested revision when we support it, so
    /// the connection settles on the newest revision BOTH sides speak rather
    /// than on a version the server picked alone. Hardcoding one is what left
    /// this server answering `2025-06-18` to clients asking for `2025-11-25`.
    fn handle_initialize(&self, request: &crate::protocol::JsonRpcRequest) -> Result<Value> {
        let params: InitializeParams = request.decode_params().unwrap_or(InitializeParams {
            protocol_version: String::new(),
            capabilities: Value::Null,
            client_info: Value::Null,
        });
        let agreed = if SUPPORTED_PROTOCOL_VERSIONS.contains(&params.protocol_version.as_str()) {
            params.protocol_version.clone()
        } else {
            // Unknown revision: answer with our newest and let the client
            // decide whether it can proceed.
            PROTOCOL_VERSION.to_string()
        };
        let meta = RequestMeta {
            protocol_version: Some(agreed.clone()),
            client_capabilities: Some(params.capabilities),
            client_info: Some(params.client_info),
        };
        let result = InitializeResult {
            protocol_version: agreed,
            capabilities: self.capabilities(&meta),
            server_info: ServerInfo {
                name: self.descriptor.name.clone(),
                version: self.descriptor.version.clone(),
            },
            instructions: self.descriptor.instructions.clone(),
        };
        Ok(serde_json::to_value(result)?)
    }

    /// Reject sessions that may not open this server.
    ///
    /// A `public` server is open to anyone. Otherwise the caller must be
    /// authenticated — `public: false` means "not anonymous", even when the
    /// server declares no scopes — and must hold every scope the server requires.
    fn authorize_session(&self, identity: &McpIdentity) -> Result<()> {
        if self.descriptor.public {
            return Ok(());
        }
        if identity.is_anonymous() {
            return Err(McpError::unauthorized(
                "authentication required for this MCP server".to_string(),
            ));
        }
        let missing = identity.missing_scopes(&self.descriptor.scopes);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(McpError::unauthorized(format!(
                "session is missing required scopes: {}",
                missing.join(", ")
            )))
        }
    }

    /// `server/discover` — the 2026-07-28 replacement for `initialize`.
    ///
    /// Servers MUST implement it. Unlike the handshake it replaces it
    /// establishes no session: it advertises what this server can do, and the
    /// client then states its own version and capabilities on every request.
    fn handle_discover(&self, meta: &RequestMeta) -> Result<Value> {
        let mut result = serde_json::to_value(DiscoverResult {
            result_type: RESULT_TYPE_COMPLETE.to_string(),
            supported_versions: SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .map(|v| v.to_string())
                .collect(),
            capabilities: self.capabilities(meta),
            instructions: self.descriptor.instructions.clone(),
            // Capabilities are derived from the caller's visible tool set, so
            // this is per-identity and must not be shared across auth contexts.
            ttl_ms: DISCOVER_TTL_MS,
            cache_scope: CACHE_SCOPE_PRIVATE.to_string(),
        })?;
        self.attach_server_info(&mut result);
        Ok(result)
    }

    /// Report this server's identity in a result's `_meta`, which the spec says
    /// servers SHOULD include on every response.
    fn attach_server_info(&self, result: &mut Value) {
        let Some(object) = result.as_object_mut() else {
            return;
        };
        let meta = object
            .entry("_meta".to_string())
            .or_insert_with(|| json!({}));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(
                META_SERVER_INFO.to_string(),
                json!({
                    "name": self.descriptor.name.clone(),
                    "version": self.descriptor.version.clone(),
                }),
            );
        }
    }
}

/// Convenience: wrap a raw result value as a single-JSON-block tool result.
pub fn tool_content(value: Value) -> Vec<ContentBlock> {
    vec![ContentBlock::json(value)]
}
