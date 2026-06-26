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
    /// `list_workspaces` — list the workspaces this server exposes.
    ListWorkspaces,
}

impl DataOperation {
    /// The full set of operations, in stable order.
    pub const ALL: [DataOperation; 7] = [
        DataOperation::QueryNodes,
        DataOperation::GetNode,
        DataOperation::SearchNodes,
        DataOperation::CreateNode,
        DataOperation::UpdateNode,
        DataOperation::DeleteNode,
        DataOperation::ListWorkspaces,
    ];

    /// Whether this operation mutates content.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            DataOperation::CreateNode | DataOperation::UpdateNode | DataOperation::DeleteNode
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
    /// Scopes a caller must hold to invoke this tool.
    #[serde(default)]
    pub scopes: Vec<String>,
}

fn default_object_schema() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
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

        Some(Self {
            name,
            description,
            function,
            input_schema,
            scopes,
        })
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
