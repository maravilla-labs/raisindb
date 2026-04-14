//! Graph computation target and scope configuration types

use serde::{Deserialize, Serialize};

/// Target mode for graph algorithm computation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetMode {
    /// Compute for specific branches, tracking HEAD
    Branch,
    /// Compute for all branches, tracking each HEAD
    AllBranches,
    /// Compute for specific revisions (immutable)
    Revision,
    /// Compute for branches matching a glob pattern
    BranchPattern,
}

/// Graph computation target configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTarget {
    pub mode: TargetMode,
    /// Branch IDs (for mode=branch)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<String>,
    /// Revision IDs (for mode=revision)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<String>,
    /// Branch pattern glob (for mode=branch_pattern)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_pattern: Option<String>,
}

/// Scope configuration for filtering nodes in graph computation
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphScope {
    /// Path patterns (glob syntax, e.g., "social/users/**")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Node types to include (e.g., "raisin:User")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_types: Vec<String>,
    /// Workspaces to include
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    /// Relation types to filter by (only include nodes connected via these)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relation_types: Vec<String>,
}

/// Refresh trigger configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RefreshConfig {
    /// TTL in seconds before recomputation
    #[serde(default)]
    pub ttl_seconds: u64,
    /// Recompute when branch HEAD changes
    #[serde(default)]
    pub on_branch_change: bool,
    /// Recompute when relations change within scope
    #[serde(default)]
    pub on_relation_change: bool,
    /// Optional cron schedule (e.g., "0 */6 * * *")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
}
