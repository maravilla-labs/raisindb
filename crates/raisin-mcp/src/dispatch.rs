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
    ListToolsResult, ReadResourceParams, ReadResourceResult, ResourcesCapability, ServerCapabilities,
    ServerInfo, SubscribeResourceParams, ToolsCapability, PROTOCOL_VERSION,
};
use crate::registry::ToolRegistry;
use crate::resources::NodeResourceProvider;
use crate::server::McpServerDescriptor;

/// Routes decoded MCP methods to the tool registry and resource provider.
pub struct Dispatcher {
    descriptor: McpServerDescriptor,
    registry: ToolRegistry,
    resources: Option<NodeResourceProvider>,
}

impl Dispatcher {
    /// Build a dispatcher over a resolved descriptor and its tool registry.
    pub fn new(descriptor: McpServerDescriptor, registry: ToolRegistry) -> Self {
        Self {
            descriptor,
            registry,
            resources: None,
        }
    }

    /// Attach a resource provider, enabling `resources/*` methods.
    pub fn with_resources(mut self, resources: NodeResourceProvider) -> Self {
        self.resources = Some(resources);
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
    pub async fn handle(&self, identity: &McpIdentity, request: &crate::protocol::JsonRpcRequest) -> Result<Value> {
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
