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
use crate::server::{McpServerDescriptor, UiBinding, UiMode};
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
}

impl Dispatcher {
    /// Build a dispatcher over a resolved descriptor and its tool registry.
    pub fn new(descriptor: McpServerDescriptor, registry: ToolRegistry) -> Self {
        Self {
            descriptor,
            registry,
            resources: None,
            assets: None,
        }
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
        Ok(serde_json::to_value(ListToolsResult { tools })?)
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
                let mut result = if descriptor.output_schema.is_some() {
                    CallToolResult::json_structured(value)
                } else {
                    CallToolResult::json(value)
                };
                // When the tool binds a UI widget, append its resource block. The
                // structured/text content stays intact so non-UI hosts still see
                // the data; a UI-capable host renders the widget resource.
                if let Some(ui) = &descriptor.ui {
                    if let Some(block) = self.build_ui_block(identity, ui).await? {
                        result.content.push(block);
                    }
                }
                Ok(serde_json::to_value(result)?)
            }
            Err(McpError::FunctionFailed(message)) => {
                Ok(serde_json::to_value(CallToolResult::error(message))?)
            }
            Err(err) => Err(err),
        }
    }

    /// Build the widget resource content block for a tool's `ui` binding.
    ///
    /// - `mode: html` reads the entry asset's bytes through the asset reader and
    ///   returns them inline as a `text/html` resource; a `#fragment` injects a
    ///   `window.__RAISIN_INITIAL_ROUTE__` bootstrap so the widget's router boots
    ///   into the right in-app view. Returns `Ok(None)` when no asset reader is
    ///   wired (graceful degradation).
    /// - `mode: uri-list` returns a `text/uri-list` resource whose single URL
    ///   points at the static endpoint for the entry path (fragment preserved on
    ///   the URL so the iframe's own hash router reads it).
    ///
    /// The entry path resolves against the session's active workspace.
    async fn build_ui_block(
        &self,
        identity: &McpIdentity,
        ui: &UiBinding,
    ) -> Result<Option<ContentBlock>> {
        let (path, fragment) = ui.split_entry();
        let resource_uri = ui_resource_uri(&identity.workspace, &ui.entry);

        match ui.mode {
            UiMode::Html => {
                let Some(assets) = self.assets.as_ref() else {
                    // No asset reader: return the structured result without a
                    // widget rather than failing the whole tool call.
                    return Ok(None);
                };
                let asset = assets
                    .read_asset(identity, &identity.workspace, path)
                    .await?;
                let mut html = String::from_utf8_lossy(&asset.bytes).into_owned();
                if let Some(fragment) = fragment {
                    html = inject_initial_route(&html, fragment);
                }
                Ok(Some(ContentBlock::resource(ResourceContents {
                    uri: resource_uri,
                    mime_type: "text/html".to_string(),
                    text: Some(html),
                    blob: None,
                })))
            }
            UiMode::UriList => {
                let url = static_endpoint_url(
                    &identity.repo,
                    &identity.branch,
                    &identity.workspace,
                    path,
                    fragment,
                );
                Ok(Some(ContentBlock::resource(ResourceContents {
                    uri: resource_uri,
                    mime_type: "text/uri-list".to_string(),
                    text: Some(url),
                    blob: None,
                })))
            }
        }
    }

    fn handle_resources_list(&self, _identity: &McpIdentity) -> Result<Value> {
        // The resource set is open-ended (any node path); advertise the workspace
        // roots the server's data policy exposes as browsable entry points.
        let mut resources = Vec::new();
        if self.resources.is_some() {
            for workspace in &self.descriptor.data_policy.workspaces {
                resources.push(crate::resources::ResourceDescriptor {
                    uri: crate::resources::resource_uri(workspace, "/"),
                    name: format!("{workspace} (workspace root)"),
                    description: Some(format!(
                        "Browse nodes in the `{workspace}` workspace by path."
                    )),
                    mime_type: "application/json".to_string(),
                });
            }
        }
        Ok(serde_json::to_value(ListResourcesResult { resources })?)
    }

    async fn handle_resources_read(
        &self,
        identity: &McpIdentity,
        request: &crate::protocol::JsonRpcRequest,
    ) -> Result<Value> {
        let provider = self
            .resources
            .as_ref()
            .ok_or_else(|| McpError::not_found("resources are not enabled"))?;
        let params: ReadResourceParams = request.decode_params()?;
        let contents = provider.read(identity, &params.uri).await?;
        Ok(serde_json::to_value(ReadResourceResult {
            contents: vec![contents],
        })?)
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

/// Build the static-endpoint URL a `uri-list` widget resource points at.
///
/// `GET /resources/{repo}/{branch}/{ws}/{path}`, with any `#fragment` appended
/// so the iframed SPA's hash router reads it on mount (the fragment is never
/// sent to the server — the browser strips it before the request).
fn static_endpoint_url(
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    fragment: Option<&str>,
) -> String {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    let mut url = format!("/resources/{repo}/{branch}/{workspace}/{trimmed}");
    if let Some(fragment) = fragment {
        url.push('#');
        // Strip CR/LF so a fragment can never inject an extra line into the
        // `text/uri-list` body this URL is emitted into.
        url.extend(fragment.chars().filter(|c| *c != '\r' && *c != '\n'));
    }
    url
}

/// Inject a `window.__RAISIN_INITIAL_ROUTE__` bootstrap into HTML for a
/// `mode: html` widget so its router boots into the fragment's in-app route.
///
/// The script is placed right after the opening `<head>` (or `<head ...>`) tag;
/// when no head tag is present it is prepended. The fragment is JSON-encoded (so
/// it is a valid, safely-escaped JS string) and any `</` sequence is neutralized
/// so the value can never close the `<script>` element early.
fn inject_initial_route(html: &str, fragment: &str) -> String {
    let encoded = serde_json::to_string(fragment).unwrap_or_else(|_| "\"\"".to_string());
    let safe = encoded.replace("</", "<\\/");
    let script = format!("<script>window.__RAISIN_INITIAL_ROUTE__={safe};</script>");

    // Find the end of the opening `<head>` tag, case-insensitively.
    if let Some(head_start) = find_head_open(html) {
        if let Some(rel_close) = html[head_start..].find('>') {
            let insert_at = head_start + rel_close + 1;
            let mut out = String::with_capacity(html.len() + script.len());
            out.push_str(&html[..insert_at]);
            out.push_str(&script);
            out.push_str(&html[insert_at..]);
            return out;
        }
    }

    // No usable <head>: prepend so the global is set before any body script runs.
    format!("{script}{html}")
}

/// Byte offset of an opening `<head` tag (case-insensitive), if present.
fn find_head_open(html: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    lower.find("<head")
}
