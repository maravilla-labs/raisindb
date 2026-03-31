// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dry run simulation for package installation
//!
//! Simulates the installation process without making any changes, producing
//! a detailed log and summary of what would happen.

mod content_simulation;
mod schema_simulation;

use raisin_error::Result;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;

use super::handler::PackageInstallHandler;
use super::types::{DryRunActionCounts, DryRunLogEntry, DryRunResult, DryRunSummary, InstallMode};

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Perform a dry run simulation of package installation
    ///
    /// This method simulates the installation process without making any changes.
    /// It returns a detailed log of what would happen and a summary of actions.
    pub async fn dry_run(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        zip_data: &[u8],
        install_mode: InstallMode,
    ) -> Result<DryRunResult> {
        let mut logs: Vec<DryRunLogEntry> = Vec::new();
        let mut summary = DryRunSummary::default();

        // Parse manifest
        let manifest = self.extract_manifest(zip_data)?;

        logs.push(DryRunLogEntry {
            level: "info".to_string(),
            category: "manifest".to_string(),
            path: "manifest.yaml".to_string(),
            message: format!("Package: {} v{}", manifest.name, manifest.version),
            action: "info".to_string(),
        });

        // Clone the data for each phase since we can't hold ZipArchive across await points
        let zip_data_owned = zip_data.to_vec();

        // Simulate mixins (before node types)
        self.dry_run_mixins(
            &zip_data_owned,
            tenant_id,
            repo_id,
            install_mode,
            &mut logs,
            &mut summary.mixins,
        )
        .await?;

        // Simulate node types
        self.dry_run_node_types(
            &zip_data_owned,
            tenant_id,
            repo_id,
            install_mode,
            &mut logs,
            &mut summary.node_types,
        )
        .await?;

        // Simulate archetypes
        self.dry_run_archetypes(
            &zip_data_owned,
            tenant_id,
            repo_id,
            install_mode,
            &mut logs,
            &mut summary.archetypes,
        )
        .await?;

        // Simulate element types
        self.dry_run_element_types(
            &zip_data_owned,
            tenant_id,
            repo_id,
            install_mode,
            &mut logs,
            &mut summary.element_types,
        )
        .await?;

        // Simulate workspaces
        self.dry_run_workspaces(
            &zip_data_owned,
            tenant_id,
            repo_id,
            install_mode,
            &mut logs,
            &mut summary.workspaces,
        )
        .await?;

        // Simulate content nodes
        self.dry_run_content_nodes(
            &zip_data_owned,
            tenant_id,
            repo_id,
            branch,
            install_mode,
            &mut logs,
            &mut summary.content_nodes,
            &mut summary.binary_files,
        )
        .await?;

        // Simulate package assets
        self.dry_run_package_assets(
            &zip_data_owned,
            tenant_id,
            repo_id,
            branch,
            &manifest.name,
            install_mode,
            &mut logs,
            &mut summary.package_assets,
        )
        .await?;

        Ok(DryRunResult { logs, summary })
    }

    /// Helper to compute dry run action and message
    pub(super) fn dry_run_action(
        exists: bool,
        install_mode: InstallMode,
        item_type: &str,
        name: &str,
        counts: &mut DryRunActionCounts,
    ) -> (&'static str, String) {
        match (exists, install_mode) {
            (true, InstallMode::Skip) => {
                counts.skip += 1;
                if name.is_empty() {
                    ("skip", format!("{} already exists, will skip", item_type))
                } else {
                    (
                        "skip",
                        format!("{} '{}' already exists, will skip", item_type, name),
                    )
                }
            }
            (true, InstallMode::Sync) | (true, InstallMode::Overwrite) => {
                counts.update += 1;
                if name.is_empty() {
                    ("update", format!("{} exists, will update", item_type))
                } else {
                    (
                        "update",
                        format!("{} '{}' exists, will update", item_type, name),
                    )
                }
            }
            (false, _) => {
                counts.create += 1;
                if name.is_empty() {
                    ("create", format!("{} will be installed", item_type))
                } else {
                    (
                        "create",
                        format!("{} '{}' will be created", item_type, name),
                    )
                }
            }
        }
    }
}
