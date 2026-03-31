// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function loading from storage

use raisin_error::{Error, Result};

use crate::types::{
    FunctionLanguage, FunctionMetadata, LoadedFunction, NetworkPolicy, ResourceLimits,
};

/// Function loader
///
/// Loads functions from RaisinDB node storage.
pub struct FunctionLoader {
    /// Functions workspace name
    workspace: String,
}

impl FunctionLoader {
    /// Create a new function loader
    pub fn new() -> Self {
        Self {
            workspace: "functions".to_string(),
        }
    }

    /// Create with custom workspace name
    pub fn with_workspace(workspace: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// Get the workspace name
    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    /// Load a function from a node
    ///
    /// This is a placeholder - the actual implementation will load from RaisinDB storage.
    ///
    /// # Arguments
    /// * `node` - The raisin:Function node as JSON
    /// * `code_node` - The child raisin:Asset node containing the code
    pub fn load_from_node(
        &self,
        node: &serde_json::Value,
        code_node: &serde_json::Value,
    ) -> Result<LoadedFunction> {
        // Extract metadata from node properties
        let props = node
            .get("properties")
            .ok_or_else(|| Error::Validation("Function node missing properties".to_string()))?;

        let name = props
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Function missing name property".to_string()))?
            .to_string();

        let title = props
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&name)
            .to_string();

        let language_str = props
            .get("language")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Function missing language property".to_string()))?;

        let language: FunctionLanguage = language_str
            .parse()
            .map_err(|e: String| Error::Validation(e))?;

        // Support both new entry_file format and legacy entrypoint
        let entry_file = props
            .get("entry_file")
            .and_then(|v| v.as_str())
            .or_else(|| {
                // Backward compatibility: convert old entrypoint to entry_file format
                props.get("entrypoint").and_then(|v| v.as_str())
            })
            .map(|s| {
                // If it's just a function name (no colon), treat as legacy format
                if s.contains(':') {
                    s.to_string()
                } else {
                    // Legacy: "handler" becomes "index.js:handler" for new functions
                    // or could be preserved as-is for backward compat
                    s.to_string()
                }
            })
            .unwrap_or_else(|| "index.js:handler".to_string());

        let enabled = props
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Parse resource limits
        let resource_limits = props
            .get("resource_limits")
            .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
            .unwrap_or_else(ResourceLimits::default);

        // Parse network policy
        let network_policy = props
            .get("network_policy")
            .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
            .unwrap_or_else(NetworkPolicy::default);

        let metadata = FunctionMetadata {
            name,
            title,
            description: props
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            language,
            execution_mode: props
                .get("execution_mode")
                .and_then(|v| v.as_str())
                .and_then(|s| match s {
                    "async" => Some(crate::types::ExecutionMode::Async),
                    "sync" => Some(crate::types::ExecutionMode::Sync),
                    "both" => Some(crate::types::ExecutionMode::Both),
                    _ => None,
                })
                .unwrap_or_default(),
            version: props.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            enabled,
            entry_file,
            resource_limits,
            network_policy,
            triggers: props
                .get("triggers")
                .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
                .unwrap_or_default(),
            input_schema: props.get("input_schema").cloned(),
            output_schema: props.get("output_schema").cloned(),
            metadata: props
                .get("metadata")
                .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
                .unwrap_or_default(),
        };

        // Extract code from Asset node
        // The code could be in:
        // 1. A "code" text property on the Asset
        // 2. A "file" Resource property with inline content
        let code_props = code_node
            .get("properties")
            .ok_or_else(|| Error::Validation("Code node missing properties".to_string()))?;

        let code = code_props
            .get("code")
            .and_then(|v| v.as_str())
            .or_else(|| {
                code_props
                    .get("file")
                    .and_then(|f| f.get("content"))
                    .and_then(|c| c.as_str())
            })
            .ok_or_else(|| Error::Validation("Code node missing code content".to_string()))?
            .to_string();

        let path = node
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let node_id = node
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let workspace = node
            .get("workspace")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.workspace)
            .to_string();

        Ok(LoadedFunction::new(
            metadata, code, path, node_id, workspace,
        ))
    }

    /// Create a mock function for testing
    pub fn mock_function(name: impl Into<String>, code: impl Into<String>) -> LoadedFunction {
        let name = name.into();
        LoadedFunction::new(
            FunctionMetadata::javascript(&name),
            code.into(),
            format!("/lib/raisin/function/{}", name),
            nanoid::nanoid!(),
            "functions".to_string(),
        )
    }
}

impl Default for FunctionLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_from_node() {
        let loader = FunctionLoader::new();

        let node = serde_json::json!({
            "id": "func-123",
            "path": "/lib/raisin/function/my_function",
            "workspace": "functions",
            "node_type": "raisin:Function",
            "properties": {
                "name": "my_function",
                "title": "My Function",
                "language": "javascript",
                "entry_file": "index.js:handler",
                "enabled": true
            }
        });

        let code_node = serde_json::json!({
            "id": "code-123",
            "path": "/lib/raisin/function/my_function/code",
            "node_type": "raisin:Asset",
            "properties": {
                "title": "Function Code",
                "code": "async function handler(input) { return { success: true }; }"
            }
        });

        let function = loader.load_from_node(&node, &code_node).unwrap();

        assert_eq!(function.metadata.name, "my_function");
        assert_eq!(function.metadata.language, FunctionLanguage::JavaScript);
        assert!(function.code.contains("handler"));
    }
}
