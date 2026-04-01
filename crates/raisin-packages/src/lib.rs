// SPDX-License-Identifier: BSL-1.1

// TODO(v0.2): Clean up unused code
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

//! RaisinDB Package Management
//!
//! This crate provides functionality for managing .rap (Raisin Archive Package) files:
//! - Parse and validate package manifests
//! - Browse package contents without extracting
//! - Install/uninstall packages to repositories
//! - Apply workspace patches
//!
//! # Package Structure
//!
//! A `.rap` file is a ZIP archive containing:
//! ```text
//! manifest.yaml           # Package metadata
//! mixins/                 # Mixin definitions (reusable property sets)
//! nodetypes/              # Node type definitions
//! workspaces/             # Workspace configurations
//! content/                # Content to install (nodes, assets)
//! ```

mod browser;
pub mod dependency_graph;
mod error;
pub mod exporter;
mod installer;
mod manifest;
pub mod namespace_encoding;
mod patcher;
pub mod sync;
pub mod sync_config;

pub use browser::{EntryType, PackageBrowser, ZipEntry};
pub use dependency_graph::{
    AvailableTypes, ContentValidationResult, ContentValidationWarning, ContentValidator,
    DependencyGraph, DependencyGraphError, PackageNode,
};
pub use error::{PackageError, PackageResult};
pub use exporter::{
    ContentBuilder, ExportContent, ExportMixin, ExportNodeType, ExportResult, PackageComparator,
    PackageExporter,
};
pub use installer::{ContentNode, InstallResult, PackageInstaller, UninstallResult};
pub use manifest::{Dependency, Manifest, Provides, WorkspacePatch};
pub use patcher::{PatchOperation, WorkspacePatcher};
pub use sync::{
    compute_hash, DiffType, ExportMode, ExportOptions, FileDiff, OverallSyncStatus,
    PackageSyncStatus, SyncError, SyncFileInfo, SyncFileStatus, SyncResult, SyncSummary,
};
pub use sync_config::{
    ConflictOverride, ConflictStrategy, PropertyFilter, RemoteConfig, SyncConfig, SyncDefaults,
    SyncDirection, SyncFilter, SyncMode,
};
