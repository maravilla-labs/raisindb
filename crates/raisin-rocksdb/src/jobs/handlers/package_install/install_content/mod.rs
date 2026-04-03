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

//! Content node and package asset installation
//!
//! This module orchestrates the installation of content nodes (from `content/`
//! directories) and package assets (README.md, static files) from ZIP archives.
//!
//! Submodules:
//! - [`zip_collector`]: ZIP iteration and raw entry collection
//! - [`node_installer`]: Sorted entry batch installation (YAML nodes + binary files)
//! - [`package_assets`]: README and static file installation as package assets

mod node_installer;
mod package_assets;
pub(super) mod reference_sort;
mod zip_collector;

pub(in crate::jobs::handlers::package_install) use self::zip_collector::CollectedEntries;

use raisin_error::Result;
use raisin_storage::jobs::JobId;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::collections::HashMap;
use std::io::Cursor;
use zip::ZipArchive;

use super::content_types::InstallStats;
use super::handler::PackageInstallHandler;
use super::manifest::WorkspacePatch;
use super::types::InstallMode;

/// Default folder type used when no workspace patch override is specified
const DEFAULT_FOLDER_TYPE: &str = "raisin:Folder";

/// Resolve the folder type for a workspace from the patch map
pub(in crate::jobs::handlers::package_install) fn resolve_folder_type<'a>(
    folder_type_map: &'a HashMap<String, String>,
    workspace: &str,
) -> &'a str {
    folder_type_map
        .get(workspace)
        .map(|s| s.as_str())
        .unwrap_or(DEFAULT_FOLDER_TYPE)
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Install content nodes from content/ directory
    ///
    /// This function handles:
    /// 1. YAML node definitions (.node.yaml, *.yaml)
    /// 2. Asset metadata files (.node.{filename}.yaml)
    /// 3. Binary files (index.js, images, etc.) -> created as raisin:Asset child nodes
    ///
    /// In `skip` mode: Skip if node already exists at the same path (or hash matches for binaries)
    /// In `overwrite` mode: Delete and replace existing nodes
    /// In `sync` mode: Update if changed (using content hash for binaries)
    pub(super) async fn install_content_nodes(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        workspace_patches: &Option<HashMap<String, WorkspacePatch>>,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // Build a lookup map for workspace -> folder type (owned Strings)
        let folder_type_map: HashMap<String, String> = workspace_patches
            .as_ref()
            .map(|patches| {
                patches
                    .iter()
                    .filter_map(|(ws_name, patch)| {
                        patch
                            .default_folder_type
                            .as_ref()
                            .map(|ft| (ws_name.clone(), ft.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Get binary store callback (optional - binary files will be skipped if not configured)
        let binary_store = self.binary_store_callback.as_ref();

        // Phase 1: Collect all files from ZIP, categorizing them
        let (entries, asset_metadata) = self.collect_content_entries(archive, job_id)?;

        // Phase 2: Build content entries from collected data
        let entries = self.build_content_entries(entries, asset_metadata, job_id)?;

        // Phase 3: Sort and install
        self.install_sorted_entries(
            entries,
            tenant_id,
            repo_id,
            branch,
            job_id,
            install_mode,
            &folder_type_map,
            binary_store,
            stats,
        )
        .await
    }
}
