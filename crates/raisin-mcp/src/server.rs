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

//! The MCP server *descriptor* — the configuration content authors declare.
//!
//! A `raisin:McpServer` node in a workspace describes one MCP endpoint: its
//! identity ([`name`](McpServerDescriptor::name), `version`, `slug`), whether it
//! is publicly reachable, the scopes it requires, the [`DataPolicy`] governing
//! which workspaces / operations / resources the auto-generated data tools may
//! touch, and a list of [`CustomTool`]s that map to `raisin:Function`s.
//!
//! [`crate::registry`] reads these nodes and assembles the live tool set; this
//! module is the parsed, validated shape and the operation enumeration.

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use raisin_models::nodes::Node;

use crate::error::{McpError, Result};

/// A data operation an MCP server may auto-expose as a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataOperation {
    /// `query_nodes` — list/filter nodes by type within a workspace.
    QueryNodes,
    /// `get_node` — fetch a single node by path.
    GetNode,
    /// `search_nodes` — full-text / vector search.
    SearchNodes,
    /// `create_node` — create a new node.
    CreateNode,
    /// `update_node` — update an existing node.
    UpdateNode,
    /// `delete_node` — delete a node.
    DeleteNode,
    /// `move_node` — reparent a node (and its subtree).
    MoveNode,
    /// `reorder_node` — reposition a node among its siblings (editorial order).
    ReorderNode,
    /// `list_children` — a parent's direct children, in editorial order.
    ListChildren,
    /// `list_workspaces` — list the workspaces this server exposes.
    ListWorkspaces,
}

impl DataOperation {
    /// The full set of operations, in stable order.
    pub const ALL: [DataOperation; 10] = [
        DataOperation::QueryNodes,
        DataOperation::GetNode,
        DataOperation::SearchNodes,
        DataOperation::ListChildren,
        DataOperation::CreateNode,
        DataOperation::UpdateNode,
        DataOperation::DeleteNode,
        DataOperation::MoveNode,
        DataOperation::ReorderNode,
        DataOperation::ListWorkspaces,
    ];

    /// Whether this operation mutates content.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            DataOperation::CreateNode
                | DataOperation::UpdateNode
                | DataOperation::DeleteNode
                | DataOperation::MoveNode
                | DataOperation::ReorderNode
        )
    }

    /// Parse an operation from its wire name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "query_nodes" => Some(Self::QueryNodes),
            "get_node" => Some(Self::GetNode),
            "search_nodes" => Some(Self::SearchNodes),
            "create_node" => Some(Self::CreateNode),
            "update_node" => Some(Self::UpdateNode),
            "delete_node" => Some(Self::DeleteNode),
            "move_node" => Some(Self::MoveNode),
            "reorder_node" => Some(Self::ReorderNode),
            "list_children" => Some(Self::ListChildren),
            "list_workspaces" => Some(Self::ListWorkspaces),
            _ => None,
        }
    }
}

/// Which data the auto-generated tools of a server may touch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataPolicy {
    /// Workspaces the server may expose. Empty = none.
    #[serde(default)]
    pub workspaces: Vec<String>,
    /// Operations the server enables. Empty = none.
    #[serde(default)]
    pub operations: Vec<DataOperation>,
    /// Whether `raisin://` resources are exposed for the workspaces above.
    #[serde(default)]
    pub resources: bool,
}

impl DataPolicy {
    /// Whether `op` is enabled by this policy.
    pub fn allows(&self, op: DataOperation) -> bool {
        self.operations.contains(&op)
    }

    /// Whether `workspace` is in scope for this policy.
    pub fn covers_workspace(&self, workspace: &str) -> bool {
        self.workspaces.iter().any(|w| w == workspace)
    }
}

/// How an MCP host should render a tool's bound UI widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiMode {
    /// Single-file widget: the engine reads the entry asset's bytes and returns
    /// them as an inline `text/html` resource the host renders via `srcdoc`. No
    /// cross-origin iframe, so folder serving config never applies.
    Html,
    /// Multi-file widget: the returned resource is a `text/uri-list` pointing at
    /// the static endpoint, which the host iframes with a real `src=`. This is
    /// the only mode where `raisin:StaticSiteFolder.serving_config` matters.
    UriList,
}

/// Content Security Policy domains a widget declares (MCP Apps SEP-1865).
///
/// Hosts build the sandbox CSP from these; omitted lists mean the secure
/// default (no external access of that kind).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCsp {
    /// Origins for network requests (fetch/XHR/WebSocket) — CSP `connect-src`.
    #[serde(
        default,
        alias = "connect_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub connect_domains: Vec<String>,
    /// Origins for static resources (images/scripts/styles/fonts/media).
    #[serde(
        default,
        alias = "resource_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub resource_domains: Vec<String>,
    /// Origins for nested iframes — CSP `frame-src`.
    #[serde(
        default,
        alias = "frame_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub frame_domains: Vec<String>,
    /// Allowed base URIs — CSP `base-uri`.
    #[serde(
        default,
        alias = "base_uri_domains",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub base_uri_domains: Vec<String>,
}

impl UiCsp {
    /// Whether no domain list is declared at all.
    pub fn is_empty(&self) -> bool {
        self.connect_domains.is_empty()
            && self.resource_domains.is_empty()
            && self.frame_domains.is_empty()
            && self.base_uri_domains.is_empty()
    }
}

/// One requested sandbox permission.
///
/// Serializes as `{}` — SEP-1865 models a permission request as the PRESENCE of
/// an empty object, not as a boolean. Deserializes leniently from `{}`, `true`
/// or `null` so hand-written YAML (`camera: true`) means what its author
/// intended instead of shipping a value hosts ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPermissionGrant;

impl Serialize for UiPermissionGrant {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_map(Some(0))?.end()
    }
}

impl<'de> Deserialize<'de> for UiPermissionGrant {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        // Accept anything truthy-shaped; the value carries no information, only
        // its presence does. `false` is the one spelling that must NOT grant.
        match Value::deserialize(deserializer)? {
            Value::Bool(false) | Value::Null => Err(serde::de::Error::custom(
                "permission is granted by presence; use `{}` or omit the key",
            )),
            _ => Ok(UiPermissionGrant),
        }
    }
}

/// Sandbox permissions a widget requests (SEP-1865 `_meta.ui.permissions`).
///
/// Only these four are defined by the spec. Unknown keys are ignored rather
/// than rejected — a stricter parse would fail the whole descriptor, and
/// through `CustomTool`/`assemble_registry` that takes down the entire server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<UiPermissionGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<UiPermissionGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geolocation: Option<UiPermissionGrant>,
    #[serde(
        default,
        rename = "clipboardWrite",
        alias = "clipboard_write",
        skip_serializing_if = "Option::is_none"
    )]
    pub clipboard_write: Option<UiPermissionGrant>,
}

impl UiPermissions {
    /// Whether no permission is requested at all.
    pub fn is_empty(&self) -> bool {
        self.camera.is_none()
            && self.microphone.is_none()
            && self.geolocation.is_none()
            && self.clipboard_write.is_none()
    }
}

/// A tool's optional MCP-UI binding: a delivery mode plus a workspace-relative
/// path (with an optional `#fragment`) to the widget's entry document, and the
/// MCP Apps (SEP-1865) resource metadata the widget advertises to hosts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBinding {
    /// How the host should render the widget.
    ///
    /// **Deprecated and optional.** SEP-1865 defines exactly one delivery
    /// format — inline HTML with mimeType `text/html;profile=mcp-app` — and
    /// lists external URLs (`text/uri-list`) under "Content Types (deferred
    /// from MVP)". A widget is always delivered inline regardless of what this
    /// says; see [`UiMode`].
    ///
    /// Kept deserializable rather than removed: it was required with no serde
    /// default, so a deployed node carrying `mode: uri-list` would fail to
    /// parse, and a failed `UiBinding` parse cascades through `CustomTool` and
    /// `assemble_registry` to take down the WHOLE server at `initialize` — not
    /// just the one widget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<UiMode>,
    /// Workspace-relative path to the entry document, optionally suffixed with a
    /// `#fragment` naming an in-app SPA route (`site/widgets/order/index.html#/card`).
    pub entry: String,
    /// Workspace the entry path resolves in. Defaults to the session's active
    /// workspace (the server's first declared content workspace), which cannot
    /// always hold assets — e.g. a server whose primary workspace only allows
    /// document types keeps its widgets in a sibling asset workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Display name for the `ui://` resource in `resources/list`. Defaults to
    /// the tool name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Description of the UI resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// CSP domains the widget needs. When omitted, the engine declares this
    /// server's own origin for `connect`/`resource` so widget images and API
    /// calls served from the same RaisinDB instance work out of the box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csp: Option<UiCsp>,
    /// Sandbox permissions the widget requests.
    ///
    /// Typed rather than a raw `Value` because the spec requires each granted
    /// member to be an EMPTY OBJECT — `{"camera": {}}`, not `{"camera": true}`.
    /// Passing the author's YAML through verbatim shipped whatever they wrote,
    /// and `camera: true` is the natural thing to write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<UiPermissions>,
    /// Stable sandbox origin the host should serve this widget from, e.g.
    /// `a904794854a047f6.claudemcpcontent.com`.
    ///
    /// Hosts otherwise pick a per-conversation origin. A stable one is what
    /// makes OAuth callbacks, CORS allowlists and API-key allowlists possible.
    ///
    /// The format is host-specific — Claude and ChatGPT use different domains —
    /// so this is passed through verbatim and left unset by default. A wrong
    /// value is worse than none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Whether the host should draw a visible border + background.
    #[serde(
        default,
        rename = "prefersBorder",
        skip_serializing_if = "Option::is_none"
    )]
    pub prefers_border: Option<bool>,
    /// Who may call this tool: `model` (the agent) and/or `app` (the widget).
    /// Defaults to both when omitted (the SEP-1865 default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Vec<String>>,
}

impl UiBinding {
    /// Split [`entry`](Self::entry) into its file path and optional fragment.
    ///
    /// The fragment is everything after the *first* `#` (passed through verbatim,
    /// including the `#`-less remainder); the path is everything before it. A
    /// fragment never affects which file is read — only which in-app route the
    /// widget boots into.
    pub fn split_entry(&self) -> (&str, Option<&str>) {
        split_entry(&self.entry)
    }
}

/// Split a widget `entry` string into `(path, fragment)` on the first `#`.
pub fn split_entry(entry: &str) -> (&str, Option<&str>) {
    match entry.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (entry, None),
    }
}

/// A custom tool a server author declares, mapping to a `raisin:Function`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTool {
    /// Tool name advertised to clients.
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Name of the `raisin:Function` node to invoke for this tool.
    pub function: String,
    /// JSON Schema describing the tool arguments.
    #[serde(rename = "inputSchema", default = "default_object_schema")]
    pub input_schema: Value,
    /// JSON Schema describing the tool result. Inherited from the function's
    /// `output_schema` when omitted; advertised as the MCP tool's `outputSchema`.
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    /// Scopes a caller must hold to invoke this tool.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Optional MCP-UI binding: renders a widget alongside the tool result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiBinding>,
}

fn default_object_schema() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

/// Schema and description metadata read off a `raisin:Function` node, used to
/// fill the fields a server-side custom-tool declaration left out.
#[derive(Debug, Clone)]
pub struct FunctionMeta {
    /// The function node's `name`.
    pub name: String,
    /// The function node's `description`, if any.
    pub description: Option<String>,
    /// The function node's `input_schema`, if an object.
    pub input_schema: Option<Value>,
    /// The function node's `output_schema`, if an object.
    pub output_schema: Option<Value>,
}

impl FunctionMeta {
    /// Read function metadata from a `raisin:Function` node's `properties`.
    pub fn from_props(props: &Value) -> Option<Self> {
        let name = props
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())?
            .to_string();
        let description = props
            .get("description")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let input_schema = props.get("input_schema").cloned().filter(|v| v.is_object());
        let output_schema = props
            .get("output_schema")
            .cloned()
            .filter(|v| v.is_object());
        Some(Self {
            name,
            description,
            input_schema,
            output_schema,
        })
    }
}

impl CustomTool {
    /// Build a custom tool from the `mcp` block on a `raisin:Function` node.
    ///
    /// This is the *function-side* declaration: a `raisin:Function` opts into
    /// being exposed as a tool by carrying an `mcp` object. Fields default to the
    /// function's own metadata so a bare `mcp: { enabled: true }` is sufficient:
    /// `name` / `description` / `inputSchema` fall back to the function's `name`,
    /// `description`, and `input_schema` properties respectively.
    ///
    /// Returns `None` when there is no `mcp` block, when it sets `enabled: false`,
    /// or when no usable tool name can be derived.
    pub fn from_function_properties(props: &Value) -> Option<Self> {
        let mcp = props.get("mcp")?;
        if mcp.get("enabled").and_then(Value::as_bool) == Some(false) {
            return None;
        }

        let function = props
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())?
            .to_string();

        let name = mcp
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| function.clone());

        let description = mcp
            .get("description")
            .and_then(Value::as_str)
            .or_else(|| props.get("description").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();

        let input_schema = mcp
            .get("inputSchema")
            .cloned()
            .or_else(|| props.get("input_schema").cloned())
            .filter(|v| v.is_object())
            .unwrap_or_else(default_object_schema);

        let output_schema = mcp
            .get("outputSchema")
            .cloned()
            .or_else(|| props.get("output_schema").cloned())
            .filter(|v| v.is_object());

        let scopes = mcp
            .get("scopes")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let ui = mcp
            .get("ui")
            .cloned()
            .and_then(|v| serde_json::from_value::<UiBinding>(v).ok());

        Some(Self {
            name,
            description,
            function,
            input_schema,
            output_schema,
            scopes,
            ui,
        })
    }

    /// Fill fields a server-side author omitted from the referenced function's
    /// metadata: `description` when empty, `input_schema` when left at the empty
    /// default, and `output_schema` when absent.
    pub fn fill_defaults_from(&mut self, meta: &FunctionMeta) {
        if self.description.is_empty() {
            if let Some(description) = &meta.description {
                self.description = description.clone();
            }
        }
        if self.input_schema == default_object_schema() {
            if let Some(input_schema) = &meta.input_schema {
                self.input_schema = input_schema.clone();
            }
        }
        if self.output_schema.is_none() {
            self.output_schema = meta.output_schema.clone();
        }
    }
}

/// A parsed `raisin:McpServer` descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDescriptor {
    /// Stable server name advertised in `initialize`.
    pub name: String,
    /// Server version string.
    #[serde(default = "default_version")]
    pub version: String,
    /// URL-friendly slug used to route to this server.
    pub slug: String,
    /// Natural-language usage instructions surfaced to the client/agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Whether the server is reachable without authentication.
    #[serde(default)]
    pub public: bool,
    /// Scopes a caller must hold to reach the server at all.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Auto-tool data policy.
    #[serde(default)]
    pub data_policy: DataPolicy,
    /// Author-declared custom function tools.
    #[serde(default)]
    pub custom_tools: Vec<CustomTool>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

impl McpServerDescriptor {
    /// Parse a descriptor from a `raisin:McpServer` [`Node`].
    ///
    /// Reads `name`, `version`, `slug`, `instructions`, `public`, `scopes`,
    /// `data` (`{ workspaces, operations, resources }`), and `tools`
    /// (`[{ function, name, description, inputSchema, scopes }]`) off the node's
    /// properties, tolerating absent optional keys. These property names match
    /// the `raisin:McpServer` NodeType declared by the `raisin-mcp` builtin
    /// package, so the schema and this reader stay aligned.
    pub fn from_node(node: &Node) -> Result<Self> {
        // Serialize the typed property map into a plain JSON object so the same
        // parser serves both this path and SQL-row resolution.
        let props =
            serde_json::to_value(&node.properties).unwrap_or(Value::Object(Default::default()));
        Self::from_properties(&node.name, &props)
    }

    /// Parse a descriptor from a node `name` and its raw `properties` object.
    ///
    /// This is the resolution path used over SQL rows, where `properties` is the
    /// JSON column rather than a typed [`Node`].
    pub fn from_properties(node_name: &str, props: &Value) -> Result<Self> {
        let name = str_prop(props, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| node_name.to_string());

        let slug = str_prop(props, "slug")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                McpError::protocol(format!("raisin:McpServer `{node_name}` is missing `slug`"))
            })?;

        let version = str_prop(props, "version")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(default_version);

        let instructions = str_prop(props, "instructions").filter(|s| !s.is_empty());
        let public = bool_prop(props, "public").unwrap_or(false);
        let scopes = str_array_prop(props, "scopes");

        let data_policy = parse_data_policy(node_name, props)?;
        let custom_tools = parse_custom_tools(node_name, props)?;

        Ok(Self {
            name,
            version,
            slug,
            instructions,
            public,
            scopes,
            data_policy,
            custom_tools,
        })
    }

    /// Whether `op` is enabled for this server by its data policy.
    pub fn allows_operation(&self, op: DataOperation) -> bool {
        self.data_policy.allows(op)
    }
}

/// Read a string property, descending one level into `dataPolicy` if needed.
fn str_prop(props: &Value, key: &str) -> Option<String> {
    props.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Read a boolean property.
fn bool_prop(props: &Value, key: &str) -> Option<bool> {
    props.get(key).and_then(Value::as_bool)
}

/// Read a string-array property (returns empty when absent or non-array).
fn str_array_prop(props: &Value, key: &str) -> Vec<String> {
    props
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Read the `data` object (`{ workspaces, operations, resources }`).
fn parse_data_policy(node_name: &str, props: &Value) -> Result<DataPolicy> {
    // Accept either a nested `data` object or top-level discrete keys.
    let policy = props.get("data").cloned().unwrap_or_else(|| props.clone());

    let workspaces = str_array_prop(&policy, "workspaces");

    let op_names = str_array_prop(&policy, "operations");
    let mut operations = Vec::with_capacity(op_names.len());
    for op_name in &op_names {
        let op = DataOperation::parse(op_name).ok_or_else(|| {
            McpError::protocol(format!(
                "raisin:McpServer `{node_name}` lists unknown operation `{op_name}`"
            ))
        })?;
        if !operations.contains(&op) {
            operations.push(op);
        }
    }

    let resources = bool_prop(&policy, "resources").unwrap_or(false);

    Ok(DataPolicy {
        workspaces,
        operations,
        resources,
    })
}

/// Read and parse the `tools` array.
fn parse_custom_tools(node_name: &str, props: &Value) -> Result<Vec<CustomTool>> {
    let entries = match props.get("tools").and_then(Value::as_array) {
        Some(entries) => entries,
        None => return Ok(Vec::new()),
    };

    let mut tools = Vec::with_capacity(entries.len());
    for entry in entries {
        let tool: CustomTool = serde_json::from_value(entry.clone()).map_err(|e| {
            McpError::protocol(format!(
                "raisin:McpServer `{node_name}` has an invalid tools entry: {e}"
            ))
        })?;
        tools.push(tool);
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn function_side_inherits_name_description_and_schemas() {
        let props = json!({
            "name": "recommend",
            "description": "Recommend products",
            "input_schema": { "type": "object", "properties": { "customer_id": { "type": "string" } } },
            "output_schema": { "type": "object", "properties": { "items": { "type": "array" } } },
            "mcp": { "enabled": true, "scopes": ["catalog:read"] }
        });
        let tool = CustomTool::from_function_properties(&props).expect("tool");
        assert_eq!(tool.name, "recommend"); // defaults to the function name
        assert_eq!(tool.function, "recommend");
        assert_eq!(tool.description, "Recommend products");
        assert_eq!(tool.input_schema, props["input_schema"]);
        assert_eq!(tool.output_schema, Some(props["output_schema"].clone()));
        assert_eq!(tool.scopes, vec!["catalog:read".to_string()]);
    }

    #[test]
    fn ui_binding_parses_optional_workspace() {
        let with: UiBinding = serde_json::from_value(json!({
            "mode": "html",
            "entry": "/widgets/order/index.html",
            "workspace": "assets"
        }))
        .expect("ui with workspace");
        assert_eq!(with.workspace.as_deref(), Some("assets"));

        let without: UiBinding = serde_json::from_value(json!({
            "mode": "uri-list",
            "entry": "site/widgets/order/index.html#/card"
        }))
        .expect("ui without workspace");
        assert_eq!(without.workspace, None);
        // Omitted workspace round-trips as absent, not null.
        let round = serde_json::to_value(&without).expect("serialize");
        assert!(round.get("workspace").is_none());
    }

    #[test]
    fn function_side_none_without_mcp_or_when_disabled() {
        assert!(CustomTool::from_function_properties(&json!({ "name": "f" })).is_none());
        assert!(CustomTool::from_function_properties(
            &json!({ "name": "f", "mcp": { "enabled": false } })
        )
        .is_none());
    }

    #[test]
    fn server_side_fill_defaults_from_function() {
        let mut tool = CustomTool {
            name: "recommend".to_string(),
            description: String::new(),
            function: "recommend".to_string(),
            input_schema: default_object_schema(),
            output_schema: None,
            scopes: vec![],
            ui: None,
        };
        let meta = FunctionMeta::from_props(&json!({
            "name": "recommend",
            "description": "Recommend products",
            "input_schema": { "type": "object", "properties": { "customer_id": { "type": "string" } } },
            "output_schema": { "type": "object" }
        }))
        .expect("meta");

        tool.fill_defaults_from(&meta);
        assert_eq!(tool.description, "Recommend products");
        assert_eq!(Some(tool.input_schema.clone()), meta.input_schema);
        assert_eq!(tool.output_schema, meta.output_schema);
    }

    #[test]
    fn fill_defaults_keeps_explicit_values() {
        let explicit_input = json!({ "type": "object", "properties": { "x": {} } });
        let mut tool = CustomTool {
            name: "t".to_string(),
            description: "explicit".to_string(),
            function: "f".to_string(),
            input_schema: explicit_input.clone(),
            output_schema: Some(json!({ "type": "string" })),
            scopes: vec![],
            ui: None,
        };
        let meta = FunctionMeta::from_props(&json!({
            "name": "f", "description": "fn desc",
            "input_schema": { "type": "object" }, "output_schema": { "type": "object" }
        }))
        .expect("meta");

        tool.fill_defaults_from(&meta);
        assert_eq!(tool.description, "explicit");
        assert_eq!(tool.input_schema, explicit_input);
        assert_eq!(tool.output_schema, Some(json!({ "type": "string" })));
    }
}
