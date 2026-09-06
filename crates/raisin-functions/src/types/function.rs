// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function definition types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{
    EmailPolicy, FunctionCode, IdentityPolicy, NetworkPolicy, ResourceLimits, SecretPolicy,
    TriggerCondition,
};

/// Supported function runtime languages
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionLanguage {
    /// JavaScript runtime (QuickJS)
    #[default]
    JavaScript,
    /// Python-like runtime (Starlark)
    /// Also accepts "python" in YAML for user convenience
    #[serde(alias = "python")]
    Starlark,
    /// Native SQL passthrough
    Sql,
    /// WebAssembly component (Component Model + WIT), run by wasmtime.
    ///
    /// A wasm function has no source on the server: its code is the uploaded
    /// artifact, which is why [`crate::types::FunctionCode`] has a `Bytes`
    /// shape at all.
    #[serde(alias = "wasm-component")]
    Wasm,
}

impl std::fmt::Display for FunctionLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JavaScript => write!(f, "javascript"),
            Self::Starlark => write!(f, "starlark"),
            Self::Sql => write!(f, "sql"),
            Self::Wasm => write!(f, "wasm"),
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
            "wasm" | "wasm-component" => Ok(Self::Wasm),
            _ => Err(format!("Unknown function language: {}", s)),
        }
    }
}

/// Function execution mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Always run via job queue (default)
    #[default]
    Async,
    /// Can run synchronously (for WebAPI with timeout)
    Sync,
    /// Both modes supported - caller decides
    Both,
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

impl ExecutionMode {
    /// Whether a caller may invoke the function synchronously.
    ///
    /// The one definition of that question. `invoke_function` used to ask it
    /// inverted (`mode == Async`), which was the same answer by accident — a
    /// third variant that could not run inline would have had to be remembered
    /// in two places.
    pub fn allows_sync(&self) -> bool {
        matches!(self, ExecutionMode::Sync | ExecutionMode::Both)
    }
}

/// Parse the `execution_mode` property of a `raisin:Function` node.
///
/// CASE-INSENSITIVE, and that is the whole point of having it. The property is
/// hand-written YAML and the shipped nodes spell it both ways — `Async` in most
/// built-in packages, `"async"` in others — while the parser this replaces
/// matched lowercase only and fell through to `Async` for everything else.
///
/// That fall-through is the dangerous shape: `execution_mode: Sync` parsed as
/// `Async`, so four built-in functions that declared themselves synchronous
/// were refused synchronous invocation with "does not support synchronous
/// execution" — an error that names the function rather than the typo, and sends
/// the reader looking at the caller. An unknown value must not silently become
/// the most restrictive mode.
///
/// `inline` is accepted as a synonym for `sync`: four `ai-tools` functions use
/// it, and an agent tool whose result is consumed by the caller is inline by
/// definition.
impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "async" => Ok(Self::Async),
            "sync" | "inline" => Ok(Self::Sync),
            "both" => Ok(Self::Both),
            other => Err(format!("Unknown execution mode: {other}")),
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

    /// Secret access policy for `raisin.secrets.*`.
    ///
    /// Absent means **no secret access** — see [`SecretPolicy`] for why the
    /// default has to be the denying one.
    #[serde(default)]
    pub secret_policy: SecretPolicy,

    /// Outbound-email policy for `raisin.email.*`.
    ///
    /// Absent means **no sending at all** — see [`EmailPolicy`] for why the
    /// default has to be the denying one.
    #[serde(default)]
    pub email_policy: EmailPolicy,

    /// Tenant-identity policy for `raisin.identities.*`.
    ///
    /// Absent means **no identity access** — see [`IdentityPolicy`] for why
    /// the default has to be the denying one.
    #[serde(default)]
    pub identity_policy: IdentityPolicy,

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
            secret_policy: SecretPolicy::default(),
            email_policy: EmailPolicy::default(),
            identity_policy: IdentityPolicy::default(),
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

    /// Create a WebAssembly component function
    pub fn wasm(name: impl Into<String>) -> Self {
        Self::new(name, FunctionLanguage::Wasm)
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

    /// Set secret policy
    pub fn with_secret_policy(mut self, policy: SecretPolicy) -> Self {
        self.secret_policy = policy;
        self
    }

    /// Set email policy
    pub fn with_email_policy(mut self, policy: EmailPolicy) -> Self {
        self.email_policy = policy;
        self
    }

    /// Set identity policy
    pub fn with_identity_policy(mut self, policy: IdentityPolicy) -> Self {
        self.identity_policy = policy;
        self
    }

    /// Add an inline trigger
    pub fn with_trigger(mut self, trigger: TriggerCondition) -> Self {
        self.triggers.push(trigger);
        self
    }

    /// Check if sync execution is allowed. Delegates, so there is one answer.
    pub fn allows_sync(&self) -> bool {
        self.execution_mode.allows_sync()
    }
}

/// A loaded function ready for execution
#[derive(Debug, Clone)]
pub struct LoadedFunction {
    /// Function metadata
    pub metadata: FunctionMetadata,

    /// Entry file payload — source text, or a binary artifact for `wasm`
    /// (the main file specified in entry_file)
    pub code: FunctionCode,

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
        code: impl Into<FunctionCode>,
        path: String,
        node_id: String,
        workspace: String,
    ) -> Self {
        Self {
            metadata,
            code: code.into(),
            files: HashMap::new(),
            path,
            node_id,
            workspace,
        }
    }

    /// Create a loaded function with file map for module support
    pub fn with_files(
        metadata: FunctionMetadata,
        code: impl Into<FunctionCode>,
        files: HashMap<String, String>,
        path: String,
        node_id: String,
        workspace: String,
    ) -> Self {
        Self {
            metadata,
            code: code.into(),
            files,
            path,
            node_id,
            workspace,
        }
    }
}

#[cfg(test)]
mod execution_mode_tests {
    use super::*;

    /// The shipped `.node.yaml` files spell this both ways — `Async` in most
    /// built-in packages, `"async"` in others — so a case-sensitive parser
    /// reads half of them wrong.
    #[test]
    fn parsing_is_case_insensitive() {
        for (raw, expected) in [
            ("async", ExecutionMode::Async),
            ("Async", ExecutionMode::Async),
            ("ASYNC", ExecutionMode::Async),
            ("sync", ExecutionMode::Sync),
            ("Sync", ExecutionMode::Sync),
            ("  Sync  ", ExecutionMode::Sync),
            ("both", ExecutionMode::Both),
            ("Both", ExecutionMode::Both),
            // `ai-tools` spells a synchronous tool this way.
            ("inline", ExecutionMode::Sync),
            ("Inline", ExecutionMode::Sync),
        ] {
            assert_eq!(
                raw.parse::<ExecutionMode>().expect("parses"),
                expected,
                "{raw} should parse as {expected}"
            );
        }
    }

    /// An unknown value is an ERROR, not a silent demotion to the most
    /// restrictive mode. The caller decides what to do with it (the HTTP layer
    /// logs and defaults); swallowing it here is how `Sync` became `Async`.
    #[test]
    fn an_unknown_value_does_not_silently_become_async() {
        assert!("whenever".parse::<ExecutionMode>().is_err());
        assert!("".parse::<ExecutionMode>().is_err());
    }

    /// The one definition of "may this be invoked synchronously".
    #[test]
    fn only_sync_and_both_allow_a_synchronous_invoke() {
        assert!(ExecutionMode::Sync.allows_sync());
        assert!(ExecutionMode::Both.allows_sync());
        assert!(!ExecutionMode::Async.allows_sync());
    }

    /// Display and FromStr must round-trip, or a value written back by the
    /// server would not parse on the way in again.
    #[test]
    fn display_round_trips_through_from_str() {
        for mode in [
            ExecutionMode::Async,
            ExecutionMode::Sync,
            ExecutionMode::Both,
        ] {
            assert_eq!(mode.to_string().parse::<ExecutionMode>().unwrap(), mode);
        }
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    /// Every variant, its wire spelling, and the aliases `FromStr` accepts.
    ///
    /// `language` is written by hand in `.node.yaml` files and by three
    /// different clients, so the spellings are a contract; the serde
    /// representation and `FromStr` have to agree about all of them or a
    /// function that installs cannot be loaded.
    #[test]
    fn language_spellings_round_trip() {
        for (variant, wire, aliases) in [
            (
                FunctionLanguage::JavaScript,
                "javascript",
                &["js", "JavaScript"][..],
            ),
            (FunctionLanguage::Starlark, "starlark", &["python"][..]),
            (FunctionLanguage::Sql, "sql", &["SQL"][..]),
            (
                FunctionLanguage::Wasm,
                "wasm",
                &["wasm-component", "WASM"][..],
            ),
        ] {
            assert_eq!(variant.to_string(), wire, "Display for {variant:?}");
            assert_eq!(wire.parse::<FunctionLanguage>().unwrap(), variant);
            for alias in aliases {
                assert_eq!(
                    alias.parse::<FunctionLanguage>().unwrap(),
                    variant,
                    "FromStr for alias {alias}"
                );
            }
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::Value::String(wire.to_string())
            );
            assert_eq!(
                serde_json::from_value::<FunctionLanguage>(serde_json::json!(wire)).unwrap(),
                variant
            );
        }

        // The serde alias exists so a node written before the name settled
        // still deserialises.
        assert_eq!(
            serde_json::from_value::<FunctionLanguage>(serde_json::json!("wasm-component"))
                .unwrap(),
            FunctionLanguage::Wasm
        );
        assert!("brainfuck".parse::<FunctionLanguage>().is_err());
    }

    #[test]
    fn a_wasm_function_declares_its_language() {
        let metadata = FunctionMetadata::wasm("greet");
        assert_eq!(metadata.language, FunctionLanguage::Wasm);
        assert_eq!(metadata.name, "greet");
    }
}
