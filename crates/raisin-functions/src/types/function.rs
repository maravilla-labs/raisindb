// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function definition types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{NetworkPolicy, ResourceLimits, TriggerCondition};

/// Supported function runtime languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionLanguage {
    /// JavaScript runtime (QuickJS)
    JavaScript,
    /// Python-like runtime (Starlark)
    /// Also accepts "python" in YAML for user convenience
    #[serde(alias = "python")]
    Starlark,
    /// Native SQL passthrough
    Sql,
}

impl Default for FunctionLanguage {
    fn default() -> Self {
        Self::JavaScript
    }
}

impl std::fmt::Display for FunctionLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JavaScript => write!(f, "javascript"),
            Self::Starlark => write!(f, "starlark"),
            Self::Sql => write!(f, "sql"),
        }
    }
}

impl std::str::FromStr for FunctionLanguage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "javascript" | "js" => Ok(Self::JavaScript),
            "starlark" | "python" => Ok(Self::Starlark),
            "sql" => Ok(Self::Sql),
            _ => Err(format!("Unknown function language: {}", s)),
        }
    }
}

/// Function execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Always run via job queue (default)
    Async,
    /// Can run synchronously (for WebAPI with timeout)
    Sync,
    /// Both modes supported - caller decides
    Both,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Async
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Async => write!(f, "async"),
            Self::Sync => write!(f, "sync"),
            Self::Both => write!(f, "both"),
        }
    }
}

/// Function metadata stored in raisin:Function node properties
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionMetadata {
    /// Unique function name (used in SQL calls, REST paths)
    pub name: String,

    /// Human-readable title
    pub title: String,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Runtime language
    pub language: FunctionLanguage,

    /// Execution mode
    #[serde(default)]
    pub execution_mode: ExecutionMode,

    /// Function version (incremented on updates)
    #[serde(default = "default_version")]
    pub version: u32,

    /// Whether function is active
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Entry file in format 'filename:function' (e.g., 'index.js:handler')
    /// For backward compatibility, also accepts just function name (e.g., 'handler')
    #[serde(default = "default_entry_file", alias = "entrypoint")]
    pub entry_file: String,

    /// Resource limits for execution
    #[serde(default)]
    pub resource_limits: ResourceLimits,

    /// Network access policy
    #[serde(default)]
    pub network_policy: NetworkPolicy,

    /// Inline triggers (events that invoke this function)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggerCondition>,

    /// Input parameter schema (JSON Schema)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,

    /// Output schema (JSON Schema)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,

    /// Custom metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_entry_file() -> String {
    "index.js:handler".to_string()
}

impl FunctionMetadata {
    /// Create a new function metadata with minimal required fields
    pub fn new(name: impl Into<String>, language: FunctionLanguage) -> Self {
        let name = name.into();
        Self {
            title: name.clone(),
            name,
            description: None,
            language,
            execution_mode: ExecutionMode::default(),
            version: 1,
            enabled: true,
            entry_file: "index.js:handler".to_string(),
            resource_limits: ResourceLimits::default(),
            network_policy: NetworkPolicy::default(),
            triggers: Vec::new(),
            input_schema: None,
            output_schema: None,
            metadata: HashMap::new(),
        }
    }

    /// Create a JavaScript function
    pub fn javascript(name: impl Into<String>) -> Self {
        Self::new(name, FunctionLanguage::JavaScript)
    }

    /// Create a Starlark function
    pub fn starlark(name: impl Into<String>) -> Self {
        Self::new(name, FunctionLanguage::Starlark)
    }

    /// Create a SQL function
    pub fn sql(name: impl Into<String>) -> Self {
        Self::new(name, FunctionLanguage::Sql)
    }

    /// Set description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set execution mode
    pub fn with_execution_mode(mut self, mode: ExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Set entry file (format: 'filename:function' e.g., 'index.js:handler')
    pub fn with_entry_file(mut self, entry_file: impl Into<String>) -> Self {
        self.entry_file = entry_file.into();
        self
    }

    /// Get the file path component from entry_file
    /// e.g., "index.js:handler" -> "index.js", "handler" -> "index.js" (backward compat)
    pub fn entry_file_path(&self) -> &str {
        if let Some((path, _)) = self.entry_file.rsplit_once(':') {
            path
        } else {
            // Backward compatibility: if no colon, assume it's just a function name
            // and default to "code" (old hardcoded asset name) or "index.js"
            "index.js"
        }
    }

    /// Get the function name component from entry_file
    /// e.g., "index.js:handler" -> "handler", "handler" -> "handler" (backward compat)
    pub fn entry_function_name(&self) -> &str {
        if let Some((_, func)) = self.entry_file.rsplit_once(':') {
            func
        } else {
            // Backward compatibility: if no colon, the whole string is the function name
            &self.entry_file
        }
    }

    /// Set resource limits
    pub fn with_resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.resource_limits = limits;
        self
    }

    /// Set network policy
    pub fn with_network_policy(mut self, policy: NetworkPolicy) -> Self {
        self.network_policy = policy;
        self
    }

    /// Add an inline trigger
    pub fn with_trigger(mut self, trigger: TriggerCondition) -> Self {
        self.triggers.push(trigger);
        self
    }

    /// Check if sync execution is allowed
    pub fn allows_sync(&self) -> bool {
        matches!(
            self.execution_mode,
            ExecutionMode::Sync | ExecutionMode::Both
        )
    }
}

/// A loaded function ready for execution
#[derive(Debug, Clone)]
pub struct LoadedFunction {
    /// Function metadata
    pub metadata: FunctionMetadata,

    /// Entry file source code (the main file specified in entry_file)
    pub code: String,

    /// All function files (path -> content) for module resolution
    /// Includes the entry file and any additional files in the function directory
    pub files: HashMap<String, String>,

    /// Path to the function node in the content tree
    pub path: String,

    /// Node ID of the function
    pub node_id: String,

    /// Workspace containing the function
    pub workspace: String,
}

impl LoadedFunction {
    /// Create a new loaded function
    pub fn new(
        metadata: FunctionMetadata,
        code: String,
        path: String,
        node_id: String,
        workspace: String,
    ) -> Self {
        Self {
            metadata,
            code,
            files: HashMap::new(),
            path,
            node_id,
            workspace,
        }
    }

    /// Create a loaded function with file map for module support
    pub fn with_files(
        metadata: FunctionMetadata,
        code: String,
        files: HashMap<String, String>,
        path: String,
        node_id: String,
        workspace: String,
    ) -> Self {
        Self {
            metadata,
            code,
            files,
            path,
            node_id,
            workspace,
        }
    }
}
