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

//! Package install handler struct, constructors, and main job entry point

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobRegistry, JobType};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::Storage;
use std::io::Cursor;
use std::sync::Arc;
use zip::ZipArchive;

use super::content_types::InstallStats;
use super::types::{
    BinaryRetrievalCallback, BinaryStorageCallback, InstallMode, PackageInstallResult,
};

/// Handler for package installation jobs
pub struct PackageInstallHandler<S: Storage + TransactionalStorage> {
    pub(super) storage: Arc<S>,
    pub(super) job_registry: Arc<JobRegistry>,
    pub(super) binary_callback: Option<BinaryRetrievalCallback>,
    pub(super) binary_store_callback: Option<BinaryStorageCallback>,
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Create a new package install handler
    pub fn new(storage: Arc<S>, job_registry: Arc<JobRegistry>) -> Self {
        Self {
            storage,
            job_registry,
            binary_callback: None,
            binary_store_callback: None,
        }
    }

    /// Set the binary retrieval callback
    pub fn with_binary_callback(mut self, callback: BinaryRetrievalCallback) -> Self {
        self.binary_callback = Some(callback);
        self
    }

    /// Set the binary retrieval callback (mutable reference)
    pub fn set_binary_callback(&mut self, callback: BinaryRetrievalCallback) {
        self.binary_callback = Some(callback);
    }

    /// Set the binary storage callback (for writing binaries)
    pub fn with_binary_store_callback(mut self, callback: BinaryStorageCallback) -> Self {
        self.binary_store_callback = Some(callback);
        self
    }

    /// Set the binary storage callback (mutable reference)
    pub fn set_binary_store_callback(&mut self, callback: BinaryStorageCallback) {
        self.binary_store_callback = Some(callback);
    }

    /// Handle package installation job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract parameters from JobType
        let (package_name, package_version, package_node_id) = match &job.job_type {
            JobType::PackageInstall {
                package_name,
                package_version,
                package_node_id,
            } => (
                package_name.as_str(),
                package_version.as_str(),
                package_node_id.as_str(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected PackageInstall job type".to_string(),
                ))
            }
        };

        // Get install mode from context metadata (defaults to Keep)
        let install_mode: InstallMode = context
            .metadata
            .get("install_mode")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        tracing::info!(
            job_id = %job.id,
            package_name = %package_name,
            package_version = %package_version,
            install_mode = ?install_mode,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Starting package installation"
        );

        // Get resource key from context metadata
        let resource_key = context
            .metadata
            .get("resource_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Missing resource_key in job context".to_string()))?
            .to_string();

        // Get binary callback
        let binary_callback = self.binary_callback.as_ref().ok_or_else(|| {
            Error::Validation("Binary retrieval callback not configured".to_string())
        })?;

        // Retrieve the ZIP binary
        let zip_data = binary_callback(resource_key).await?;

        // Report progress: extracting
        self.report_progress(&job.id, 0.1, "Extracting package contents")
            .await;

        // Parse manifest
        let manifest = self.extract_manifest(&zip_data)?;

        // Report progress: validating
        self.report_progress(&job.id, 0.12, "Validating manifest")
            .await;

        // Initialize stats
        let mut stats = InstallStats::default();

        // Phase 0: Install nested .rap dependencies FIRST (15-20%)
        self.report_progress(&job.id, 0.15, "Installing dependencies")
            .await;
        self.install_nested_packages(
            &zip_data,
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &job.id,
            install_mode,
            &mut stats,
            1, // depth starts at 1 for the outer package
        )
        .await?;

        // Open ZIP archive
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

        // Phase 0b: Install mixins BEFORE node types (20-24%)
        self.report_progress(&job.id, 0.2, "Installing mixins")
            .await;
        self.install_mixins(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &job.id,
            install_mode,
            &mut stats,
        )
        .await?;

        // Phase 1: Install node types (24-32%)
        self.report_progress(&job.id, 0.24, "Installing node types")
            .await;
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;
        self.install_node_types(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &job.id,
            install_mode,
            &mut stats,
        )
        .await?;

        // Phase 1b: Install archetypes (32-36%)
        self.report_progress(&job.id, 0.32, "Installing archetypes")
            .await;
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;
        self.install_archetypes(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &job.id,
            install_mode,
            &mut stats,
        )
        .await?;

        // Phase 1c: Install element types (36-40%)
        self.report_progress(&job.id, 0.36, "Installing element types")
            .await;
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;
        self.install_element_types(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &job.id,
            install_mode,
            &mut stats,
        )
        .await?;

        // Phase 2: Install workspaces (40-60%)
        self.report_progress(&job.id, 0.4, "Installing workspaces")
            .await;
        // Re-open archive (zip crate limitation)
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;
        self.install_workspaces(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &job.id,
            install_mode,
            &mut stats,
        )
        .await?;

        // Phase 3: Apply workspace patches (60-70%)
        self.report_progress(&job.id, 0.6, "Applying workspace patches")
            .await;
        self.apply_workspace_patches(
            &manifest,
            &context.tenant_id,
            &context.repo_id,
            &job.id,
            &mut stats,
        )
        .await?;

        // Phase 4: Install content nodes (70-85%)
        self.report_progress(&job.id, 0.7, "Installing content nodes")
            .await;
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;
        self.install_content_nodes(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &job.id,
            install_mode,
            &manifest.workspace_patches,
            &mut stats,
        )
        .await?;

        // Phase 5: Install package assets (README.md, static/) (85-90%)
        self.report_progress(&job.id, 0.85, "Installing package assets")
            .await;
        let cursor = Cursor::new(&zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;
        self.install_package_assets(
            &mut archive,
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            package_node_id,
            package_name,
            &job.id,
            install_mode,
            &mut stats,
        )
        .await?;

        // Phase 6: Finalize - update package node (90-100%)
        self.report_progress(&job.id, 0.9, "Finalizing installation")
            .await;
        self.finalize_installation(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            package_node_id,
        )
        .await?;

        self.report_progress(&job.id, 1.0, "Installation complete")
            .await;

        tracing::info!(
            job_id = %job.id,
            package_name = %package_name,
            install_mode = ?install_mode,
            mixins = stats.mixins_installed,
            mixins_skipped = stats.mixins_skipped,
            node_types = stats.node_types_installed,
            node_types_skipped = stats.node_types_skipped,
            workspaces = stats.workspaces_installed,
            workspaces_skipped = stats.workspaces_skipped,
            patches = stats.patches_applied,
            content_nodes = stats.content_nodes_created,
            content_nodes_skipped = stats.content_nodes_skipped,
            nested_packages = stats.nested_packages_installed,
            binary_files = stats.binary_files_installed,
            package_assets = stats.package_assets_installed,
            translations = stats.translations_applied,
            translations_skipped = stats.translations_skipped,
            "Package installation completed"
        );

        // Return summary
        let result = PackageInstallResult {
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
            mixins_installed: stats.mixins_installed,
            node_types_installed: stats.node_types_installed,
            workspaces_installed: stats.workspaces_installed,
            workspace_patches_applied: stats.patches_applied,
            content_nodes_created: stats.content_nodes_created,
            binary_files_installed: stats.binary_files_installed,
            translations_applied: stats.translations_applied,
        };

        Ok(Some(serde_json::to_value(result).unwrap_or_default()))
    }

    /// Report progress to job registry
    pub(super) async fn report_progress(&self, job_id: &JobId, progress: f32, message: &str) {
        tracing::debug!(job_id = %job_id, progress = %progress, message = %message, "Package install progress");
        if let Err(e) = self.job_registry.update_progress(job_id, progress).await {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to update job progress");
        }
    }

    /// Finalize installation by updating package node
    pub(super) async fn finalize_installation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        package_node_id: &str,
    ) -> Result<()> {
        let workspace = "packages";

        // Create transaction with system auth context
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant_id, repo_id)?;
        tx.set_branch(branch)?;
        tx.set_actor("package-installer")?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message("Finalize package installation")?;

        // Get the package node
        let node = tx.get_node(workspace, package_node_id).await?;

        if let Some(mut node) = node {
            // Update installed status
            node.properties
                .insert("installed".to_string(), PropertyValue::Boolean(true));
            node.properties.insert(
                "installed_at".to_string(),
                PropertyValue::String(chrono::Utc::now().to_rfc3339()),
            );

            tx.upsert_node(workspace, &node).await?;
            tx.commit().await?;
        }

        Ok(())
    }
}
