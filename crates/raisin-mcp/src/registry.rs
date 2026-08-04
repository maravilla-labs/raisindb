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

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use raisin_functions::FunctionApi;

use crate::data_tools::build_data_tools;
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::server::{CustomTool, FunctionMeta, McpServerDescriptor, UiBinding};
use crate::services::{SharedFunctionInvoker, SharedSearchProvider};

/// NodeType name of an MCP server declaration node.
pub const MCP_SERVER_NODE_TYPE: &str = "raisin:McpServer";

/// NodeType name of a serverless function node.
pub const FUNCTION_NODE_TYPE: &str = "raisin:Function";

/// Canonical workspace that holds `raisin:Function` nodes.
const FUNCTIONS_WORKSPACE: &str = "functions";

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
    /// JSON Schema describing the tool's structured result, when known.
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    /// Scopes a caller must hold to invoke this tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Whether the tool is a data operation or a function.
    pub kind: ToolKind,
    /// Optional MCP-UI binding the dispatcher uses to shape the call result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiBinding>,
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
            output_schema: None,
            scopes: Vec::new(),
            kind,
            ui: None,
        }
    }

    /// Attach required scopes.
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = scopes;
        self
    }

    /// Attach a JSON Schema for the tool's structured result.
    pub fn with_output_schema(mut self, output_schema: Option<Value>) -> Self {
        self.output_schema = output_schema;
        self
    }

    /// Attach an MCP-UI binding.
    pub fn with_ui(mut self, ui: Option<UiBinding>) -> Self {
        self.ui = ui;
        self
    }

    /// Build a no-argument data-tool descriptor.
    pub fn no_args(
        name: impl Into<String>,
        description: impl Into<String>,
        kind: ToolKind,
    ) -> Self {
        Self::new(
            name,
            description,
            json!({ "type": "object", "properties": {} }),
            kind,
        )
    }

    /// Whether this tool carries an MCP-UI binding.
    pub fn has_ui(&self) -> bool {
        self.ui.is_some()
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
            let mut custom = custom.clone();
            let tool = crate::data_tools::FunctionTool::new(custom, functions.clone());
            registry.register(tool)?;
        }
    }

    Ok(registry)
}

/// A scope-gated tool must not become widget-callable by omission.
///
/// SEP-1865 defaults `visibility` to `["model", "app"]` when it is absent, so a
/// tool that declares `scopes` — i.e. one the author considered privileged —
/// silently becomes callable by the widget as well as the agent. That is the
/// wrong direction to fail in: a widget renders untrusted DATA and is a
/// confused deputy within the user's authority, so the privileged surface
/// should be opt-in, not opt-out.
///
/// Stamping an explicit `["model"]` only when the author wrote no `visibility`
/// keeps `visibility: ["model", "app"]` working for authors who mean it.
///
/// Note the limit: `visibility` lives on the UI binding, so this can only speak
/// for tools that HAVE a widget. A scope-gated tool with no `ui` emits no
/// `_meta.ui` at all and is left to the host's own defaulting — its real gate
/// is `scopes`, re-checked on every `tools/call`, which is unaffected by any of
/// this.
///
/// And that limit is why the rule is NOT applied to a tool the author bound a
/// widget to. Attaching `ui:` to a tool IS the statement that this tool belongs
/// to the app surface: the widget is rendered from its result and calls it back
/// to navigate. Stamping `["model"]` there disarmed exactly the tools the app
/// depends on — Studio's three widget tools all declare `scopes`, so all three
/// went out as model-only, and in-widget navigation had no way to work. The
/// stamp was aimed at OTHER privileged tools becoming app-callable by
/// omission, which is a real concern; a tool carrying its own widget is not an
/// omission. Those are warned about instead, so the author can decide.
fn warn_on_implicit_app_visibility(custom: &crate::server::CustomTool) {
    if custom.scopes.is_empty() {
        return;
    }
    let Some(ui) = custom.ui.as_ref() else {
        return;
    };
    if ui.visibility.is_some() {
        return;
    }
    tracing::warn!(
        tool = %custom.name,
        scopes = ?custom.scopes,
        "scope-gated tool carries a widget but declares no ui.visibility, so it \
         inherits the SEP-1865 default [\"model\", \"app\"] and the widget may call \
         it. That is usually intended for a tool the widget renders. Set \
         visibility explicitly to be sure."
    );
}

/// Scan the `properties` of `raisin:Function` nodes that may feed MCP tools.
///
/// Reads the canonical `functions` workspace plus any content workspaces the
/// server's data policy covers (deduplicated). The result drives both function-side
/// tool discovery and server-side schema inheritance.
async fn scan_function_props(
    backend: &Arc<dyn FunctionApi>,
    data_workspaces: &[String],
) -> Result<Vec<(Option<String>, Value)>> {
    let mut workspaces: Vec<&str> = vec![FUNCTIONS_WORKSPACE];
    for ws in data_workspaces {
        if ws != FUNCTIONS_WORKSPACE {
            workspaces.push(ws);
        }
    }

    let mut props = Vec::new();
    for ws in workspaces {
        let table = crate::sql::quote_workspace(ws)?;
        // `path` comes along so a tool may reference its function by path — one
        // direct read at invoke time instead of listing every function node.
        let sql = format!("SELECT path, properties FROM {table} WHERE node_type = $1");
        let rows = backend
            .sql_query(&sql, vec![json!(FUNCTION_NODE_TYPE)])
            .await?;
        for row in rows.as_array().cloned().unwrap_or_default() {
            if let Some(p) = row.get("properties") {
                let path = row.get("path").and_then(Value::as_str).map(str::to_string);
                props.push((path, p.clone()));
            }
        }
    }
    Ok(props)
}

/// Discover function-side MCP tools.
///
/// Scans `raisin:Function` nodes (canonical `functions` workspace + the data
/// policy's workspaces) for those carrying an `mcp` block (see
/// [`CustomTool::from_function_properties`]) and returns one [`CustomTool`] per
/// opted-in function. This is the *function-side* declaration form, complementing
/// the server-side `tools` list on the `raisin:McpServer` node.
pub async fn discover_function_tools(
    backend: &Arc<dyn FunctionApi>,
    descriptor: &McpServerDescriptor,
) -> Result<Vec<CustomTool>> {
    let props = scan_function_props(backend, &descriptor.data_policy.workspaces).await?;
    Ok(props
        .iter()
        .filter_map(|(_, p)| CustomTool::from_function_properties(p))
        .collect())
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
    let plan = resolve_plan(discovery_backend, workspace, slug).await?;
    let registry = assemble_from_plan(&plan, services)?;
    Ok((plan.descriptor, registry))
}

/// Everything about a server that does NOT depend on who is calling.
///
/// This is the expensive half — the SQL scans and the parsing — and it is
/// identical for every caller, so it is what a cache should hold.
///
/// **Nothing here may depend on the caller.** It is resolved through the SYSTEM
/// `discovery_backend` and handed to any caller that asks for the same
/// `(tenant, repo, branch, slug)`. Scope filtering happens downstream, per
/// request, in [`ToolRegistry::visible_descriptors`] and again — independently
/// — in `handle_tools_call`. Moving a scope decision into this struct would
/// hand one caller's tool set to another; a cached value must be safe for
/// everybody.
#[derive(Debug, Clone)]
pub struct McpServerPlan {
    /// The parsed server node, with tool schemas already inherited from the
    /// referenced `raisin:Function` nodes.
    pub descriptor: McpServerDescriptor,
    /// Function-side tools (an `mcp` block on a `raisin:Function`), already
    /// resolved against the server's named widgets. Still needs to lose any
    /// name that a server-side tool claims, which happens at bind time.
    pub function_tools: Vec<CustomTool>,
}

/// Resolve the caller-independent half: the SQL scans and the parsing.
///
/// Expensive and cacheable. This is where a request's ~0.9 s went — it was run
/// on EVERY request, before the JSON-RPC method was even looked at, so a
/// 119-byte error cost the same as a full `tools/list`.
pub async fn resolve_plan(
    discovery_backend: &Arc<dyn FunctionApi>,
    workspace: &str,
    slug: &str,
) -> Result<McpServerPlan> {
    let mut descriptor = resolve_server_descriptor(discovery_backend, workspace, slug).await?;

    // One scan of the function nodes feeding this server, reused for server-side
    // schema inheritance and function-side discovery.
    let func_props =
        scan_function_props(discovery_backend, &descriptor.data_policy.workspaces).await?;
    // Indexed by BOTH the function's `name` property and its node path, because
    // a tool may reference either. Schema inheritance failing silently is not a
    // small bug: a tool that inherits no `outputSchema` returns only a text
    // block, so its widget receives no structuredContent and renders empty.
    let mut meta_by_key: HashMap<String, FunctionMeta> = HashMap::new();
    for (path, props) in &func_props {
        let Some(meta) = FunctionMeta::from_props(props) else {
            continue;
        };
        if let Some(path) = path {
            meta_by_key.insert(path.clone(), meta.clone());
        }
        meta_by_key.insert(meta.name.clone(), meta);
    }

    // Fill omitted server-side tool fields (description / inputSchema / outputSchema)
    // from the referenced `raisin:Function`.
    for tool in &mut descriptor.custom_tools {
        if let Some(meta) = meta_by_key.get(&tool.function) {
            tool.fill_defaults_from(meta);
        }
    }

    // Function-side tools, resolved but not yet deduplicated against
    // server-side names — that needs a built registry, which is per-caller.
    let mut function_tools = Vec::new();
    for (_, props) in &func_props {
        if let Some(mut custom) = CustomTool::from_function_properties(props) {
            // A function-side `mcp` block may reference the server's named
            // widgets too; the descriptor resolved only its own tools.
            let name = custom.name.clone();
            if let Some(ui) = custom.ui.as_mut() {
                if !descriptor.resolve_ui(ui, &name) {
                    custom.ui = None;
                }
            }
            function_tools.push(custom);
        }
    }

    // Authoring diagnostics belong here, not on the per-request path: emitted
    // during binding they repeated on every single request.
    warn_on_ui_resource_conflicts(&descriptor);
    for tool in descriptor.custom_tools.iter().chain(function_tools.iter()) {
        warn_on_implicit_app_visibility(tool);
    }

    Ok(McpServerPlan {
        descriptor,
        function_tools,
    })
}

/// Bind a cached [`McpServerPlan`] to THIS caller's services.
///
/// Cheap: allocation and wiring only, no storage access. Everything expensive
/// already happened in [`resolve_plan`].
pub fn assemble_from_plan(
    plan: &McpServerPlan,
    services: &AssemblyServices,
) -> Result<ToolRegistry> {
    let mut registry = assemble_registry(&plan.descriptor, services)?;

    // Server-side tools win on a name collision.
    if let Some(functions) = &services.functions {
        for custom in &plan.function_tools {
            if registry.get(&custom.name).is_some() {
                continue;
            }
            registry.register(crate::data_tools::FunctionTool::new(
                custom.clone(),
                functions.clone(),
            ))?;
        }
    }

    Ok(registry)
}

/// Warn when two tools describe the same widget differently.
///
/// A `ui://` uri is derived from the entry document, so several tools sharing
/// one SPA — the normal case for a multi-view app — resolve to ONE resource.
/// SEP-1865 scopes `csp`, `permissions` and `domain` to that resource, but
/// RaisinDB accepts them per tool binding, so nothing structurally prevents
/// three copies from drifting apart.
///
/// The engine cannot serve two policies for one document: `resources/list`
/// dedupes by uri and `read_ui_resource` takes the first matching binding, so
/// whichever tool the registry ordered first silently wins and edits to the
/// others do nothing. Declaring the widget once in `ui_resources` and
/// referencing it by name avoids the whole question; this warns for the authors
/// who have not.
fn warn_on_ui_resource_conflicts(descriptor: &McpServerDescriptor) {
    let mut seen: HashMap<(Option<&str>, &str), &crate::server::UiBinding> = HashMap::new();
    for tool in &descriptor.custom_tools {
        let Some(ui) = tool.ui.as_ref() else { continue };
        let (path, _fragment) = ui.split_entry();
        let key = (ui.workspace.as_deref(), path);
        match seen.get(&key) {
            Some(first) if first.resource_facet() != ui.resource_facet() => tracing::warn!(
                server = %descriptor.name,
                tool = %tool.name,
                widget = %path,
                "two tools declare the same widget with different resource metadata; \
                 one document can only be served with one policy, so the first \
                 declaration wins — declare it once under `ui_resources` and reference \
                 it by name"
            ),
            Some(_) => {}
            None => {
                seen.insert(key, ui);
            }
        }
    }
}
