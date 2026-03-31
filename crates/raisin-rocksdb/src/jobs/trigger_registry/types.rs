//! Trigger type definitions

use serde_json::Value as JsonValue;

/// Cached trigger for quick matching
///
/// This is a lightweight representation of a trigger optimized for
/// in-memory filtering and index lookups.
#[derive(Debug, Clone)]
pub struct CachedTrigger {
    /// Unique identifier for this trigger
    pub id: String,
    /// Path to the function to execute (for single-function triggers)
    pub function_path: Option<String>,
    /// Name of the trigger
    pub trigger_name: String,
    /// Path to the trigger node (for standalone raisin:Trigger nodes)
    pub trigger_path: Option<String>,
    /// Priority (higher executes first)
    pub priority: i32,
    /// Whether the trigger is currently enabled
    pub enabled: bool,
    /// Event kinds this trigger subscribes to (e.g., "Created", "Updated")
    pub event_kinds: Vec<String>,
    /// Filters for matching events
    pub filters: TriggerFilters,
    /// Maximum retry attempts on failure
    pub max_retries: Option<u32>,
    /// Workflow data from referenced raisin:Flow node
    pub workflow_data: Option<JsonValue>,
}

/// Filters for trigger matching
#[derive(Debug, Clone, Default)]
pub struct TriggerFilters {
    /// Workspace filter (None = matches all)
    pub workspaces: Option<Vec<String>>,
    /// Node type filter (None = matches all)
    pub node_types: Option<Vec<String>>,
    /// Path glob patterns (None = matches all)
    pub paths: Option<Vec<String>>,
    /// Property value filters
    pub property_filters: Option<serde_json::Map<String, JsonValue>>,
}
