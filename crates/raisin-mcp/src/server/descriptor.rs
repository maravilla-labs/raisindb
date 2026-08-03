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

//! The parsed `raisin:McpServer` node: one MCP endpoint's whole configuration.
//!
//! Property names here match the `raisin:McpServer` NodeType shipped by the
//! `raisin-mcp` builtin package verbatim, so the schema an author edits against
//! and this reader stay aligned.
//!
//! Parsing is deliberately forgiving about author mistakes that are LOCAL — an
//! unresolvable widget reference drops that widget — because a returned `Err`
//! is not local: it propagates through `assemble_registry` to `initialize` and
//! takes every tool on the server down with it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use raisin_models::nodes::Node;

use super::{CustomTool, DataOperation, DataPolicy, UiBinding, UiResource};
use crate::error::{McpError, Result};

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
    /// Widgets declared once and referenced by name from `ui: { resource: … }`.
    #[serde(default, alias = "uiResources")]
    pub ui_resources: Vec<UiResource>,
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
        let ui_resources = parse_ui_resources(node_name, props)?;

        let mut custom_tools = custom_tools;
        // Resolve `ui: { resource: … }` once, here, so everything downstream —
        // uri minting, `_meta.ui`, the read path — sees a binding that is
        // already complete and needs no knowledge of named resources.
        for tool in &mut custom_tools {
            let servable = match tool.ui.as_mut() {
                Some(ui) => resolve_ui(ui, &ui_resources, &tool.name, &name),
                None => true,
            };
            if !servable {
                tool.ui = None;
            }
        }

        Ok(Self {
            name,
            version,
            slug,
            instructions,
            public,
            scopes,
            data_policy,
            custom_tools,
            ui_resources,
        })
    }

    /// Fill a binding from the `ui_resources` entry it names, returning whether
    /// the result is servable.
    ///
    /// For bindings this descriptor did not parse — the function-side `mcp`
    /// block, assembled in [`crate::registry`] — which still resolve against
    /// this server's resources.
    #[must_use]
    pub fn resolve_ui(&self, ui: &mut UiBinding, tool_name: &str) -> bool {
        resolve_ui(ui, &self.ui_resources, tool_name, &self.name)
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

/// Fill `ui` from the `ui_resources` entry it names, and report whether the
/// result can actually be served.
///
/// A dangling reference warns and leaves the binding as authored: it is almost
/// always a typo, and rejecting the descriptor would take down the whole server
/// over one mistyped widget name.
///
/// Returns `false` when no entry document could be resolved. The caller drops
/// the BINDING, not the tool — the tool stays callable as text-only, which is
/// what the spec asks for an unrenderable widget. Left in place it would mint
/// `ui://{workspace}/` and 404 on read while still advertising a widget.
fn resolve_ui(ui: &mut UiBinding, resources: &[UiResource], tool: &str, server: &str) -> bool {
    if let Some(id) = ui.resource.as_deref() {
        match resources.iter().find(|resource| resource.id == id) {
            Some(resource) => ui.inherit_from(&resource.ui),
            None => tracing::warn!(
                %tool,
                resource = %id,
                %server,
                "ui binding references an undeclared ui_resources entry"
            ),
        }
    }
    if ui.entry.is_empty() {
        tracing::warn!(
            %tool,
            %server,
            "ui binding has neither `entry` nor a resolvable `resource`; \
             serving the tool without a widget"
        );
        return false;
    }
    true
}

/// Read the `ui_resources` array (`[{ id, entry, workspace, csp, … }]`).
fn parse_ui_resources(node_name: &str, props: &Value) -> Result<Vec<UiResource>> {
    let entries = match props
        .get("ui_resources")
        .or_else(|| props.get("uiResources"))
        .and_then(Value::as_array)
    {
        Some(entries) => entries,
        None => return Ok(Vec::new()),
    };

    let mut resources = Vec::with_capacity(entries.len());
    for entry in entries {
        let resource: UiResource = serde_json::from_value(entry.clone()).map_err(|e| {
            McpError::protocol(format!(
                "raisin:McpServer `{node_name}` has an invalid ui_resources entry: {e}"
            ))
        })?;
        resources.push(resource);
    }
    Ok(resources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The whole point of `ui_resources`: one widget declared once, referenced
    /// by every tool that renders it. Studio restated a 25-line csp block three
    /// times because there was nowhere else to put it.
    #[test]
    fn a_named_ui_resource_fills_every_tool_that_references_it() {
        let props = json!({
            "slug": "studio",
            "ui_resources": [{
                "id": "panel",
                "workspace": "assets",
                "entry": "/mcp-widgets/studio/index.html",
                "prefersBorder": true,
                "baseHref": true,
                "csp": { "frameDomains": ["https://site.test"] }
            }],
            "tools": [
                { "function": "f1", "name": "preview_page", "ui": { "resource": "panel" } },
                { "function": "f2", "name": "show_stories", "ui": { "resource": "panel" } }
            ]
        });
        let descriptor = McpServerDescriptor::from_properties("studio", &props).unwrap();
        for tool in &descriptor.custom_tools {
            let ui = tool.ui.as_ref().expect("binding kept");
            assert_eq!(ui.entry, "/mcp-widgets/studio/index.html");
            assert_eq!(ui.workspace.as_deref(), Some("assets"));
            assert_eq!(ui.prefers_border, Some(true));
            assert!(ui.wants_base_href());
            assert_eq!(
                ui.csp.as_ref().unwrap().frame_domains,
                vec!["https://site.test".to_string()]
            );
        }
        // Same document, so the two resolve to one resource with one policy —
        // which is exactly what `warn_on_ui_resource_conflicts` checks for.
        let facets: Vec<_> = descriptor
            .custom_tools
            .iter()
            .map(|t| t.ui.as_ref().unwrap().resource_facet())
            .collect();
        assert_eq!(facets[0], facets[1]);
    }

    /// Inheritance is one-directional: a tool may narrow a shared widget
    /// without restating the rest of it.
    #[test]
    fn an_inline_field_overrides_the_named_resource() {
        let props = json!({
            "slug": "s",
            "ui_resources": [{
                "id": "panel",
                "entry": "/w/index.html",
                "workspace": "assets",
                "prefersBorder": true
            }],
            "tools": [{
                "function": "f", "name": "t",
                "ui": { "resource": "panel", "prefersBorder": false }
            }]
        });
        let descriptor = McpServerDescriptor::from_properties("s", &props).unwrap();
        let ui = descriptor.custom_tools[0].ui.as_ref().unwrap();
        assert_eq!(ui.prefers_border, Some(false));
        // …and inherits everything it did not override.
        assert_eq!(ui.entry, "/w/index.html");
        assert_eq!(ui.workspace.as_deref(), Some("assets"));
    }

    /// A typo in `resource` must not take the server down — a failed descriptor
    /// parse cascades through `assemble_registry` and kills every tool, not
    /// just the mistyped one. The tool survives, without a widget.
    #[test]
    fn a_dangling_resource_reference_drops_the_widget_not_the_server() {
        let props = json!({
            "slug": "s",
            "ui_resources": [{ "id": "panel", "entry": "/w/index.html" }],
            "tools": [{ "function": "f", "name": "t", "ui": { "resource": "pannel" } }]
        });
        let descriptor = McpServerDescriptor::from_properties("s", &props).unwrap();
        assert_eq!(
            descriptor.custom_tools.len(),
            1,
            "tool must remain callable"
        );
        assert!(
            descriptor.custom_tools[0].ui.is_none(),
            "an unresolvable binding would mint `ui://ws/` and 404 on read \
             while still advertising a widget"
        );
    }

    /// `entry` became optional so a binding may carry only `resource`. A
    /// binding with neither is not servable and must not be advertised.
    #[test]
    fn a_binding_with_neither_entry_nor_resource_is_dropped() {
        let props = json!({
            "slug": "s",
            "tools": [{ "function": "f", "name": "t", "ui": { "prefersBorder": true } }]
        });
        let descriptor = McpServerDescriptor::from_properties("s", &props).unwrap();
        assert!(descriptor.custom_tools[0].ui.is_none());
    }

    /// A binding that spells the widget out inline keeps working untouched —
    /// every deployed node is this shape.
    #[test]
    fn an_inline_binding_needs_no_ui_resources_declaration() {
        let props = json!({
            "slug": "s",
            "tools": [{
                "function": "f", "name": "t",
                "ui": { "mode": "uri-list", "entry": "/w/index.html", "workspace": "assets" }
            }]
        });
        let descriptor = McpServerDescriptor::from_properties("s", &props).unwrap();
        let ui = descriptor.custom_tools[0].ui.as_ref().unwrap();
        assert_eq!(ui.entry, "/w/index.html");
        assert!(
            ui.wants_base_href(),
            "legacy same-origin widget keeps its base"
        );
    }
}
