//! Graph cache response and event types.

use serde::{Deserialize, Serialize};

/// Events sent via SSE for live updates
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GraphCacheEvent {
    /// Countdown to next automatic tick
    TickCountdown {
        next_tick_at: u64,
        seconds_remaining: u64,
    },
    /// Computation started for a config
    ComputationStarted {
        config_id: String,
        algorithm: String,
        node_count: Option<u64>,
    },
    /// Progress update during computation
    ComputationProgress {
        config_id: String,
        progress_pct: u8,
        current_step: String,
    },
    /// Computation completed successfully
    ComputationCompleted {
        config_id: String,
        duration_ms: u64,
        node_count: u64,
    },
    /// Computation failed
    ComputationFailed { config_id: String, error: String },
    /// Status changed for a config
    StatusChanged {
        config_id: String,
        old_status: String,
        new_status: String,
    },
}

/// Response for config status listing
#[derive(Debug, Serialize)]
pub struct ConfigStatusResponse {
    pub configs: Vec<ConfigStatus>,
    pub next_tick_at: u64,
    pub tick_interval_seconds: u64,
}

/// Status of a single graph algorithm config
#[derive(Debug, Serialize)]
pub struct ConfigStatus {
    pub id: String,
    pub algorithm: String,
    pub enabled: bool,
    pub status: String,
    pub last_computed_at: Option<u64>,
    pub next_scheduled_at: Option<u64>,
    pub node_count: Option<u64>,
    pub error: Option<String>,
    /// Full configuration details
    pub config: GraphAlgorithmConfigResponse,
}

/// Serializable representation of GraphAlgorithmConfig
#[derive(Debug, Serialize, Deserialize)]
pub struct GraphAlgorithmConfigResponse {
    pub id: String,
    pub algorithm: String,
    pub enabled: bool,
    pub target: TargetConfigResponse,
    pub scope: ScopeConfigResponse,
    pub algorithm_config: serde_json::Value,
    pub refresh: RefreshConfigResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TargetConfigResponse {
    pub mode: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub branches: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub revisions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_pattern: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScopeConfigResponse {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub node_types: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub workspaces: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub relation_types: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshConfigResponse {
    pub ttl_seconds: u64,
    pub on_branch_change: bool,
    pub on_relation_change: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
}

/// API response wrapper
#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error.into()),
        }
    }
}
