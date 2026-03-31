// SPDX-License-Identifier: BSL-1.1

//! Type definitions for the package management API.
//!
//! Contains all request/response types, enums, and serialization helpers
//! used across the package handler submodules.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Package manifest structure (from manifest.yaml in .rap file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<PackageDependency>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provides: Option<PackageProvides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_patches: Option<HashMap<String, WorkspacePatch>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDependency {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageProvides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodetypes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_node_types: Option<AllowedNodeTypesPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedNodeTypesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add: Option<Vec<String>>,
}

/// Response for listing ZIP contents
#[derive(Debug, Serialize)]
pub struct ZipContentsResponse {
    pub entries: Vec<ZipEntry>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct ZipEntry {
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
}

/// Response for upload endpoint
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub package_name: String,
    pub version: String,
    pub node_id: String,
}

/// Response for install/uninstall operations
#[derive(Debug, Serialize)]
pub struct InstallResponse {
    pub package_name: String,
    pub version: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
    /// Job ID for tracking installation progress (only for async install)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// Response for dry run preview endpoint
#[derive(Debug, Serialize)]
pub struct DryRunResponse {
    pub package_name: String,
    pub package_version: String,
    pub mode: InstallMode,
    pub logs: Vec<DryRunLogEntry>,
    pub summary: DryRunSummary,
}

/// A single log entry from the dry run simulation
#[derive(Debug, Clone, Serialize)]
pub struct DryRunLogEntry {
    pub level: String,
    pub category: String,
    pub path: String,
    pub message: String,
    pub action: String,
}

/// Summary of actions that would be taken
#[derive(Debug, Default, Serialize)]
pub struct DryRunSummary {
    pub node_types: ActionCounts,
    pub archetypes: ActionCounts,
    pub element_types: ActionCounts,
    pub workspaces: ActionCounts,
    pub content_nodes: ActionCounts,
    pub binary_files: ActionCounts,
    pub package_assets: ActionCounts,
}

/// Counts of create/update/skip actions
#[derive(Debug, Default, Serialize)]
pub struct ActionCounts {
    pub create: usize,
    pub update: usize,
    pub skip: usize,
}

/// Install mode for handling conflicts during package installation
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    #[default]
    Skip,
    Overwrite,
    Sync,
}

/// Query parameters for package install command
#[derive(Debug, Default, Deserialize)]
pub struct InstallQuery {
    #[serde(default)]
    pub mode: InstallMode,
}

/// SSE events for package installation progress
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallEvent {
    Started {
        package_name: String,
        version: String,
    },
    Progress {
        phase: InstallPhase,
        message: String,
        progress: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Installed {
        item_type: InstalledItemType,
        name: String,
    },
    Warning {
        message: String,
    },
    Completed {
        package_name: String,
        version: String,
        installed_at: String,
        summary: InstallSummary,
    },
    Failed {
        error: String,
    },
    Done,
}

/// Phases of package installation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Validating,
    Extracting,
    NodeTypes,
    Workspaces,
    Patches,
    Content,
    Finalizing,
}

/// Types of items that can be installed
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledItemType {
    NodeType,
    Workspace,
    WorkspacePatch,
    ContentNode,
}

/// Summary of installation results
#[derive(Debug, Clone, Serialize)]
pub struct InstallSummary {
    pub node_types_installed: usize,
    pub workspaces_installed: usize,
    pub workspace_patches_applied: usize,
    pub content_nodes_created: usize,
}

/// Response for browse endpoint (matches frontend PackageFile type)
#[derive(Debug, Serialize)]
pub struct PackageFile {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: FileType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    File,
    Directory,
}

/// Parsed package command from URL path
#[derive(Debug)]
pub(super) enum PackageCommand {
    Browse { zip_path: String },
    File { zip_path: String },
    Install,
    DryRun { mode: InstallMode },
    Export,
    Download { job_id: String },
    SyncStatus,
}

/// Request body for package export
#[derive(Debug, Deserialize)]
pub struct ExportPackageRequest {
    #[serde(default = "default_export_mode")]
    pub export_mode: String,
    #[serde(default = "default_true_fn")]
    pub include_modifications: bool,
}

fn default_export_mode() -> String {
    "filtered".to_string()
}

fn default_true_fn() -> bool {
    true
}

/// Response for export endpoint
#[derive(Debug, Serialize)]
pub struct ExportResponse {
    pub job_id: String,
    pub status: String,
    pub download_path: String,
}

/// Response for sync status endpoint
#[derive(Debug, Serialize)]
pub struct SyncStatusResponse {
    pub package_name: String,
    pub package_version: String,
    pub status: String,
    pub files: Vec<SyncFileInfo>,
    pub summary: SyncSummary,
}

#[derive(Debug, Serialize)]
pub struct SyncFileInfo {
    pub path: String,
    pub status: String,
    pub workspace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncSummary {
    pub synced: u32,
    pub modified: u32,
    pub local_only: u32,
    pub server_only: u32,
    pub conflict: u32,
}

/// Response for diff endpoint
#[derive(Debug, Serialize)]
pub struct DiffResponse {
    pub path: String,
    pub diff_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unified_diff: Option<String>,
}

/// A selected path for package creation
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SelectedPath {
    pub workspace: String,
    pub path: String,
}

/// Request body for creating a package from selected content
#[derive(Debug, Deserialize)]
pub struct CreateFromSelectionRequest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub selected_paths: Vec<SelectedPath>,
    #[serde(default)]
    pub include_node_types: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// Response for create-from-selection endpoint
#[derive(Debug, Serialize)]
pub struct CreateFromSelectionResponse {
    pub job_id: String,
    pub status: String,
    pub download_path: String,
    pub selected_count: usize,
}
