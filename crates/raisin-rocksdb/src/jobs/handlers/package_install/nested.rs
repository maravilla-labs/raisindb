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

//! Nested package installation and manifest extraction

use raisin_error::{Error, Result};
use raisin_storage::jobs::JobId;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use super::content_types::{InstallStats, MAX_NESTING_DEPTH};
use super::handler::PackageInstallHandler;
use super::manifest::PackageManifest;
use super::types::InstallMode;

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Extract manifest.yaml from ZIP
    pub(super) fn extract_manifest(&self, zip_data: &[u8]) -> Result<PackageManifest> {
        let cursor = Cursor::new(zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

        let mut manifest_file = archive
            .by_name("manifest.yaml")
            .map_err(|_| Error::Validation("Package must contain manifest.yaml".to_string()))?;

        let mut manifest_content = String::new();
        manifest_file
            .read_to_string(&mut manifest_content)
            .map_err(|e| Error::Validation(format!("Failed to read manifest.yaml: {}", e)))?;

        serde_yaml::from_str(&manifest_content)
            .map_err(|e| Error::Validation(format!("Invalid manifest.yaml format: {}", e)))
    }

    /// Install nested .rap packages (dependencies) before the outer package
    ///
    /// Nested .rap files within a package are treated as dependencies and
    /// must be installed FIRST, before the outer package's content.
    pub(super) async fn install_nested_packages(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
        depth: usize,
    ) -> Result<()> {
        // Check depth limit
        if depth > MAX_NESTING_DEPTH {
            return Err(Error::Validation(format!(
                "Maximum nesting depth ({}) exceeded for nested packages",
                MAX_NESTING_DEPTH
            )));
        }

        // Open archive and find all .rap files
        let cursor = Cursor::new(zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

        // Collect nested .rap file data (we need to read them before dropping the archive)
        let mut nested_packages: Vec<(String, Vec<u8>)> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check if this is a nested .rap package
            // Only look for .rap files in specific directories:
            // - dependencies/
            // - packages/
            // This avoids false positives from sample files or other .rap extensions
            let is_nested_package = name.ends_with(".rap")
                && !file.is_dir()
                && (name.starts_with("dependencies/") || name.starts_with("packages/"));

            if is_nested_package {
                tracing::debug!(
                    job_id = %job_id,
                    file = %name,
                    "Found nested package"
                );

                let mut data = Vec::new();
                file.read_to_end(&mut data).map_err(|e| {
                    Error::storage(format!("Failed to read nested package {}: {}", name, e))
                })?;

                nested_packages.push((name, data));
            }
        }

        // Install each nested package (recursively, depth-first)
        for (name, nested_data) in nested_packages {
            tracing::info!(
                job_id = %job_id,
                nested_package = %name,
                depth = depth,
                "Installing nested package dependency"
            );

            // First, recursively install any packages nested within this one
            Box::pin(self.install_nested_packages(
                &nested_data,
                tenant_id,
                repo_id,
                branch,
                job_id,
                install_mode,
                stats,
                depth + 1,
            ))
            .await?;

            // Now install this nested package's content
            // Re-open archive for each phase due to zip crate limitations
            let cursor = Cursor::new(&nested_data);
            let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
                Error::Validation(format!("Invalid nested ZIP file '{}': {}", name, e))
            })?;

            // Install node types from nested package
            self.install_node_types(
                &mut nested_archive,
                tenant_id,
                repo_id,
                job_id,
                install_mode,
                stats,
            )
            .await?;

            // Install archetypes from nested package
            let cursor = Cursor::new(&nested_data);
            let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
                Error::Validation(format!("Invalid nested ZIP file '{}': {}", name, e))
            })?;

            self.install_archetypes(
                &mut nested_archive,
                tenant_id,
                repo_id,
                job_id,
                install_mode,
                stats,
            )
            .await?;

            // Install element types from nested package
            let cursor = Cursor::new(&nested_data);
            let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
                Error::Validation(format!("Invalid nested ZIP file '{}': {}", name, e))
            })?;

            self.install_element_types(
                &mut nested_archive,
                tenant_id,
                repo_id,
                job_id,
                install_mode,
                stats,
            )
            .await?;

            // Install workspaces from nested package
            let cursor = Cursor::new(&nested_data);
            let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
                Error::Validation(format!("Invalid nested ZIP file '{}': {}", name, e))
            })?;

            self.install_workspaces(
                &mut nested_archive,
                tenant_id,
                repo_id,
                job_id,
                install_mode,
                stats,
            )
            .await?;

            // Apply workspace patches from nested package
            let manifest = self.extract_manifest(&nested_data)?;
            self.apply_workspace_patches(&manifest, tenant_id, repo_id, job_id, stats)
                .await?;

            // Install content nodes from nested package
            let cursor = Cursor::new(&nested_data);
            let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
                Error::Validation(format!("Invalid nested ZIP file '{}': {}", name, e))
            })?;

            self.install_content_nodes(
                &mut nested_archive,
                tenant_id,
                repo_id,
                branch,
                job_id,
                install_mode,
                &manifest.workspace_patches,
                stats,
            )
            .await?;

            stats.nested_packages_installed += 1;

            tracing::info!(
                job_id = %job_id,
                nested_package = %name,
                depth = depth,
                "Nested package dependency installed"
            );
        }

        Ok(())
    }
}
