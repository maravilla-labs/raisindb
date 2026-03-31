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

//! Schema installation: node types, archetypes, element types, workspaces, patches

use raisin_error::{Error, Result};
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::{Archetype, NodeType};
use raisin_models::workspace::Workspace;
use raisin_storage::jobs::JobId;
use raisin_storage::scope::{BranchScope, RepoScope};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    ArchetypeRepository, CommitMetadata, ElementTypeRepository, NodeTypeRepository, Storage,
    WorkspaceRepository,
};
use std::io::{Cursor, Read};
use zip::ZipArchive;

use super::content_types::InstallStats;
use super::handler::PackageInstallHandler;
use super::manifest::PackageManifest;
use super::types::InstallMode;

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Install mixins from mixins/ directory
    ///
    /// Mixins are installed BEFORE node types since node types may reference them.
    /// In `skip` mode: Skip if node type (mixin) already exists
    /// In `overwrite`/`sync` mode: Overwrite existing mixins
    pub(super) async fn install_mixins(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // First, collect all mixins synchronously (ZipFile is not Send)
        let mut mixins_to_install: Vec<(String, NodeType)> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check if this is a mixin YAML file
            if name.starts_with("mixins/") && name.ends_with(".yaml") && !file.is_dir() {
                let mut content = String::new();
                file.read_to_string(&mut content).map_err(|e| {
                    Error::storage(format!("Failed to read mixin file {}: {}", name, e))
                })?;

                // Parse mixin as NodeType
                let node_type: NodeType = serde_yaml::from_str(&content).map_err(|e| {
                    Error::Validation(format!("Invalid mixin YAML in {}: {}", name, e))
                })?;

                mixins_to_install.push((name, node_type));
            }
        }

        // Now install them (with await) - file references are dropped
        let node_type_repo = self.storage.node_types();

        for (name, node_type) in mixins_to_install {
            let type_name = node_type.name.clone();

            // In skip mode, check if mixin exists first
            if install_mode == InstallMode::Skip {
                let existing = node_type_repo
                    .get(
                        BranchScope::new(tenant_id, repo_id, "main"),
                        &type_name,
                        None,
                    )
                    .await?;
                if existing.is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        mixin = %type_name,
                        "Mixin already exists, skipping (skip mode)"
                    );
                    stats.mixins_skipped += 1;
                    continue;
                }
            }

            // Register mixin as a node type (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install mixin: {}", name));

            node_type_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    node_type,
                    commit,
                )
                .await?;

            tracing::debug!(
                job_id = %job_id,
                mixin = %type_name,
                mode = ?install_mode,
                "Installed mixin"
            );

            stats.mixins_installed += 1;
        }

        Ok(())
    }

    /// Install node types from nodetypes/ directory
    ///
    /// In `keep` mode: Skip if node type already exists
    /// In `force` mode: Overwrite existing node types
    pub(super) async fn install_node_types(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // First, collect all node types synchronously (ZipFile is not Send)
        let mut node_types_to_install: Vec<(String, NodeType)> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check if this is a node type YAML file
            if name.starts_with("nodetypes/") && name.ends_with(".yaml") && !file.is_dir() {
                let mut content = String::new();
                file.read_to_string(&mut content).map_err(|e| {
                    Error::storage(format!("Failed to read node type file {}: {}", name, e))
                })?;

                // Parse node type
                let node_type: NodeType = serde_yaml::from_str(&content).map_err(|e| {
                    Error::Validation(format!("Invalid node type YAML in {}: {}", name, e))
                })?;

                node_types_to_install.push((name, node_type));
            }
        }

        // Now install them (with await) - file references are dropped
        let node_type_repo = self.storage.node_types();

        for (name, node_type) in node_types_to_install {
            let type_name = node_type.name.clone();

            // In skip mode, check if node type exists first
            if install_mode == InstallMode::Skip {
                let existing = node_type_repo
                    .get(
                        BranchScope::new(tenant_id, repo_id, "main"),
                        &type_name,
                        None,
                    )
                    .await?;
                if existing.is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        node_type = %type_name,
                        "Node type already exists, skipping (skip mode)"
                    );
                    stats.node_types_skipped += 1;
                    continue;
                }
            }

            // Register node type (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install: {}", name));

            node_type_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    node_type,
                    commit,
                )
                .await?;

            tracing::debug!(
                job_id = %job_id,
                node_type = %type_name,
                mode = ?install_mode,
                "Installed node type"
            );

            stats.node_types_installed += 1;
        }

        Ok(())
    }

    /// Install archetypes from archetypes/ directory
    ///
    /// In `skip` mode: Skip if archetype already exists
    /// In `force` mode: Overwrite existing archetypes
    pub(super) async fn install_archetypes(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // First, collect all archetypes synchronously (ZipFile is not Send)
        let mut archetypes_to_install: Vec<(String, Archetype)> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check if this is an archetype YAML file
            if name.starts_with("archetypes/") && name.ends_with(".yaml") && !file.is_dir() {
                let mut content = String::new();
                file.read_to_string(&mut content).map_err(|e| {
                    Error::storage(format!("Failed to read archetype file {}: {}", name, e))
                })?;

                // Parse archetype
                let archetype: Archetype = serde_yaml::from_str(&content).map_err(|e| {
                    Error::Validation(format!("Invalid archetype YAML in {}: {}", name, e))
                })?;

                archetypes_to_install.push((name, archetype));
            }
        }

        // Now install them (with await) - file references are dropped
        let archetype_repo = self.storage.archetypes();

        for (name, archetype) in archetypes_to_install {
            let archetype_name = archetype.name.clone();

            // In skip mode, check if archetype exists first
            if install_mode == InstallMode::Skip {
                let existing = archetype_repo
                    .get(
                        BranchScope::new(tenant_id, repo_id, "main"),
                        &archetype_name,
                        None,
                    )
                    .await?;
                if existing.is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        archetype = %archetype_name,
                        "Archetype already exists, skipping (skip mode)"
                    );
                    stats.archetypes_skipped += 1;
                    continue;
                }
            }

            // Register archetype (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install: {}", name));

            archetype_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    archetype,
                    commit,
                )
                .await?;

            tracing::debug!(
                job_id = %job_id,
                archetype = %archetype_name,
                mode = ?install_mode,
                "Installed archetype"
            );

            stats.archetypes_installed += 1;
        }

        Ok(())
    }

    /// Install element types from elementtypes/ directory
    ///
    /// In `skip` mode: Skip if element type already exists
    /// In `force` mode: Overwrite existing element types
    pub(super) async fn install_element_types(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // First, collect all element types synchronously (ZipFile is not Send)
        let mut element_types_to_install: Vec<(String, ElementType)> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check if this is an element type YAML file
            if name.starts_with("elementtypes/") && name.ends_with(".yaml") && !file.is_dir() {
                let mut content = String::new();
                file.read_to_string(&mut content).map_err(|e| {
                    Error::storage(format!("Failed to read element type file {}: {}", name, e))
                })?;

                // Parse element type
                let element_type: ElementType = serde_yaml::from_str(&content).map_err(|e| {
                    Error::Validation(format!("Invalid element type YAML in {}: {}", name, e))
                })?;

                element_types_to_install.push((name, element_type));
            }
        }

        // Now install them (with await) - file references are dropped
        let element_type_repo = self.storage.element_types();

        for (name, element_type) in element_types_to_install {
            let type_name = element_type.name.clone();

            // In skip mode, check if element type exists first
            if install_mode == InstallMode::Skip {
                let existing = element_type_repo
                    .get(
                        BranchScope::new(tenant_id, repo_id, "main"),
                        &type_name,
                        None,
                    )
                    .await?;
                if existing.is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        element_type = %type_name,
                        "Element type already exists, skipping (skip mode)"
                    );
                    stats.element_types_skipped += 1;
                    continue;
                }
            }

            // Register element type (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install: {}", name));

            element_type_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    element_type,
                    commit,
                )
                .await?;

            tracing::debug!(
                job_id = %job_id,
                element_type = %type_name,
                mode = ?install_mode,
                "Installed element type"
            );

            stats.element_types_installed += 1;
        }

        Ok(())
    }

    /// Install workspaces from workspaces/ directory
    ///
    /// In `keep` mode: Skip if workspace already exists
    /// In `force` mode: Overwrite existing workspaces
    pub(super) async fn install_workspaces(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // First, collect all workspaces synchronously (ZipFile is not Send)
        let mut workspaces_to_install: Vec<Workspace> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check if this is a workspace YAML file
            if name.starts_with("workspaces/") && name.ends_with(".yaml") && !file.is_dir() {
                let mut content = String::new();
                file.read_to_string(&mut content).map_err(|e| {
                    Error::storage(format!("Failed to read workspace file {}: {}", name, e))
                })?;

                // Parse workspace
                let workspace: Workspace = serde_yaml::from_str(&content).map_err(|e| {
                    Error::Validation(format!("Invalid workspace YAML in {}: {}", name, e))
                })?;

                workspaces_to_install.push(workspace);
            }
        }

        // Now install them (with await) - file references are dropped
        let workspace_repo = self.storage.workspaces();

        for workspace in workspaces_to_install {
            let ws_name = workspace.name.clone();

            // Check if workspace already exists
            let existing = workspace_repo
                .get(RepoScope::new(tenant_id, repo_id), &ws_name)
                .await?;

            if existing.is_some() && install_mode == InstallMode::Skip {
                // Skip mode: skip existing
                tracing::debug!(
                    job_id = %job_id,
                    workspace = %ws_name,
                    "Workspace already exists, skipping (skip mode)"
                );
                stats.workspaces_skipped += 1;
            } else {
                // Create or overwrite workspace
                workspace_repo
                    .put(RepoScope::new(tenant_id, repo_id), workspace)
                    .await?;

                tracing::debug!(
                    job_id = %job_id,
                    workspace = %ws_name,
                    mode = ?install_mode,
                    overwrite = existing.is_some(),
                    "Installed workspace"
                );

                stats.workspaces_installed += 1;
            }
        }

        Ok(())
    }

    /// Apply workspace patches from manifest
    pub(super) async fn apply_workspace_patches(
        &self,
        manifest: &PackageManifest,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        stats: &mut InstallStats,
    ) -> Result<()> {
        let workspace_repo = self.storage.workspaces();

        if let Some(patches) = &manifest.workspace_patches {
            for (ws_name, patch) in patches {
                // Get existing workspace
                let existing = workspace_repo
                    .get(RepoScope::new(tenant_id, repo_id), ws_name)
                    .await?;

                if let Some(mut workspace) = existing {
                    let mut modified = false;

                    // Apply allowed_node_types patch
                    if let Some(ant_patch) = &patch.allowed_node_types {
                        if let Some(add_types) = &ant_patch.add {
                            for type_name in add_types {
                                if !workspace.allowed_node_types.contains(type_name) {
                                    workspace.allowed_node_types.push(type_name.clone());
                                    modified = true;
                                }
                            }
                        }
                    }

                    if modified {
                        workspace_repo
                            .put(RepoScope::new(tenant_id, repo_id), workspace)
                            .await?;

                        tracing::debug!(
                            job_id = %job_id,
                            workspace = %ws_name,
                            "Applied workspace patch"
                        );

                        stats.patches_applied += 1;
                    }
                } else {
                    tracing::warn!(
                        job_id = %job_id,
                        workspace = %ws_name,
                        "Cannot apply patch: workspace does not exist"
                    );
                }
            }
        }

        Ok(())
    }
}
