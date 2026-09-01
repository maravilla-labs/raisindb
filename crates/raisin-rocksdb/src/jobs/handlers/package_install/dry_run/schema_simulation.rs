// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dry run simulation for schema entities (node types, archetypes, element types, workspaces)

use raisin_error::{Error, Result};
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::{Archetype, NodeType};
use raisin_models::workspace::Workspace;
use raisin_storage::scope::{BranchScope, RepoScope};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    ArchetypeRepository, ElementTypeRepository, NodeTypeRepository, ProcessingRulesRepository,
    Storage, WorkspaceRepository,
};
use std::io::{Cursor, Read};
use zip::ZipArchive;

use super::super::handler::PackageInstallHandler;
use super::super::types::{DryRunActionCounts, DryRunLogEntry, InstallMode};

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Dry run simulation for mixins
    pub(in crate::jobs::handlers::package_install) async fn dry_run_mixins(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let mixins_to_check: Vec<(String, String)> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if name.starts_with("mixins/") && name.ends_with(".yaml") && !file.is_dir() {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map_err(|e| {
                        Error::storage(format!("Failed to read mixin file {}: {}", name, e))
                    })?;

                    let node_type: NodeType = serde_yaml::from_str(&content).map_err(|e| {
                        Error::Validation(format!("Invalid mixin YAML in {}: {}", name, e))
                    })?;

                    items.push((node_type.name.clone(), name));
                }
            }
            items
        };

        let node_type_repo = self.storage.node_types();
        for (type_name, _file_name) in mixins_to_check {
            let existing = node_type_repo
                .get(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    &type_name,
                    None,
                )
                .await?;

            let (action, message) = Self::dry_run_action(
                existing.is_some(),
                install_mode,
                "Mixin",
                &type_name,
                counts,
            );

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "mixin".to_string(),
                path: type_name,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        Ok(())
    }

    /// Dry run simulation for node types
    pub(in crate::jobs::handlers::package_install) async fn dry_run_node_types(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let node_types_to_check: Vec<(String, String)> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if name.starts_with("nodetypes/") && name.ends_with(".yaml") && !file.is_dir() {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map_err(|e| {
                        Error::storage(format!("Failed to read node type file {}: {}", name, e))
                    })?;

                    let node_type: NodeType = serde_yaml::from_str(&content).map_err(|e| {
                        Error::Validation(format!("Invalid node type YAML in {}: {}", name, e))
                    })?;

                    items.push((node_type.name.clone(), name));
                }
            }
            items
        };

        let node_type_repo = self.storage.node_types();
        for (type_name, _file_name) in node_types_to_check {
            let existing = node_type_repo
                .get(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    &type_name,
                    None,
                )
                .await?;

            let (action, message) = Self::dry_run_action(
                existing.is_some(),
                install_mode,
                "Node type",
                &type_name,
                counts,
            );

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "node_type".to_string(),
                path: type_name,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        Ok(())
    }

    /// Dry run simulation for archetypes
    pub(in crate::jobs::handlers::package_install) async fn dry_run_archetypes(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let archetypes_to_check: Vec<String> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if name.starts_with("archetypes/") && name.ends_with(".yaml") && !file.is_dir() {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map_err(|e| {
                        Error::storage(format!("Failed to read archetype file {}: {}", name, e))
                    })?;

                    let archetype: Archetype = serde_yaml::from_str(&content).map_err(|e| {
                        Error::Validation(format!("Invalid archetype YAML in {}: {}", name, e))
                    })?;

                    items.push(archetype.name.clone());
                }
            }
            items
        };

        let archetype_repo = self.storage.archetypes();
        for archetype_name in archetypes_to_check {
            let existing = archetype_repo
                .get(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    &archetype_name,
                    None,
                )
                .await?;

            let (action, message) = Self::dry_run_action(
                existing.is_some(),
                install_mode,
                "Archetype",
                &archetype_name,
                counts,
            );

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "archetype".to_string(),
                path: archetype_name,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        Ok(())
    }

    /// Dry run simulation for element types
    pub(in crate::jobs::handlers::package_install) async fn dry_run_element_types(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let element_types_to_check: Vec<String> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if name.starts_with("elementtypes/") && name.ends_with(".yaml") && !file.is_dir() {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map_err(|e| {
                        Error::storage(format!("Failed to read element type file {}: {}", name, e))
                    })?;

                    let element_type: ElementType =
                        serde_yaml::from_str(&content).map_err(|e| {
                            Error::Validation(format!(
                                "Invalid element type YAML in {}: {}",
                                name, e
                            ))
                        })?;

                    items.push(element_type.name.clone());
                }
            }
            items
        };

        let element_type_repo = self.storage.element_types();
        for type_name in element_types_to_check {
            let existing = element_type_repo
                .get(
                    BranchScope::new(tenant_id, repo_id, "main"),
                    &type_name,
                    None,
                )
                .await?;

            let (action, message) = Self::dry_run_action(
                existing.is_some(),
                install_mode,
                "Element type",
                &type_name,
                counts,
            );

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "element_type".to_string(),
                path: type_name,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        Ok(())
    }

    /// Dry run simulation for asset-processing rules.
    ///
    /// Mirrors [`super::super::install_schema`]'s installer, including the
    /// single-or-list YAML shape and the filename-derived id. A dry run that
    /// parsed the file differently from the install would report an action that
    /// is not the one taken — worse than no preview, because it is believed.
    pub(in crate::jobs::handlers::package_install) async fn dry_run_processing_rules(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let rule_ids: Vec<String> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
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

                    let parsed: Vec<raisin_ai::ProcessingRule> =
                        if content.trim_start().starts_with('-') {
                            serde_yaml::from_str(&content).map_err(|e| {
                                Error::Validation(format!(
                                    "Invalid processing rules YAML in {name}: {e}"
                                ))
                            })?
                        } else {
                            vec![serde_yaml::from_str(&content).map_err(|e| {
                                Error::Validation(format!(
                                    "Invalid processing rule YAML in {name}: {e}"
                                ))
                            })?]
                        };

                    for rule in parsed {
                        if rule.id.trim().is_empty() {
                            let stem = name
                                .rsplit('/')
                                .next()
                                .unwrap_or(&name)
                                .trim_end_matches(".yaml")
                                .trim_end_matches(".yml");
                            items.push(stem.to_string());
                        } else {
                            items.push(rule.id.clone());
                        }
                    }
                }
            }
            items
        };

        if rule_ids.is_empty() {
            return Ok(());
        }

        let existing = self
            .storage
            .processing_rules()
            .get_rules(RepoScope::new(tenant_id, repo_id))
            .await?
            .unwrap_or_default();

        for rule_id in rule_ids {
            let (action, message) = Self::dry_run_action(
                existing.get_rule(&rule_id).is_some(),
                install_mode,
                "Processing rule",
                &rule_id,
                counts,
            );

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "processing_rule".to_string(),
                path: rule_id,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        Ok(())
    }

    /// Dry run simulation for workspaces
    pub(in crate::jobs::handlers::package_install) async fn dry_run_workspaces(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let workspaces_to_check: Vec<String> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if name.starts_with("workspaces/") && name.ends_with(".yaml") && !file.is_dir() {
                    let mut content = String::new();
                    file.read_to_string(&mut content).map_err(|e| {
                        Error::storage(format!("Failed to read workspace file {}: {}", name, e))
                    })?;

                    let workspace: Workspace = serde_yaml::from_str(&content).map_err(|e| {
                        Error::Validation(format!("Invalid workspace YAML in {}: {}", name, e))
                    })?;

                    items.push(workspace.name.clone());
                }
            }
            items
        };

        let workspace_repo = self.storage.workspaces();
        for ws_name in workspaces_to_check {
            let existing = workspace_repo
                .get(RepoScope::new(tenant_id, repo_id), &ws_name)
                .await?;

            let (action, message) = Self::dry_run_action(
                existing.is_some(),
                install_mode,
                "Workspace",
                &ws_name,
                counts,
            );

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "workspace".to_string(),
                path: ws_name,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        Ok(())
    }
}
