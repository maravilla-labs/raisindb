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
    AppliedHashRecorder, BinaryRetrievalCallback, BinaryStorageCallback, InstallMode,
    PackageInstallResult,
};

/// Handler for package installation jobs
pub struct PackageInstallHandler<S: Storage + TransactionalStorage> {
    pub(super) storage: Arc<S>,
    pub(super) job_registry: Arc<JobRegistry>,
    pub(super) binary_callback: Option<BinaryRetrievalCallback>,
    pub(super) binary_store_callback: Option<BinaryStorageCallback>,
    /// Records what content this install applied, so a later release can be
    /// seen as an update. See [`AppliedHashRecorder`].
    pub(super) applied_recorder: Option<AppliedHashRecorder>,
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Create a new package install handler
    pub fn new(storage: Arc<S>, job_registry: Arc<JobRegistry>) -> Self {
        Self {
            storage,
            job_registry,
            binary_callback: None,
            binary_store_callback: None,
            applied_recorder: None,
        }
    }

    /// Set the applied-hash recorder.
    pub fn with_applied_recorder(mut self, recorder: AppliedHashRecorder) -> Self {
        self.applied_recorder = Some(recorder);
        self
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
                package_name.clone(),
                package_version.clone(),
                package_node_id.clone(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected PackageInstall job type".to_string(),
                ))
            }
        };

        // Status lifecycle: uploaded -> installing -> installed | failed
        self.set_package_status(context, &package_node_id, "installing", None)
            .await;

        let result = self
            .run_install(
                job,
                context,
                &package_name,
                &package_version,
                &package_node_id,
            )
            .await;

        if let Err(ref e) = result {
            self.set_package_status(context, &package_node_id, "failed", Some(&e.to_string()))
                .await;
        }

        result
    }

    /// Best-effort update of the package node's status (and error detail).
    ///
    /// Failures here are logged but never fail the install job itself. If the
    /// repository's raisin:Package NodeType predates the `error` property
    /// (strict mode rejects unknown properties), the write is retried without
    /// the error detail so the status itself always lands.
    pub(super) async fn set_package_status(
        &self,
        context: &JobContext,
        package_node_id: &str,
        status: &str,
        error: Option<&str>,
    ) {
        let attempt = |include_error: bool| async move {
            let tx = self.storage.begin_context().await?;
            tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
            tx.set_branch(&context.branch)?;
            tx.set_actor("package-installer")?;
            tx.set_auth_context(AuthContext::system())?;
            tx.set_message(&format!("Set package status to {}", status))?;

            if let Some(mut node) = tx.get_node("packages", package_node_id).await? {
                node.properties.insert(
                    "status".to_string(),
                    PropertyValue::String(status.to_string()),
                );
                match error {
                    Some(detail) if include_error => {
                        node.properties.insert(
                            "error".to_string(),
                            PropertyValue::String(detail.to_string()),
                        );
                    }
                    Some(_) => {}
                    None => {
                        node.properties.remove("error");
                    }
                }
                tx.upsert_node("packages", &node).await?;
                tx.commit().await?;
            }
            Ok::<(), Error>(())
        };

        let result = match attempt(true).await {
            Err(Error::Validation(_)) if error.is_some() => attempt(false).await,
            other => other,
        };

        if let Err(e) = result {
            tracing::warn!(
                package_node_id = %package_node_id,
                status = %status,
                error = %e,
                "Failed to update package status"
            );
        }
    }

    /// Run the actual installation phases.
    async fn run_install(
        &self,
        job: &JobInfo,
        context: &JobContext,
        package_name: &str,
        package_version: &str,
        package_node_id: &str,
    ) -> Result<Option<serde_json::Value>> {
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

        // Parse the optional `.raisin-sync.yaml` per-path sync/install policy,
        // if this package ships one at its root.
        let sync_config = self.extract_sync_config(&zip_data)?;

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
            &context.branch,
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
            &context.branch,
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
            &context.branch,
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
            &context.branch,
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
            sync_config.as_ref(),
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
        // Content entries are installed best-effort so one bad node cannot
        // silence the rest (see `is_per_node_error` in `node_installer.rs`).
        // Fail here, before `finalize_installation` flips `installed` to true —
        // a package that half-applied must not report itself as installed, or
        // the next `deploy --install` will skip it and the gap becomes
        // permanent.
        if !stats.content_errors.is_empty() {
            let detail = stats
                .content_errors
                .iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n");
            tracing::error!(
                job_id = %job.id,
                package_name = %package_name,
                rejected = stats.content_errors.len(),
                "Package installation rejected content entries"
            );
            return Err(raisin_error::Error::Validation(format!(
                "Package '{}' installed {} content node(s) but {} were rejected:\n{}",
                package_name,
                stats.content_nodes_created + stats.content_nodes_synced,
                stats.content_errors.len(),
                detail
            )));
        }

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
            // Update installed status (lifecycle: ... -> installing -> installed)
            node.properties
                .insert("installed".to_string(), PropertyValue::Boolean(true));
            node.properties.insert(
                "installed_at".to_string(),
                PropertyValue::String(chrono::Utc::now().to_rfc3339()),
            );
            node.properties.insert(
                "status".to_string(),
                PropertyValue::String("installed".to_string()),
            );
            node.properties.remove("error");

            // What content this install applied, read off the node that was
            // just installed rather than passed in — the registration wrote it
            // there, so this is the hash of the bytes that actually landed.
            let applied_hash = match node.properties.get("content_hash") {
                Some(PropertyValue::String(h)) if !h.is_empty() => Some(h.clone()),
                _ => None,
            };
            let package_name = match node.properties.get("name") {
                Some(PropertyValue::String(n)) if !n.is_empty() => n.clone(),
                _ => node.name.clone(),
            };

            tx.upsert_node(workspace, &node).await?;
            tx.commit().await?;

            // Record it AFTER the commit, and never fail the install on it: the
            // package is installed either way, and a missing record costs an
            // update opportunity rather than correctness. Recording before the
            // commit would claim content that a failed commit never wrote.
            //
            // Without this an on-demand install is invisible to the staleness
            // check, which reads "no record" as "assume current" — so the
            // package never updates again, no matter how many releases ship.
            if let (Some(recorder), Some(hash)) = (&self.applied_recorder, applied_hash) {
                if let Err(e) = recorder(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    package_name.clone(),
                    hash,
                )
                .await
                {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        package = %package_name,
                        error = %e,
                        "Package installed, but recording its applied content hash failed; \
                         it will not be seen as updatable until it is installed again"
                    );
                }
            }
        }

        Ok(())
    }
}
