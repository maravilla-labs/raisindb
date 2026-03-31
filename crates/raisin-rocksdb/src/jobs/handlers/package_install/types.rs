// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Public types for package installation

use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Install mode for handling conflicts during package installation
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// Skip if exists (default) - don't overwrite existing content
    #[default]
    Skip,
    /// Overwrite - delete and replace existing content
    Overwrite,
    /// Sync - update existing content, create new, leave untouched content alone
    Sync,
}

/// Callback type for binary retrieval
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (resource_key)
/// Returns: Result<Vec<u8>> - the binary data
pub type BinaryRetrievalCallback = Arc<
    dyn Fn(
            String, // resource_key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for binary storage (writing)
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (data, content_type, extension, filename, tenant_context)
/// Returns: Result<StoredObject> - metadata about stored binary
pub type BinaryStorageCallback = Arc<
    dyn Fn(
            Vec<u8>,        // data
            Option<String>, // content_type
            Option<String>, // extension
            Option<String>, // original_name
            Option<String>, // tenant_context
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<raisin_binary::StoredObject>> + Send>,
        > + Send
        + Sync,
>;

/// Callback type for binary storage from file path (writing large files)
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Used for large files to avoid loading entire file into memory.
/// Arguments: (file_path, content_type, extension, filename)
/// Returns: Result<StoredObject> - metadata about stored binary
pub type BinaryStorageFromPathCallback = Arc<
    dyn Fn(
            std::path::PathBuf, // file_path
            Option<String>,     // content_type
            Option<String>,     // extension
            Option<String>,     // original_name
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<raisin_binary::StoredObject>> + Send>,
        > + Send
        + Sync,
>;

/// Result of package installation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInstallResult {
    /// Package name
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// Number of mixins installed
    #[serde(default)]
    pub mixins_installed: usize,
    /// Number of node types installed
    pub node_types_installed: usize,
    /// Number of workspaces installed
    pub workspaces_installed: usize,
    /// Number of workspace patches applied
    pub workspace_patches_applied: usize,
    /// Number of content nodes created
    pub content_nodes_created: usize,
    /// Number of binary files installed as assets
    pub binary_files_installed: usize,
    /// Number of translations applied to content nodes
    pub translations_applied: usize,
}

// ============================================================================
// Dry Run Types
// ============================================================================

/// Result of a dry run simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunResult {
    pub logs: Vec<DryRunLogEntry>,
    pub summary: DryRunSummary,
}

/// A single log entry from the dry run simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunLogEntry {
    /// Log level: "info", "create", "update", "skip"
    pub level: String,
    /// Category: "node_type", "workspace", "content", "binary", "archetype", "element_type"
    pub category: String,
    /// Path or name of the item
    pub path: String,
    /// Human-readable message
    pub message: String,
    /// Action that would be taken: "create", "update", "skip"
    pub action: String,
}

/// Summary of actions that would be taken
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DryRunSummary {
    pub mixins: DryRunActionCounts,
    pub node_types: DryRunActionCounts,
    pub archetypes: DryRunActionCounts,
    pub element_types: DryRunActionCounts,
    pub workspaces: DryRunActionCounts,
    pub content_nodes: DryRunActionCounts,
    pub binary_files: DryRunActionCounts,
    pub package_assets: DryRunActionCounts,
}

/// Counts of create/update/skip actions
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DryRunActionCounts {
    pub create: usize,
    pub update: usize,
    pub skip: usize,
}
