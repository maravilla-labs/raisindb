// SPDX-License-Identifier: BSL-1.1

//! Types for hybrid search: query parameters, results, and responses.

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

/// Hybrid search request parameters.
#[derive(Debug, Deserialize)]
pub struct HybridSearchQuery {
    /// Text query for fulltext search
    #[serde(default)]
    pub q: String,

    /// Optional vector query (comma-separated floats or base64-encoded)
    #[serde(default)]
    pub vector: Option<String>,

    /// Search strategy: "fulltext", "vector", "hybrid" (default: "hybrid")
    #[serde(default = "default_strategy")]
    pub strategy: String,

    /// Number of results to return (default: 10)
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// RRF k parameter (default: 60)
    #[serde(default = "default_rrf_k")]
    pub k: f32,

    /// Workspace to search in (default: "default")
    #[serde(default = "default_workspace")]
    pub workspace: String,

    /// Branch to search in (default: "main")
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_strategy() -> String {
    "hybrid".to_string()
}

fn default_limit() -> usize {
    10
}

fn default_rrf_k() -> f32 {
    60.0
}

fn default_workspace() -> String {
    "default".to_string()
}

fn default_branch() -> String {
    "main".to_string()
}

/// Hybrid search result.
#[derive(Debug, Serialize)]
pub struct HybridSearchResult {
    /// Node ID
    pub node_id: String,

    /// Node name
    pub name: String,

    /// Node type
    pub node_type: String,

    /// Node path
    pub path: String,

    /// Workspace ID
    pub workspace_id: String,

    /// Combined RRF score (higher is better)
    pub score: f32,

    /// Fulltext rank (if available)
    pub fulltext_rank: Option<usize>,

    /// Vector distance (if available, lower is better)
    pub vector_distance: Option<f32>,

    /// Revision number
    pub revision: HLC,
}

/// Hybrid search response.
#[derive(Debug, Serialize)]
pub struct HybridSearchResponse {
    /// Search results
    pub results: Vec<HybridSearchResult>,

    /// Number of results
    pub count: usize,

    /// Strategy used
    pub strategy: String,

    /// Fulltext results count
    pub fulltext_count: usize,

    /// Vector results count
    pub vector_count: usize,
}
