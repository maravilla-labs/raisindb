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
//! processing-rules/       # Asset-processing rules (which uploads get which tasks)
//! content/                # Content to install (nodes, assets)
//! ```
//!
//! `processing-rules/*.yaml` holds one rule or a list of them, and exists so an
//! application's handling of uploaded binaries can travel with the application.
//! Without it a package could ship the nodetype for its documents, the workspace
//! they live in and the trigger that captions them — and then need a human to
//! retype four rules into an admin console, or the uploads would sit unindexed
//! with nothing saying why.
//!
//! Rules are repo CONFIG, not content, so they follow `workspaces/` rather than
//! `content/`. One difference from workspaces: `skip` mode leaves an existing
//! rule id completely alone instead of merging into it. A rule's matcher and
//! task list are one decision, and half a package's rule combined with half an
//! operator's is a third rule neither wrote.

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

pub use browser::{EntryType, PackageBrowser, ZipEntry, SYNC_CONFIG_FILENAME};
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
