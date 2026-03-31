// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Trigger definition types for function invocation

use serde::{Deserialize, Serialize};

use super::FunctionFlow;

/// Trigger condition that invokes a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerCondition {
    /// Unique trigger name within function
    pub name: String,

    /// Trigger type configuration
    pub trigger_type: TriggerType,

    /// Whether trigger is active
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Filter conditions to narrow trigger scope
    #[serde(default)]
    pub filters: TriggerFilters,

    /// Execution priority (lower = higher priority)
    #[serde(default)]
    pub priority: i32,
}

fn default_true() -> bool {
    true
}

impl TriggerCondition {
    /// Create a new trigger condition
    pub fn new(name: impl Into<String>, trigger_type: TriggerType) -> Self {
        Self {
            name: name.into(),
            trigger_type,
            enabled: true,
            filters: TriggerFilters::default(),
            priority: 0,
        }
    }

    /// Create a node event trigger
    pub fn node_event(name: impl Into<String>, events: Vec<NodeEventKind>) -> Self {
        Self::new(
            name,
            TriggerType::NodeEvent {
                event_kinds: events,
            },
        )
    }

    /// Create an HTTP trigger
    pub fn http(name: impl Into<String>, methods: Vec<HttpMethod>) -> Self {
        Self::new(
            name,
            TriggerType::Http(HttpTriggerConfig {
                methods,
                route_mode: HttpRouteMode::Config,
                path_pattern: None,
                path_suffix: None,
                default_sync: false,
            }),
        )
    }

    /// Create a scheduled trigger
    pub fn schedule(name: impl Into<String>, cron: impl Into<String>) -> Self {
        Self::new(
            name,
            TriggerType::Schedule {
                cron_expression: cron.into(),
            },
        )
    }

    /// Create a SQL call trigger
    pub fn sql_call(name: impl Into<String>) -> Self {
        Self::new(name, TriggerType::SqlCall)
    }

    /// Set filters
    pub fn with_filters(mut self, filters: TriggerFilters) -> Self {
        self.filters = filters;
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Disable trigger
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// HTTP trigger route matching mode
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRouteMode {
    /// Route matching defined in config (path_pattern)
    #[default]
    Config,
    /// Route matching handled by script logic
    Script,
}

/// HTTP trigger configuration with enhanced routing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTriggerConfig {
    /// Allowed HTTP methods
    pub methods: Vec<HttpMethod>,

    /// Route matching mode
    #[serde(default)]
    pub route_mode: HttpRouteMode,

    /// Path pattern for config-based routing (matchit syntax)
    /// Examples: "/:userId", "/:userId/orders/:orderId", "/{*rest}"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_pattern: Option<String>,

    /// Optional path suffix (deprecated, use path_pattern)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_suffix: Option<String>,

    /// Default execution mode: true = wait for result, false = fire-and-forget
    #[serde(default)]
    pub default_sync: bool,
}

impl Default for HttpTriggerConfig {
    fn default() -> Self {
        Self {
            methods: vec![HttpMethod::POST],
            route_mode: HttpRouteMode::Config,
            path_pattern: None,
            path_suffix: None,
            default_sync: false,
        }
    }
}

/// Types of triggers that can invoke functions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerType {
    /// Triggered by node events (create, update, delete, etc.)
    NodeEvent {
        /// Which event kinds trigger this
        event_kinds: Vec<NodeEventKind>,
    },

    /// Triggered by scheduled cron expression
    Schedule {
        /// Cron expression (e.g., "0 * * * *" for hourly)
        cron_expression: String,
    },

    /// Triggered by HTTP request to function endpoint
    Http(HttpTriggerConfig),

    /// Triggered by SQL function call (raisin.function_name())
    SqlCall,
}

/// Node event kinds that can trigger functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeEventKind {
    /// Node was created
    Created,
    /// Node was updated
    Updated,
    /// Node was deleted
    Deleted,
    /// Node was published
    Published,
    /// Node was unpublished
    Unpublished,
    /// Node was moved
    Moved,
    /// Node was renamed
    Renamed,
}

impl std::fmt::Display for NodeEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Updated => write!(f, "updated"),
            Self::Deleted => write!(f, "deleted"),
            Self::Published => write!(f, "published"),
            Self::Unpublished => write!(f, "unpublished"),
            Self::Moved => write!(f, "moved"),
            Self::Renamed => write!(f, "renamed"),
        }
    }
}

/// HTTP methods for HTTP triggers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GET => write!(f, "GET"),
            Self::POST => write!(f, "POST"),
            Self::PUT => write!(f, "PUT"),
            Self::PATCH => write!(f, "PATCH"),
            Self::DELETE => write!(f, "DELETE"),
        }
    }
}

/// Filters to narrow down trigger scope
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriggerFilters {
    /// Workspace filter (glob patterns)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Vec<String>>,

    /// Path filter (glob patterns like "/content/**")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,

    /// Node type filter (e.g., ["raisin:Page", "raisin:Asset"])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_types: Option<Vec<String>>,

    /// Property value filters (JSON query)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property_filters: Option<serde_json::Value>,
}

impl TriggerFilters {
    /// Create empty filters (matches everything)
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by workspaces
    pub fn with_workspaces(mut self, workspaces: Vec<String>) -> Self {
        self.workspaces = Some(workspaces);
        self
    }

    /// Filter by paths
    pub fn with_paths(mut self, paths: Vec<String>) -> Self {
        self.paths = Some(paths);
        self
    }

    /// Filter by node types
    pub fn with_node_types(mut self, node_types: Vec<String>) -> Self {
        self.node_types = Some(node_types);
        self
    }

    /// Check if a given context matches these filters
    pub fn matches(&self, workspace: &str, path: &str, node_type: &str) -> bool {
        // Check workspace filter
        if let Some(ref workspaces) = self.workspaces {
            let matches = workspaces.iter().any(|pattern| {
                glob::Pattern::new(pattern)
                    .map(|p| p.matches(workspace))
                    .unwrap_or(false)
            });
            if !matches {
                return false;
            }
        }

        // Check path filter
        if let Some(ref paths) = self.paths {
            let matches = paths.iter().any(|pattern| {
                glob::Pattern::new(pattern)
                    .map(|p| p.matches(path))
                    .unwrap_or(false)
            });
            if !matches {
                return false;
            }
        }

        // Check node type filter
        if let Some(ref node_types) = self.node_types {
            if !node_types.contains(&node_type.to_string()) {
                return false;
            }
        }

        true
    }
}

/// Standalone trigger metadata (stored in raisin:Trigger nodes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandaloneTrigger {
    /// Trigger name
    pub name: String,

    /// Human-readable title
    pub title: String,

    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Path to a single function node to invoke (deprecated, use function_flow instead)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_path: Option<String>,

    /// Multi-function execution flow (preferred over function_path)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_flow: Option<FunctionFlow>,

    /// Trigger type and configuration
    pub trigger_type: TriggerType,

    /// Filter conditions
    #[serde(default)]
    pub filters: TriggerFilters,

    /// Whether trigger is active
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Execution priority
    #[serde(default)]
    pub priority: i32,
}

impl StandaloneTrigger {
    /// Create a new standalone trigger with a single function path (legacy)
    pub fn new(
        name: impl Into<String>,
        function_path: impl Into<String>,
        trigger_type: TriggerType,
    ) -> Self {
        let name = name.into();
        Self {
            title: name.clone(),
            name,
            description: None,
            function_path: Some(function_path.into()),
            function_flow: None,
            trigger_type,
            filters: TriggerFilters::default(),
            enabled: true,
            priority: 0,
        }
    }

    /// Create a new standalone trigger with a function flow
    pub fn with_flow(
        name: impl Into<String>,
        flow: FunctionFlow,
        trigger_type: TriggerType,
    ) -> Self {
        let name = name.into();
        Self {
            title: name.clone(),
            name,
            description: None,
            function_path: None,
            function_flow: Some(flow),
            trigger_type,
            filters: TriggerFilters::default(),
            enabled: true,
            priority: 0,
        }
    }

    /// Check if this trigger has a valid target (either function_path or function_flow)
    pub fn has_valid_target(&self) -> bool {
        self.function_path.is_some() || self.function_flow.is_some()
    }

    /// Returns true if this trigger uses a function flow (multi-function)
    pub fn uses_flow(&self) -> bool {
        self.function_flow.is_some()
    }
}
