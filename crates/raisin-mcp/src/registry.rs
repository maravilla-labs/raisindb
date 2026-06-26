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

//! Tool trait, registry, and the assembly of a server's live tool set.
//!
//! A [`Tool`] is one callable MCP capability. The [`ToolRegistry`] owns the set
//! of registered tools and answers `tools/list` (via
//! [`ToolRegistry::visible_descriptors`]) and `tools/call` (via
//! [`ToolRegistry::get`]). [`assemble_registry`] resolves the
//! `raisin:McpServer` node for a `(workspace, slug)` and builds the full tool
//! set = built-in data tools (gated by the server's [`DataPolicy`]) + custom
//! `raisin:Function` tools.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use raisin_functions::FunctionApi;

use crate::data_tools::build_data_tools;
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::server::{CustomTool, McpServerDescriptor};
use crate::services::{SharedFunctionInvoker, SharedSearchProvider};

/// NodeType name of an MCP server declaration node.
pub const MCP_SERVER_NODE_TYPE: &str = "raisin:McpServer";

/// NodeType name of a serverless function node.
pub const FUNCTION_NODE_TYPE: &str = "raisin:Function";

/// Default repository used when an identity does not pin one.
pub const DEFAULT_REPO: &str = "default";

/// Kind of a tool: a built-in data operation, or a user function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    /// Auto-generated tool backed by the node/SQL/search data path.
    Data,
    /// Custom tool backed by a `raisin:Function`.
    Function,
}

/// Self-describing metadata for a tool, returned by `tools/list`.
///
/// `scopes` and `kind` are RaisinDB extensions carried alongside the standard
/// MCP `name` / `description` / `inputSchema` fields; clients that don't
/// understand them ignore them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Stable, unique tool name advertised to clients.
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the accepted argument object.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// Scopes a caller must hold to invoke this tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Whether the tool is a data operation or a function.
    pub kind: ToolKind,
}

impl ToolDescriptor {
    /// Build a descriptor with an explicit JSON Schema for its arguments.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        kind: ToolKind,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            scopes: Vec::new(),
            kind,
        }
    }

    /// Attach required scopes.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Build a no-argument data-tool descriptor.
    pub fn no_args(name: impl Into<String>, description: impl Into<String>, kind: ToolKind) -> Self {
        Self::new(
            name,
            description,
            json!({ "type": "object", "properties": {} }),
            kind,
        )
    }
}

/// A single callable MCP tool.
///
/// Native `async fn` in trait; `Send + Sync` so tools can be shared across the
/// async runtime behind an [`Arc`].
pub trait Tool: Send + Sync {
    /// Stable name this tool is invoked by; must match its descriptor.
    fn name(&self) -> &str;

    /// Self-description used to answer `tools/list`.
    fn descriptor(&self) -> ToolDescriptor;

    /// Execute the tool against decoded JSON `args` for the given `identity`.
    fn call(
        &self,
        identity: &McpIdentity,
        args: Value,
    ) -> impl std::future::Future<Output = Result<Value>> + Send;
}

/// Object-safe erased form of [`Tool`] for dynamic dispatch in the registry.
///
/// Boxes the future so heterogeneous tool types live behind one trait object.
pub trait DynTool: Send + Sync {
    /// See [`Tool::name`].
    fn name(&self) -> &str;
    /// See [`Tool::descriptor`].
    fn descriptor(&self) -> ToolDescriptor;
    /// See [`Tool::call`]; boxed future for object safety.
    fn call<'a>(
        &'a self,
        identity: &'a McpIdentity,
        args: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>;
}

impl<T: Tool> DynTool for T {
    fn name(&self) -> &str {
        Tool::name(self)
    }

    fn descriptor(&self) -> ToolDescriptor {
        Tool::descriptor(self)
    }

    fn call<'a>(
        &'a self,
        identity: &'a McpIdentity,
        args: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(Tool::call(self, identity, args))
    }
}

/// Registry of tools keyed by name.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn DynTool>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    /// Register a tool, returning an error if its name is already taken.
    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> Result<()> {
        self.register_dyn(Arc::new(tool))
    }

    /// Register an already-erased tool.
    pub fn register_dyn(&mut self, tool: Arc<dyn DynTool>) -> Result<()> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(McpError::protocol(format!(
                "tool already registered: {name}"
            )));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Look up a registered tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn DynTool>> {
        self.tools.get(name).cloned()
    }

    /// Descriptors for every registered tool, in stable name order.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|t| t.descriptor()).collect()
    }

    /// Descriptors for tools the `identity` is permitted to see and call.
    ///
    /// A tool is visible when the identity holds all of its required scopes.
    pub fn visible_descriptors(&self, identity: &McpIdentity) -> Vec<ToolDescriptor> {
        self.tools
            .values()
            .map(|t| t.descriptor())
            .filter(|d| identity.has_scopes(&d.scopes))
            .collect()
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry holds no tools.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// Resolve the `raisin:McpServer` node for `slug` in `workspace`.
///
/// Queries the node store through the RLS-scoped [`FunctionApi`] via SQL for
/// nodes of type [`MCP_SERVER_NODE_TYPE`] and matches on the `slug` property.
/// Each row's `name` and `properties` column drive
/// [`McpServerDescriptor::from_properties`].
pub async fn resolve_server_descriptor(
    backend: &Arc<dyn FunctionApi>,
    workspace: &str,
    slug: &str,
) -> Result<McpServerDescriptor> {
    let table = crate::sql::quote_workspace(workspace)?;
    let sql = format!("SELECT name, properties FROM {table} WHERE node_type = $1");
    let rows = backend
        .sql_query(&sql, vec![json!(MCP_SERVER_NODE_TYPE)])
        .await?;

    let rows = rows.as_array().cloned().unwrap_or_default();
    for row in rows {
        let node_name = row.get("name").and_then(Value::as_str).unwrap_or("");
        let props = row
            .get("properties")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));
        let descriptor = McpServerDescriptor::from_properties(node_name, &props)?;
        if descriptor.slug == slug {
            return Ok(descriptor);
        }
    }

    Err(McpError::not_found(format!(
        "no {MCP_SERVER_NODE_TYPE} with slug `{slug}` in workspace `{workspace}`"
    )))
}

/// Backends required to assemble a server's tool set.
#[derive(Clone)]
pub struct AssemblyServices {
    /// RLS-scoped node / SQL data backend (built-in data tools, registry).
    pub backend: Arc<dyn FunctionApi>,
    /// Optional search backend (`search_nodes`).
    pub search: Option<SharedSearchProvider>,
    /// Optional function invoker (custom function tools).
    pub functions: Option<SharedFunctionInvoker>,
}

/// Build the full tool registry for a resolved server descriptor.
///
/// Combines the built-in data tools (gated by the server's data policy) with the
/// custom function tools the author declared. Custom tools are only wired when a
/// [`FunctionInvoker`](crate::services::FunctionInvoker) is supplied.
pub fn assemble_registry(
    descriptor: &McpServerDescriptor,
    services: &AssemblyServices,
) -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();

    for tool in build_data_tools(
        &descriptor.data_policy,
        services.backend.clone(),
        services.search.clone(),
    ) {
        registry.register_dyn(tool)?;
    }

    if let Some(functions) = &services.functions {
        for custom in &descriptor.custom_tools {
            let tool = crate::data_tools::FunctionTool::new(custom.clone(), functions.clone());
            registry.register(tool)?;
        }
    }

    Ok(registry)
}

/// Discover function-side MCP tools across a data policy's workspaces.
///
/// Scans each workspace the server's [`DataPolicy`](crate::server::DataPolicy)
/// covers for `raisin:Function` nodes carrying an `mcp` block (see
/// [`CustomTool::from_function_properties`]) and returns one [`CustomTool`] per
/// opted-in function. This is the *function-side* declaration form, complementing
/// the server-side `tools` list on the `raisin:McpServer` node.
pub async fn discover_function_tools(
    backend: &Arc<dyn FunctionApi>,
    descriptor: &McpServerDescriptor,
) -> Result<Vec<CustomTool>> {
    let mut tools = Vec::new();
    for workspace in &descriptor.data_policy.workspaces {
        let table = crate::sql::quote_workspace(workspace)?;
        let sql = format!("SELECT properties FROM {table} WHERE node_type = $1");
        let rows = backend
            .sql_query(&sql, vec![json!(FUNCTION_NODE_TYPE)])
            .await?;
        for row in rows.as_array().cloned().unwrap_or_default() {
            let props = match row.get("properties") {
                Some(props) => props,
                None => continue,
            };
            if let Some(tool) = CustomTool::from_function_properties(props) {
                tools.push(tool);
            }
        }
    }
    Ok(tools)
}

/// Resolve a server by `(workspace, slug)` and assemble its tool registry.
///
/// The registry combines, in order: the built-in data tools gated by the data
/// policy, the server-side `tools` declared on the `raisin:McpServer` node, and
/// any function-side tools discovered from `raisin:Function` nodes that carry an
/// `mcp` block in the policy's workspaces. Function-side tools whose name is
/// already taken by a server-side tool are skipped (server-side wins).
///
/// `discovery_backend` resolves the server descriptor and the function-side tool
/// declarations. It is separate from `services.backend` (which executes the data
/// tools as the caller) so that a `public` server can be discovered by an
/// anonymous caller who cannot read the discovery workspace under RLS: reading a
/// server's own declaration is routing metadata, not user data. Access to the
/// resolved server is still gated by [`Dispatcher::authorize_session`], so a
/// non-public server is rejected for an unauthenticated caller after resolution.
pub async fn assemble_for_slug(
    services: &AssemblyServices,
    discovery_backend: &Arc<dyn FunctionApi>,
    workspace: &str,
    slug: &str,
) -> Result<(McpServerDescriptor, ToolRegistry)> {
    let descriptor = resolve_server_descriptor(discovery_backend, workspace, slug).await?;
    let mut registry = assemble_registry(&descriptor, services)?;

    if let Some(functions) = &services.functions {
        let discovered = discover_function_tools(discovery_backend, &descriptor).await?;
        for custom in discovered {
            if registry.get(&custom.name).is_some() {
                continue;
            }
            let tool = crate::data_tools::FunctionTool::new(custom, functions.clone());
            registry.register(tool)?;
        }
    }

    Ok((descriptor, registry))
}
