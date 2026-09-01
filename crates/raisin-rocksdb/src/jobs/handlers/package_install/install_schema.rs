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

//! Schema installation: node types, archetypes, element types, workspaces,
//! processing rules, patches

use raisin_error::{Error, Result};
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::{Archetype, NodeType};
use raisin_models::workspace::Workspace;
use raisin_storage::jobs::JobId;
use raisin_storage::scope::{BranchScope, RepoScope};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    ArchetypeRepository, CommitMetadata, ElementTypeRepository, NodeTypeRepository,
    ProcessingRulesRepository, Storage, WorkspaceRepository,
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
    // Schema registrations are BRANCH-scoped: an install writes its
    // mixins/node types/archetypes/element types onto the branch the
    // install targets — this is the explicit mechanism for bringing a
    // secondary branch's type registry up to date (a `?branch=publish`
    // install used to write content to `publish` but schema to `main`,
    // leaving branch writes failing archetype resolution).
    pub(super) async fn install_mixins(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
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

            // Schema is package-authoritative: always upsert (create or update)
            // so re-installing a package applies changed definitions, even in
            // Skip mode (Skip only protects existing *content* nodes from being
            // overwritten — schema must track the package).

            // Register mixin as a node type (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install mixin: {}", name));

            node_type_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, branch),
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
        branch: &str,
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

            // Schema is package-authoritative: always upsert (create or update)
            // so re-installing a package applies changed definitions, even in
            // Skip mode (Skip only protects existing *content* nodes).

            // Register node type (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install: {}", name));

            node_type_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, branch),
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
        branch: &str,
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

            // Schema is package-authoritative: always upsert (create or update)
            // so re-installing a package applies changed definitions, even in
            // Skip mode (Skip only protects existing *content* nodes).

            // Register archetype (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install: {}", name));

            archetype_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, branch),
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
        branch: &str,
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

            // Schema is package-authoritative: always upsert (create or update)
            // so re-installing a package applies changed definitions, even in
            // Skip mode (Skip only protects existing *content* nodes).

            // Register element type (upsert handles both create and update)
            let commit = CommitMetadata::system(format!("Package install: {}", name));

            element_type_repo
                .upsert(
                    BranchScope::new(tenant_id, repo_id, branch),
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

    /// Install asset-processing rules from the `processing-rules/` directory.
    ///
    /// # Why a package may carry these at all
    ///
    /// A processing rule is the routing table for uploaded binaries: which
    /// nodes get which tasks (`extract_text`, `doc_to_markdown`, a thumbnail).
    /// It lived only in the admin console, per repo, which made it the one part
    /// of an application's behaviour that could not travel with the
    /// application. A package could ship the nodetype for its documents, the
    /// workspace they live in, the trigger that captions them — and then
    /// require a human to open a console and retype four rules, or the uploads
    /// would sit unindexed with nothing saying why.
    ///
    /// Rules are repo config, not content, so this follows
    /// [`Self::install_workspaces`] exactly rather than the content path: same
    /// directory convention, same `Skip`/`Force` semantics, same repository
    /// upsert.
    ///
    /// # Skip mode does NOT merge, and that is the difference from workspaces
    ///
    /// A workspace merges `allowed_node_types` additively in `Skip` mode
    /// because adding a permitted type cannot break anything already there. A
    /// rule cannot be merged that way: its `tasks` list and its `matcher` are
    /// one decision, and half a package's rule combined with half an
    /// operator's is a third rule neither of them wrote. So an existing id is
    /// left completely alone.
    ///
    /// That asymmetry is the point. An operator who narrowed a rule on their
    /// box — turned OCR off because it was too slow, pointed a matcher at one
    /// path — must not have it silently restored by the next `deploy --install`.
    /// `Force` overwrites, which is what a package author asks for explicitly.
    pub(super) async fn install_processing_rules(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // Collected synchronously first: a ZipFile is not Send, so it cannot be
        // held across the awaits below. Same shape as every sibling here.
        let mut rules_to_install: Vec<raisin_ai::ProcessingRule> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            if name.starts_with("processing-rules/")
                && (name.ends_with(".yaml") || name.ends_with(".yml"))
                && !file.is_dir()
            {
                let mut content = String::new();
                file.read_to_string(&mut content).map_err(|e| {
                    Error::storage(format!("Failed to read processing rule {}: {}", name, e))
                })?;

                // One file may hold a single rule or a list, because a package
                // usually wants a handful that only make sense together (pdf,
                // office, image) and splitting them across files hides the
                // ORDER, which is load-bearing: matching is first-match-wins.
                // Detect the shape from the first MEANINGFUL line, not the
                // first character. `trim_start` removes whitespace but not
                // comments, so a documented file — which is every file worth
                // shipping, including this package's own — begins with `#`,
                // took the single-rule branch, and failed with
                // "invalid type: sequence, expected struct".
                let looks_like_list = content
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && !line.starts_with('#'))
                    .is_some_and(|line| line.starts_with('-'));

                let parsed: Vec<raisin_ai::ProcessingRule> = if looks_like_list {
                    serde_yaml::from_str(&content).map_err(|e| {
                        Error::Validation(format!("Invalid processing rules YAML in {name}: {e}"))
                    })?
                } else {
                    vec![serde_yaml::from_str(&content).map_err(|e| {
                        Error::Validation(format!("Invalid processing rule YAML in {name}: {e}"))
                    })?]
                };

                for mut rule in parsed {
                    // An id is what `Skip` mode compares and what a later
                    // update targets, so an anonymous rule would install a
                    // duplicate on every deploy. Derive a stable one from the
                    // file name rather than minting a random id.
                    if rule.id.trim().is_empty() {
                        let stem = name
                            .rsplit('/')
                            .next()
                            .unwrap_or(&name)
                            .trim_end_matches(".yaml")
                            .trim_end_matches(".yml");
                        rule.id = stem.to_string();
                    }
                    if rule.name.trim().is_empty() {
                        rule.name = rule.id.clone();
                    }
                    rules_to_install.push(rule);
                }
            }
        }

        if rules_to_install.is_empty() {
            return Ok(());
        }

        let rules_repo = self.storage.processing_rules();
        let scope = RepoScope::new(tenant_id, repo_id);

        // Read-modify-write of ONE serialized set, so the whole batch is
        // resolved against a single snapshot and written once. Calling a
        // per-rule upsert in a loop would re-read and re-write the set for
        // every file.
        let mut set = rules_repo.get_rules(scope).await?.unwrap_or_default();

        for rule in rules_to_install {
            let rule_id = rule.id.clone();
            let exists = set.get_rule(&rule_id).is_some();

            if exists && install_mode == InstallMode::Skip {
                tracing::debug!(
                    job_id = %job_id,
                    rule_id = %rule_id,
                    "Processing rule already exists, leaving the operator's version alone (keep mode)"
                );
                stats.processing_rules_skipped += 1;
                continue;
            }

            if exists {
                set.remove_rule(&rule_id);
            }
            set.add_rule(rule);

            tracing::info!(
                job_id = %job_id,
                rule_id = %rule_id,
                overwritten = exists,
                "Installed processing rule from package"
            );
            stats.processing_rules_installed += 1;
        }

        if stats.processing_rules_installed > 0 {
            rules_repo.set_rules(scope, &set).await?;
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
            let was_existing = existing.is_some();

            // Keep (skip) mode: never overwrite an existing workspace, but additively
            // merge its `allowed_node_types` from the package (add-only, never remove)
            // so NodeTypes the package newly provides become usable without requiring
            // an explicit `workspace_patches` entry. The merge is skipped when the
            // existing workspace already allows everything (empty list or "*"), so we
            // never silently narrow an unrestricted workspace.
            if install_mode == InstallMode::Skip {
                if let Some(mut existing_ws) = existing {
                    let allows_all = existing_ws.allowed_node_types.is_empty()
                        || existing_ws.allowed_node_types.iter().any(|t| t == "*");

                    let mut modified = false;
                    if !allows_all {
                        for type_name in &workspace.allowed_node_types {
                            if type_name != "*"
                                && !existing_ws.allowed_node_types.contains(type_name)
                            {
                                existing_ws.allowed_node_types.push(type_name.clone());
                                modified = true;
                            }
                        }
                    }

                    if modified {
                        workspace_repo
                            .put(RepoScope::new(tenant_id, repo_id), existing_ws)
                            .await?;
                        tracing::debug!(
                            job_id = %job_id,
                            workspace = %ws_name,
                            "Merged allowed_node_types into existing workspace (keep mode)"
                        );
                        stats.patches_applied += 1;
                    } else {
                        tracing::debug!(
                            job_id = %job_id,
                            workspace = %ws_name,
                            "Workspace already exists, nothing to merge (keep mode)"
                        );
                        stats.workspaces_skipped += 1;
                    }
                    continue;
                }
                // No existing workspace — fall through and create it.
            }

            // Create (new workspace) or overwrite (force mode).
            workspace_repo
                .put(RepoScope::new(tenant_id, repo_id), workspace)
                .await?;

            tracing::debug!(
                job_id = %job_id,
                workspace = %ws_name,
                mode = ?install_mode,
                overwrite = was_existing,
                "Installed workspace"
            );

            stats.workspaces_installed += 1;
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

#[cfg(test)]
mod workspace_patch_tests {
    use super::super::manifest::PackageManifest;

    /// The google-drive-adapter manifest must declare a `raisin:system`
    /// workspace patch that adds both connector node types, so installing the
    /// connector into a pre-existing repo self-heals the system workspace's
    /// allowed_node_types (Phase 3 runs before content install in Phase 4).
    #[test]
    fn google_drive_manifest_declares_raisin_system_patch() {
        let yaml =
            include_str!("../../../../../../builtin-packages/google-drive-adapter/manifest.yaml");
        let manifest: PackageManifest =
            serde_yaml::from_str(yaml).expect("google-drive-adapter manifest must parse");

        let patches = manifest
            .workspace_patches
            .expect("manifest must declare workspace_patches");
        let system = patches
            .get("raisin:system")
            .expect("workspace_patches must contain a raisin:system key");
        let add = system
            .allowed_node_types
            .as_ref()
            .and_then(|a| a.add.as_ref())
            .expect("raisin:system patch must add allowed_node_types");

        assert!(
            add.iter().any(|t| t == "raisin:Integration"),
            "must add raisin:Integration"
        );
        assert!(
            add.iter().any(|t| t == "raisin:VirtualMount"),
            "must add raisin:VirtualMount"
        );
    }

    /// Mirrors the add-only merge in `apply_workspace_patches`: types the patch
    /// declares are appended to an existing workspace that did NOT allow them,
    /// without duplicating types that are already present.
    #[test]
    fn patch_adds_missing_types_to_existing_workspace() {
        let yaml =
            include_str!("../../../../../../builtin-packages/google-drive-adapter/manifest.yaml");
        let manifest: PackageManifest = serde_yaml::from_str(yaml).unwrap();
        let system = manifest
            .workspace_patches
            .unwrap()
            .remove("raisin:system")
            .unwrap();
        let add = system.allowed_node_types.unwrap().add.unwrap();

        // Existing workspace created under the old schema: has some types but
        // NOT the connector types the patch introduces.
        let mut allowed_node_types: Vec<String> = vec![
            "raisin:Folder".to_string(),
            "raisin:Integration".to_string(),
        ];

        for type_name in &add {
            if !allowed_node_types.contains(type_name) {
                allowed_node_types.push(type_name.clone());
            }
        }

        assert!(allowed_node_types.contains(&"raisin:VirtualMount".to_string()));
        // No duplicate for the already-present raisin:Integration.
        assert_eq!(
            allowed_node_types
                .iter()
                .filter(|t| *t == "raisin:Integration")
                .count(),
            1
        );
    }
}

/// A processing-rules file is a LIST or a single rule, and the file that ships
/// in the studio package is a list behind a header comment.
#[cfg(test)]
mod processing_rule_shape_tests {
    /// The detection used to be `content.trim_start().starts_with('-')`, which
    /// is false for any documented file: `trim_start` removes whitespace, not
    /// comments. Every such file took the single-rule branch and failed with
    /// "invalid type: sequence, expected struct" — so a package could not ship
    /// a commented rules file at all.
    fn looks_like_list(content: &str) -> bool {
        content
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .is_some_and(|line| line.starts_with('-'))
    }

    #[test]
    fn a_list_behind_a_header_comment_is_still_a_list() {
        let content = "# Asset-processing rules — the routing table.\n\
                       #\n\
                       # WHICH nodes get WHICH tasks.\n\
                       \n\
                       - id: studio-pdfs\n  order: 10\n";
        assert!(looks_like_list(content));
    }

    #[test]
    fn a_bare_list_is_a_list() {
        assert!(looks_like_list("- id: one\n"));
        assert!(looks_like_list("\n\n   - id: one\n"));
    }

    #[test]
    fn a_single_rule_is_not_a_list_however_it_is_introduced() {
        assert!(!looks_like_list("id: one\norder: 10\n"));
        assert!(!looks_like_list("# a single rule\nid: one\n"));
        assert!(!looks_like_list(""));
    }

    /// A `-` inside a comment must not make a single rule look like a list.
    #[test]
    fn a_dash_inside_a_comment_does_not_count() {
        assert!(!looks_like_list("# - not a rule, just prose\nid: one\n"));
    }
}
