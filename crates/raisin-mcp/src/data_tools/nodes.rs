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

//! Node CRUD / query / workspace data tools backed by [`FunctionApi`].
//!
//! Reads and writes resolve against the caller's active workspace and branch via
//! the RLS-scoped backend, so every operation is tenant- and identity-scoped by
//! construction. `query_nodes` is implemented over SQL (the node-query callback
//! is intentionally SQL-delegating in the runtime); the remaining operations map
//! to the dedicated `FunctionApi` node methods.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::registry::{Tool, ToolDescriptor, ToolKind};

use super::DataBackend;

/// Largest result set `query_nodes` will return in one call.
const MAX_QUERY_LIMIT: u64 = 500;

/// The set of workspaces this server exposes (the data policy's `workspaces`).
/// Shared across the data tools so a caller may target any one of them.
pub(crate) type AllowedWorkspaces = Arc<Vec<String>>;

/// The `workspace` argument shared by every node data tool: an optional override
/// of the caller's active workspace, restricted to the server's exposed set.
pub(crate) fn workspace_arg_schema() -> Value {
    json!({
        "type": "string",
        "description": "Workspace to operate in. Defaults to the active workspace; must be one this server exposes (see list_workspaces)."
    })
}

/// Resolve the target workspace for a data-tool call: the `workspace` argument
/// when present (validated against the exposed set), else the active workspace.
pub(crate) fn resolve_workspace<'a>(
    args: &Value,
    identity: &'a McpIdentity,
    allowed: &[String],
) -> Result<std::borrow::Cow<'a, str>> {
    match args.get("workspace").and_then(Value::as_str) {
        Some(ws) if allowed.iter().any(|w| w == ws) => {
            Ok(std::borrow::Cow::Owned(ws.to_string()))
        }
        Some(ws) => Err(McpError::invalid_params(format!(
            "workspace `{ws}` is not exposed by this MCP server (allowed: {})",
            allowed.join(", ")
        ))),
        None => Ok(std::borrow::Cow::Borrowed(identity.workspace.as_str())),
    }
}

/// `get_node` — fetch a single node by path within a workspace.
pub struct GetNodeTool {
    backend: DataBackend,
    allowed: AllowedWorkspaces,
}

impl GetNodeTool {
    /// Wrap a data backend as the `get_node` tool.
    pub fn new(backend: DataBackend, allowed: AllowedWorkspaces) -> Self {
        Self { backend, allowed }
    }
}

impl Tool for GetNodeTool {
    fn name(&self) -> &str {
        "get_node"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "get_node",
            "Fetch a single node by its path within a workspace.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute node path, e.g. \"/blog/post-1\"." },
                    "workspace": workspace_arg_schema()
                },
                "required": ["path"]
            }),
            ToolKind::Data,
        )
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let workspace = resolve_workspace(&args, identity, &self.allowed)?;
        let path = require_str(&args, "path")?;
        match self.backend.node_get(&workspace, path).await? {
            Some(node) => Ok(node),
            None => Err(McpError::not_found(format!("node not found: {path}"))),
        }
    }
}

/// `query_nodes` — list nodes of a type within a workspace.
pub struct QueryNodesTool {
    backend: DataBackend,
    allowed: AllowedWorkspaces,
}

impl QueryNodesTool {
    /// Wrap a data backend as the `query_nodes` tool.
    pub fn new(backend: DataBackend, allowed: AllowedWorkspaces) -> Self {
        Self { backend, allowed }
    }
}

impl Tool for QueryNodesTool {
    fn name(&self) -> &str {
        "query_nodes"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "query_nodes",
            "List nodes in a workspace, optionally filtered by node type or parent path.",
            json!({
                "type": "object",
                "properties": {
                    "node_type": { "type": "string", "description": "Exact node type, e.g. \"raisin:Page\"." },
                    "parent_path": { "type": "string", "description": "Restrict to direct children of this path." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_QUERY_LIMIT, "description": "Maximum rows to return." },
                    "workspace": workspace_arg_schema()
                }
            }),
            ToolKind::Data,
        )
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let workspace = resolve_workspace(&args, identity, &self.allowed)?;
        let workspace = workspace.as_ref();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_QUERY_LIMIT)
            .min(MAX_QUERY_LIMIT);

        let mut clauses: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        if let Some(node_type) = args.get("node_type").and_then(Value::as_str) {
            params.push(json!(node_type));
            clauses.push(format!("node_type = ${}", params.len()));
        }
        if let Some(parent) = args.get("parent_path").and_then(Value::as_str) {
            params.push(json!(parent));
            clauses.push(format!("parent_path = ${}", params.len()));
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let table = crate::sql::quote_workspace(workspace)?;
        let sql = format!("SELECT * FROM {table}{where_sql} LIMIT {limit}");

        let rows = self.backend.sql_query(&sql, params).await?;
        Ok(rows)
    }
}

/// `create_node` — create a new node under a parent path.
pub struct CreateNodeTool {
    backend: DataBackend,
    allowed: AllowedWorkspaces,
}

impl CreateNodeTool {
    /// Wrap a data backend as the `create_node` tool.
    pub fn new(backend: DataBackend, allowed: AllowedWorkspaces) -> Self {
        Self { backend, allowed }
    }
}

impl Tool for CreateNodeTool {
    fn name(&self) -> &str {
        "create_node"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "create_node",
            "Create a new node under a parent path in a workspace.",
            json!({
                "type": "object",
                "properties": {
                    "parent_path": { "type": "string", "description": "Path of the parent node (\"/\" for root)." },
                    "name": { "type": "string", "description": "Name of the new node." },
                    "node_type": { "type": "string", "description": "Node type, e.g. \"raisin:Page\"." },
                    "properties": { "type": "object", "description": "Initial property values." },
                    "workspace": workspace_arg_schema()
                },
                "required": ["parent_path", "name", "node_type"]
            }),
            ToolKind::Data,
        )
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let workspace = resolve_workspace(&args, identity, &self.allowed)?;
        let parent_path = require_str(&args, "parent_path")?;
        let name = require_str(&args, "name")?;
        let node_type = require_str(&args, "node_type")?;
        let properties = args
            .get("properties")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        let data = json!({
            "name": name,
            "node_type": node_type,
            "properties": properties,
        });
        let created = self
            .backend
            .node_create(&workspace, parent_path, data)
            .await?;
        Ok(created)
    }
}

/// `update_node` — update properties of an existing node by path.
pub struct UpdateNodeTool {
    backend: DataBackend,
    allowed: AllowedWorkspaces,
}

impl UpdateNodeTool {
    /// Wrap a data backend as the `update_node` tool.
    pub fn new(backend: DataBackend, allowed: AllowedWorkspaces) -> Self {
        Self { backend, allowed }
    }
}

impl Tool for UpdateNodeTool {
    fn name(&self) -> &str {
        "update_node"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "update_node",
            "Update the properties of an existing node by path in a workspace.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the node to update." },
                    "properties": { "type": "object", "description": "Property values to set." },
                    "workspace": workspace_arg_schema()
                },
                "required": ["path", "properties"]
            }),
            ToolKind::Data,
        )
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let workspace = resolve_workspace(&args, identity, &self.allowed)?;
        let path = require_str(&args, "path")?;
        let properties = args
            .get("properties")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params("missing required object field `properties`")
            })?;

        let data = json!({ "properties": properties });
        let updated = self
            .backend
            .node_update(&workspace, path, data)
            .await?;
        Ok(updated)
    }
}

/// `delete_node` — delete a node by path.
pub struct DeleteNodeTool {
    backend: DataBackend,
    allowed: AllowedWorkspaces,
}

impl DeleteNodeTool {
    /// Wrap a data backend as the `delete_node` tool.
    pub fn new(backend: DataBackend, allowed: AllowedWorkspaces) -> Self {
        Self { backend, allowed }
    }
}

impl Tool for DeleteNodeTool {
    fn name(&self) -> &str {
        "delete_node"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "delete_node",
            "Delete a node (and its descendants) by path in a workspace.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the node to delete." },
                    "workspace": workspace_arg_schema()
                },
                "required": ["path"]
            }),
            ToolKind::Data,
        )
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let workspace = resolve_workspace(&args, identity, &self.allowed)?;
        let path = require_str(&args, "path")?;
        self.backend.node_delete(&workspace, path).await?;
        Ok(json!({ "deleted": true, "path": path }))
    }
}

/// `list_workspaces` — list the workspaces this server exposes.
pub struct ListWorkspacesTool {
    workspaces: Vec<String>,
}

impl ListWorkspacesTool {
    /// Build the tool over the server's policy workspaces.
    pub fn new(workspaces: Vec<String>) -> Self {
        Self { workspaces }
    }
}

impl Tool for ListWorkspacesTool {
    fn name(&self) -> &str {
        "list_workspaces"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::no_args(
            "list_workspaces",
            "List the workspaces exposed by this MCP server.",
            ToolKind::Data,
        )
    }

    async fn call(&self, _identity: &McpIdentity, _args: Value) -> Result<Value> {
        Ok(json!({ "workspaces": self.workspaces }))
    }
}

/// Extract a required string field from a tool's argument object.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params(format!("missing required string field `{key}`")))
}
