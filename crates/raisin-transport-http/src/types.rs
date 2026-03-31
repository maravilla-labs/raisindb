// SPDX-License-Identifier: BSL-1.1

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Default)]
pub struct PageParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMeta {
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: PageMeta,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepoQuery {
    #[serde(default)]
    pub level: Option<u32>,
    #[serde(default)]
    pub flatten: Option<bool>,
    #[serde(default)]
    pub format: Option<String>, // "array" for DX-friendly format, "map" for HashMap (default)
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub deep: Option<bool>,
    #[serde(default)]
    pub inline: Option<bool>,
    #[serde(default)]
    pub override_existing: Option<bool>,
    #[serde(default)]
    pub revision: Option<u64>, // Optional: view repository at specific revision
    #[serde(default)]
    pub cursor: Option<String>, // Optional: base64-encoded cursor for pagination
    #[serde(default)]
    pub limit: Option<usize>, // Optional: max items per page (default 100, max 1000)
    #[serde(default)]
    pub lang: Option<String>, // Optional: locale code for translations (e.g., "fr", "de", "es")
    #[serde(default)]
    pub commit_message: Option<String>,
    #[serde(default)]
    pub commit_actor: Option<String>,
    #[serde(default)]
    pub node_type: Option<String>, // Hint for auto-creation
    #[serde(default)]
    pub property_path: Option<String>, // Override target property (default: "file")
    #[serde(default)]
    pub new_name: Option<String>, // For one-shot creation under parent
    // Asset command parameters (for signed URLs)
    #[serde(default)]
    pub sig: Option<String>, // HMAC signature for raisin:download/display commands
    #[serde(default)]
    pub exp: Option<u64>, // Expiry timestamp (Unix seconds)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CommandBody {
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub move_position: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub property_path: Option<String>,
    // Version management fields
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub version: Option<i32>,
    #[serde(default)]
    pub keep_count: Option<usize>,
    // Transaction/commit fields
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub operations: Option<Vec<serde_json::Value>>,
    // Relationship fields
    #[serde(default)]
    pub target_workspace: Option<String>,
    #[serde(default)]
    pub weight: Option<f32>,
    #[serde(default)]
    pub relation_type: Option<String>,
    // Translation fields
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub translations: Option<serde_json::Value>,
    #[serde(default)]
    pub pointer: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    #[serde(default)]
    pub node_type: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

/// Commit metadata for GitHub-style commit pattern
/// When present in request body, operation creates a new revision
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    /// Commit message describing the change
    pub message: String,
    /// Actor performing the commit (username, email, or system identifier)
    #[serde(default = "default_actor")]
    pub actor: String,
}

fn default_actor() -> String {
    "system".to_string()
}

/// Request body for single-node commit operations (GitHub-like pattern)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitNodeRequest {
    /// Commit message describing the change
    pub message: String,
    /// Actor performing the commit (username or system)
    #[serde(default = "default_actor")]
    pub actor: String,
    /// Node properties to update (for save operation)
    #[serde(default)]
    pub properties: Option<serde_json::Value>,
    /// Full node data (for create operation)
    #[serde(default)]
    pub node: Option<serde_json::Value>,
}

/// Response from commit operations
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResponse {
    /// The revision number created by this commit
    pub revision: u64,
    /// Optional: number of operations applied
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operations_count: Option<usize>,
}
