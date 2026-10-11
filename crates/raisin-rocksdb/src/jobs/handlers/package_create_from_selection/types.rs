//! Public types for package create from selection

use serde::{Deserialize, Serialize};

/// A selected path for package creation (matches the HTTP layer definition)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SelectedPath {
    /// Workspace containing the content
    pub workspace: String,
    /// Path to the node or folder (use "/*" suffix for recursive selection)
    pub path: String,
}

/// Result of package creation from selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageCreateFromSelectionResult {
    /// Package name
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// Number of nodes included
    pub nodes_included: usize,
    /// Number of node types included (if include_node_types was true)
    pub node_types_included: usize,
    /// Blob key for downloading the package
    pub blob_key: String,
    /// URL to download the package
    pub download_url: String,
    /// Creation timestamp
    pub created_at: String,
    /// Exact archive size, from the stream that wrote it (E23)
    #[serde(default)]
    pub size_bytes: u64,
    /// SHA-256 of the archive, from the same pass (E23)
    #[serde(default)]
    pub sha256: String,
    /// Blob key of the `manifest.yaml` sidecar (E24)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_key: Option<String>,
    /// Blob key of the sorted `inventory.jsonl` sidecar (E8/E24)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_key: Option<String>,
}
