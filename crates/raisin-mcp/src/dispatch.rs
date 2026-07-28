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
    CallToolParams, CallToolResult, ContentBlock, InitializeResult, ListResourcesResult,
    ListToolsResult, ReadResourceParams, ReadResourceResult, ResourcesCapability,
    ServerCapabilities, ServerInfo, SubscribeResourceParams, ToolsCapability, PROTOCOL_VERSION,
};
use crate::registry::ToolRegistry;
use crate::resources::{NodeResourceProvider, ResourceContents};
use crate::server::{split_entry, McpServerDescriptor, UiBinding, UiMode};
use crate::services::SharedAssetReader;

/// URI scheme used to identify an MCP-UI widget resource carried in a tool
/// result. Distinct from [`crate::resources::RESOURCE_SCHEME`] (`raisin://`,
/// which addresses node content) — a `ui://` URI names a rendered widget, and
/// including the `#fragment` keeps two tools that share one SPA file but bind to
/// different routes on distinct, correctly-cached resource URIs.
const UI_RESOURCE_SCHEME: &str = "ui";

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

    /// Capabilities advertised in the `initialize` response.
    pub fn capabilities(&self) -> ServerCapabilities {
        let resources = self.resources.as_ref().map(|provider| ResourcesCapability {
            subscribe: provider.supports_subscribe(),
            list_changed: false,
        });
        ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: false,
            }),
            resources,
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
        self.authorize_session(identity)?;

        match request.method.as_str() {
            "initialize" => self.handle_initialize(),
            "tools/list" => self.handle_tools_list(identity),
            "tools/call" => self.handle_tools_call(identity, request).await,
            "resources/list" => self.handle_resources_list(identity),
            "resources/read" => self.handle_resources_read(identity, request).await,
            "resources/subscribe" => self.handle_resources_subscribe(identity, request),
            other => Err(McpError::not_found(format!("unknown method: {other}"))),
        }
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

    fn handle_initialize(&self) -> Result<Value> {
        let result = InitializeResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: self.capabilities(),
            server_info: ServerInfo {
                name: self.descriptor.name.clone(),
                version: self.descriptor.version.clone(),
            },
            instructions: self.descriptor.instructions.clone(),
        };
        Ok(serde_json::to_value(result)?)
    }

    fn handle_tools_list(&self, identity: &McpIdentity) -> Result<Value> {
        let tools = self.registry.visible_descriptors(identity);
        // MCP Apps (SEP-1865): a ui-bound tool advertises its widget as a
        // predeclared `ui://` resource via `_meta.ui.resourceUri`; Apps-capable
        // hosts fetch it with resources/read and deliver tool results to the
        // rendered view themselves. The deprecated flat key is included for
        // hosts that still read the pre-GA shape.
        let mut entries = Vec::with_capacity(tools.len());
        for tool in tools {
            let mut value = serde_json::to_value(&tool)?;
            if let Some(ui) = &tool.ui {
                let uri = self.ui_uri_for(identity, ui);
                let mut meta = json!({ "resourceUri": uri });
                if let Some(visibility) = &ui.visibility {
                    meta["visibility"] = json!(visibility);
                }
                value["_meta"] = json!({ "ui": meta, "ui/resourceUri": uri });
            }
            entries.push(value);
        }
        Ok(json!({ "tools": entries }))
    }

    /// Canonical `ui://` URI for a binding under this session (fragment
    /// stripped — it names an in-app route, never a different resource).
    fn ui_uri_for(&self, identity: &McpIdentity, ui: &UiBinding) -> String {
        let workspace = ui.workspace.as_deref().unwrap_or(&identity.workspace);
        let (path, _fragment) = ui.split_entry();
        ui_resource_uri(workspace, path)
    }

    /// The SEP-1865 `_meta.ui` object for a widget RESOURCE (csp, permissions,
    /// prefersBorder). When the binding declares no CSP, the server's own
    /// origin is declared for connect/resource so same-instance images and API
    /// calls work under the host's sandbox.
    fn ui_resource_meta(&self, ui: &UiBinding) -> Value {
        let mut meta = serde_json::Map::new();
        let csp_value = match &ui.csp {
            Some(csp) if !csp.is_empty() => Some(serde_json::to_value(csp).unwrap_or(Value::Null)),
            _ => self.public_base.as_ref().map(|base| {
                json!({ "connectDomains": [base], "resourceDomains": [base] })
            }),
        };
        if let Some(csp) = csp_value {
            meta.insert("csp".into(), csp);
        }
        if let Some(permissions) = &ui.permissions {
            meta.insert("permissions".into(), permissions.clone());
        }
        if let Some(prefers_border) = ui.prefers_border {
            meta.insert("prefersBorder".into(), json!(prefers_border));
        }
        Value::Object(meta)
    }

    async fn handle_tools_call(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let params: CallToolParams = request.decode_params()?;

        let tool = self
            .registry
            .get(&params.name)
            .ok_or_else(|| McpError::not_found(format!("unknown tool: {}", params.name)))?;

        // Per-tool scope gate.
        let descriptor = tool.descriptor();
        let missing = identity.missing_scopes(&descriptor.scopes);
        if !missing.is_empty() {
            return Err(McpError::unauthorized(format!(
                "tool `{}` requires missing scopes: {}",
                params.name,
                missing.join(", ")
            )));
        }

        // Map a function-level failure onto an MCP `isError` result rather than a
        // protocol error; everything else propagates as a JSON-RPC error.
        match tool.call(identity, params.arguments).await {
            // A tool that declares an `outputSchema` returns a result conforming
            // to it, surfaced as `structuredContent` alongside the content block.
            Ok(value) => {
                // MCP Apps (SEP-1865): a tool result carries DATA only. The
                // widget is a predeclared `ui://` resource the host discovers
                // via `_meta.ui.resourceUri`, fetches with resources/read, and
                // feeds through `ui/notifications/tool-result` — nothing UI is
                // embedded here.
                let result = if descriptor.output_schema.is_some() {
                    CallToolResult::json_structured(value)
                } else {
                    CallToolResult::json(value)
                };
                Ok(serde_json::to_value(result)?)
            }
            Err(McpError::FunctionFailed(message)) => {
                Ok(serde_json::to_value(CallToolResult::error(message))?)
            }
            Err(err) => Err(err),
        }
    }

    fn handle_resources_list(&self, identity: &McpIdentity) -> Result<Value> {
        // The resource set is open-ended (any node path); advertise the workspace
        // roots the server's data policy exposes as browsable entry points.
        let mut resources = Vec::new();
        if self.resources.is_some() {
            for workspace in &self.descriptor.data_policy.workspaces {
                resources.push(serde_json::to_value(crate::resources::ResourceDescriptor {
                    uri: crate::resources::resource_uri(workspace, "/"),
                    name: format!("{workspace} (workspace root)"),
                    description: Some(format!(
                        "Browse nodes in the `{workspace}` workspace by path."
                    )),
                    mime_type: "application/json".to_string(),
                })?);
            }
        }
        // MCP Apps (SEP-1865): predeclare each ui-bound tool's widget so hosts
        // can review and prefetch it. One SPA shared by several tools appears
        // once (deduped by resolved URI).
        let mut seen = std::collections::HashSet::new();
        for tool in self.registry.visible_descriptors(identity) {
            let Some(ui) = &tool.ui else { continue };
            let uri = self.ui_uri_for(identity, ui);
            if !seen.insert(uri.clone()) {
                continue;
            }
            let mut entry = json!({
                "uri": uri,
                "name": ui.name.clone().unwrap_or_else(|| tool.name.clone()),
                "mimeType": "text/html;profile=mcp-app",
            });
            if let Some(description) = &ui.description {
                entry["description"] = json!(description);
            }
            let meta = self.ui_resource_meta(ui);
            if meta.as_object().is_some_and(|m| !m.is_empty()) {
                entry["_meta"] = json!({ "ui": meta });
            }
            resources.push(entry);
        }
        Ok(json!({ "resources": resources }))
    }

    async fn handle_resources_read(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let params: ReadResourceParams = request.decode_params()?;
        // MCP Apps: `ui://{workspace}/{path}` resolves to a widget's HTML,
        // served with the Apps profile mime so the host renders it as a view.
        if let Some(rest) = params.uri.strip_prefix(&format!("{UI_RESOURCE_SCHEME}://")) {
            return self.read_ui_resource(identity, &params.uri, rest).await;
        }
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        let contents = provider.read(identity, &params.uri).await?;
        Ok(serde_json::to_value(ReadResourceResult {
            contents: vec![contents],
        })?)
    }

    /// Serve a `ui://` widget resource (MCP Apps SEP-1865).
    ///
    /// `rest` is `{workspace}/{entry-path}` (fragment tolerated and ignored —
    /// it names an in-app route, never a different file). The asset read is
    /// RLS-scoped to the caller like every other asset read. When the URI
    /// matches a declared tool binding, that binding's resource metadata
    /// (csp/permissions/prefersBorder) rides on the content item — the
    /// spec-preferred location, which takes precedence over listing metadata.
    async fn read_ui_resource(
        &self,
        identity: &McpIdentity,
        uri: &str,
        rest: &str,
    ) -> Result<Value> {
        let Some(assets) = self.assets.as_ref() else {
            return Err(McpError::not_found("ui resources are not enabled"));
        };
        let (rest, _fragment) = split_entry(rest);
        let (workspace, path) = rest
            .split_once('/')
            .ok_or_else(|| McpError::not_found(format!("malformed ui resource uri: {uri}")))?;
        let asset = assets
            .read_asset(identity, workspace, &format!("/{path}"))
            .await?;
        let html = String::from_utf8_lossy(&asset.bytes).into_owned();
        let mut content = json!({
            "uri": uri,
            "mimeType": "text/html;profile=mcp-app",
            "text": html,
        });
        // Attach the declaring binding's resource metadata, matched by URI.
        let canonical = {
            let (bare, _fragment) = split_entry(uri);
            bare.to_string()
        };
        let binding = self
            .registry
            .visible_descriptors(identity)
            .into_iter()
            .filter_map(|t| t.ui)
            .find(|ui| self.ui_uri_for(identity, ui) == canonical);
        if let Some(ui) = binding {
            let meta = self.ui_resource_meta(&ui);
            if meta.as_object().is_some_and(|m| !m.is_empty()) {
                content["_meta"] = json!({ "ui": meta });
            }
        }
        Ok(json!({ "contents": [content] }))
    }

    fn handle_resources_subscribe(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        let params: SubscribeResourceParams = request.decode_params()?;

        // Validate the subscription up front; the transport owns the resulting
        // stream (it forwards `notifications/resources/updated` frames). The
        // engine confirms the subscription synchronously here.
        let _ = provider.subscribe(identity, &params.uri)?;
        Ok(json!({ "subscribed": true, "uri": params.uri }))
    }
}

impl Dispatcher {
    /// Open a live resource-update stream for `uri`, for the transport to drive.
    ///
    /// Unlike the `resources/subscribe` JSON-RPC method (which only confirms the
    /// subscription), this returns the actual stream of
    /// [`ResourceUpdatedNotification`](crate::protocol::ResourceUpdatedNotification)s
    /// so a transport can forward `notifications/resources/updated` frames.
    pub fn subscribe_resource(
        &self,
        identity: &McpIdentity,
        uri: &str,
    ) -> Result<
        impl futures::Stream<Item = crate::protocol::ResourceUpdatedNotification> + Send + 'static,
    > {
        self.authorize_session(identity)?;
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        provider.subscribe(identity, uri)
    }
}

/// Convenience: wrap a raw result value as a single-JSON-block tool result.
pub fn tool_content(value: Value) -> Vec<ContentBlock> {
    vec![ContentBlock::json(value)]
}

/// Build the `ui://` identifier URI for a widget resource (fragment preserved).
fn ui_resource_uri(workspace: &str, entry: &str) -> String {
    let trimmed = entry.strip_prefix('/').unwrap_or(entry);
    format!("{UI_RESOURCE_SCHEME}://{workspace}/{trimmed}")
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    #[test]
    fn ui_resource_uri_strips_leading_slash() {
        assert_eq!(
            ui_resource_uri("assets", "/widgets/x/index.html"),
            "ui://assets/widgets/x/index.html"
        );
    }
}
