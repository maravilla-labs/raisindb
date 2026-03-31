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

//! Package installation job handler
//!
//! This module handles background installation of RaisinDB packages (.rap files).
//! Package installation includes:
//! 1. Installing nested .rap dependencies first (recursive)
//! 2. Extracting node types from nodetypes/ directory
//! 3. Extracting workspaces from workspaces/ directory
//! 4. Creating content nodes from content/ directory
//! 5. Applying workspace patches
//!
//! Progress is reported via JobRegistry updates which can be monitored via SSE.
//!
//! # Install Modes
//!
//! - `skip` (default): Only install to paths that don't exist, preserve existing content
//! - `overwrite`: Delete and replace all existing content from the package
//! - `sync`: Update existing content (upsert), create new content, leave untouched content alone
//!
//! Content nodes are installed using transactions with batch commits (every CONTENT_BATCH_SIZE nodes)
//! for better performance and atomicity.
//!
//! # Nested Packages
//!
//! .rap files can contain other .rap files. These are treated as dependencies and
//! installed BEFORE the outer package content, up to MAX_NESTING_DEPTH levels.
//!
//! # Architecture Note
//!
//! Due to dependency structure, the actual binary retrieval is done via a callback
//! provided by the transport layer which has access to BinaryStorage.

mod content_types;
mod dry_run;
mod handler;
mod install_content;
mod install_schema;
mod manifest;
mod nested;
pub(in crate::jobs::handlers) mod translation;
mod types;

pub use self::handler::PackageInstallHandler;
pub use self::manifest::{
    AllowedNodeTypesPatch, PackageDependency, PackageManifest, PackageProvides, WorkspacePatch,
};
pub use self::types::{
    BinaryRetrievalCallback, BinaryStorageCallback, BinaryStorageFromPathCallback,
    DryRunActionCounts, DryRunLogEntry, DryRunResult, DryRunSummary, InstallMode,
    PackageInstallResult,
};
