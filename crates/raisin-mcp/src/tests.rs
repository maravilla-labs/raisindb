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

//! Unit tests for scope gating, registry assembly, and data-tool descriptors.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;

use raisin_functions::api::MockFunctionApi;
use raisin_functions::FunctionApi;

use crate::identity::McpIdentity;
use crate::registry::{assemble_registry, AssemblyServices, ToolKind};
use crate::server::{CustomTool, DataOperation, DataPolicy, McpServerDescriptor};

fn backend() -> Arc<dyn FunctionApi> {
    Arc::new(MockFunctionApi::new(json!({})))
}

fn descriptor_with(
    operations: Vec<DataOperation>,
    custom_tools: Vec<CustomTool>,
    scopes: Vec<String>,
) -> McpServerDescriptor {
    McpServerDescriptor {
        name: "Test Server".to_string(),
        version: "1.0.0".to_string(),
        slug: "test".to_string(),
        instructions: None,
        public: false,
        scopes,
        data_policy: DataPolicy {
            workspaces: vec!["content".to_string()],
            operations,
            resources: false,
        },
        custom_tools,
    }
}

// ---------------------------------------------------------------------------
// Scope gating
// ---------------------------------------------------------------------------

#[test]
fn identity_has_scopes_requires_all() {
    let identity = McpIdentity::new("alice", "repo").with_scopes(["read", "write"]);
    assert!(identity.has_scopes(&["read"]));
    assert!(identity.has_scopes(&["read", "write"]));
    assert!(!identity.has_scopes(&["read", "admin"]));
    // Empty requirement is always satisfied.
    assert!(identity.has_scopes::<&str>(&[]));
}

#[test]
fn missing_scopes_lists_the_gap() {
    let identity = McpIdentity::new("alice", "repo").with_scopes(["read"]);
    let missing = identity.missing_scopes(&["read", "write", "admin"]);
    assert_eq!(missing, vec!["write".to_string(), "admin".to_string()]);
}

#[test]
fn system_identity_satisfies_any_scope() {
    let identity = McpIdentity::new("svc", "repo").as_system();
    assert!(identity.scopes.is_empty());
    assert!(identity.has_scopes(&["anything", "at", "all"]));
    assert!(identity.missing_scopes(&["x"]).is_empty());
}

#[test]
fn anonymous_identity_holds_no_scopes() {
    let identity = McpIdentity::anonymous("repo");
    assert!(identity.is_anonymous());
    assert!(!identity.has_scopes(&["read"]));
}

// ---------------------------------------------------------------------------
// Registry assembly
// ---------------------------------------------------------------------------

#[test]
fn assembly_emits_one_tool_per_enabled_operation() {
    let descriptor = descriptor_with(
        vec![DataOperation::GetNode, DataOperation::QueryNodes],
        vec![],
        vec![],
    );
    let services = AssemblyServices {
        backend: backend(),
        search: None,
        functions: None,
    };

    let registry = assemble_registry(&descriptor, &services).expect("assemble");
    let names: BTreeSet<String> = registry.descriptors().into_iter().map(|d| d.name).collect();

    assert_eq!(registry.len(), 2);
    assert!(names.contains("get_node"));
    assert!(names.contains("query_nodes"));
    // Disabled operations are absent.
    assert!(!names.contains("delete_node"));
}

#[test]
fn assembly_skips_search_without_provider() {
    let descriptor = descriptor_with(vec![DataOperation::SearchNodes], vec![], vec![]);
    let services = AssemblyServices {
        backend: backend(),
        search: None,
        functions: None,
    };

    let registry = assemble_registry(&descriptor, &services).expect("assemble");
    // search_nodes needs a SearchProvider; without one, no tool is produced.
    assert!(registry.is_empty());
}

#[test]
fn assembly_skips_custom_tools_without_invoker() {
    let custom = CustomTool {
        name: "greet".to_string(),
        description: "Greet".to_string(),
        function: "greet_fn".to_string(),
        input_schema: json!({ "type": "object" }),
        scopes: vec!["greet".to_string()],
    };
    let descriptor = descriptor_with(vec![], vec![custom], vec![]);
    let services = AssemblyServices {
        backend: backend(),
        search: None,
        functions: None,
    };

    let registry = assemble_registry(&descriptor, &services).expect("assemble");
    assert!(registry.is_empty());
}

#[test]
fn visible_descriptors_filter_by_scope() {
    // Two data tools, no per-tool scopes, but an identity with limited scopes
    // still sees them (data tools carry no scope requirement by default).
    let descriptor = descriptor_with(
        vec![DataOperation::GetNode, DataOperation::ListWorkspaces],
        vec![],
        vec![],
    );
    let services = AssemblyServices {
        backend: backend(),
        search: None,
        functions: None,
    };
    let registry = assemble_registry(&descriptor, &services).expect("assemble");

    let identity = McpIdentity::anonymous("repo");
    assert_eq!(registry.visible_descriptors(&identity).len(), 2);
}

// ---------------------------------------------------------------------------
// Data-tool descriptor generation
// ---------------------------------------------------------------------------

#[test]
fn data_tool_descriptors_are_well_formed() {
    let descriptor = descriptor_with(DataOperation::ALL.to_vec(), vec![], vec![]);
    let services = AssemblyServices {
        backend: backend(),
        search: None,
        functions: None,
    };
    let registry = assemble_registry(&descriptor, &services).expect("assemble");

    for d in registry.descriptors() {
        assert_eq!(d.kind, ToolKind::Data, "data tool `{}`", d.name);
        // inputSchema must be an object schema.
        assert_eq!(
            d.input_schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "tool `{}` schema",
            d.name
        );
        assert!(!d.description.is_empty(), "tool `{}` description", d.name);
    }
}

#[test]
fn create_node_descriptor_requires_core_fields() {
    let descriptor = descriptor_with(vec![DataOperation::CreateNode], vec![], vec![]);
    let services = AssemblyServices {
        backend: backend(),
        search: None,
        functions: None,
    };
    let registry = assemble_registry(&descriptor, &services).expect("assemble");
    let create = registry
        .get("create_node")
        .expect("create_node present")
        .descriptor();

    let required: Vec<&str> = create
        .input_schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"parent_path"));
    assert!(required.contains(&"name"));
    assert!(required.contains(&"node_type"));
}

#[test]
fn descriptor_parses_from_node_properties() {
    let props = json!({
        "name": "Assistant",
        "slug": "assistant",
        "version": "2.1.0",
        "public": false,
        "scopes": ["mcp.use"],
        "data": {
            "workspaces": ["content"],
            "operations": ["get_node", "query_nodes", "search_nodes"],
            "resources": true
        },
        "tools": [
            {
                "name": "summarize",
                "description": "Summarize a node",
                "function": "summarize_fn",
                "inputSchema": { "type": "object" },
                "scopes": ["mcp.summarize"]
            }
        ]
    });

    let parsed = McpServerDescriptor::from_properties("node-name", &props).expect("parse");
    assert_eq!(parsed.slug, "assistant");
    assert_eq!(parsed.version, "2.1.0");
    assert_eq!(parsed.scopes, vec!["mcp.use".to_string()]);
    assert!(parsed.data_policy.allows(DataOperation::SearchNodes));
    assert!(parsed.data_policy.resources);
    assert_eq!(parsed.custom_tools.len(), 1);
    assert_eq!(parsed.custom_tools[0].function, "summarize_fn");
}

#[test]
fn descriptor_without_slug_is_rejected() {
    let props = json!({ "name": "No Slug" });
    let err = McpServerDescriptor::from_properties("n", &props).unwrap_err();
    assert!(err.to_string().contains("slug"), "got: {err}");
}

#[test]
fn function_side_mcp_block_promotes_to_tool() {
    // A raisin:Function with an `mcp` block, defaulting fields to function metadata.
    let props = json!({
        "name": "weather",
        "description": "Look up the weather",
        "input_schema": { "type": "object", "properties": { "city": { "type": "string" } } },
        "mcp": { "enabled": true, "scopes": ["mcp.weather"] }
    });
    let tool = CustomTool::from_function_properties(&props).expect("promoted");
    assert_eq!(tool.name, "weather");
    assert_eq!(tool.function, "weather");
    assert_eq!(tool.description, "Look up the weather");
    assert_eq!(tool.scopes, vec!["mcp.weather".to_string()]);
    assert_eq!(tool.input_schema["properties"]["city"]["type"], "string");
}

#[test]
fn function_side_mcp_block_honors_overrides_and_disable() {
    // Explicit overrides on the `mcp` block win over function metadata.
    let props = json!({
        "name": "raw_fn",
        "description": "internal",
        "mcp": { "name": "nice_tool", "description": "A nicer tool" }
    });
    let tool = CustomTool::from_function_properties(&props).expect("promoted");
    assert_eq!(tool.name, "nice_tool");
    assert_eq!(tool.function, "raw_fn");
    assert_eq!(tool.description, "A nicer tool");

    // `enabled: false` opts out.
    let disabled = json!({ "name": "x", "mcp": { "enabled": false } });
    assert!(CustomTool::from_function_properties(&disabled).is_none());

    // No `mcp` block at all -> not a tool.
    let plain = json!({ "name": "y" });
    assert!(CustomTool::from_function_properties(&plain).is_none());
}
