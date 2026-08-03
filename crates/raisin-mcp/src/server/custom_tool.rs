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

//! Author-declared tools, and the `raisin:Function` metadata they inherit.
//!
//! A [`CustomTool`] may be declared from either side: on the server node's
//! `tools` array, or as an `mcp` block on the function itself. Either way the
//! function is the source of truth for `description`, `inputSchema` and
//! `outputSchema` — [`CustomTool::fill_defaults_from`] copies whatever the tool
//! left unset, so a schema lives in ONE place and cannot drift.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::UiBinding;

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
