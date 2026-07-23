// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dry run simulation for content nodes and package assets

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::Storage;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use super::super::content_types::{derive_content_path, resource_ref_filename, ContentNodeDef};
use super::super::handler::PackageInstallHandler;
use super::super::types::{
    resolve_install_policy_for_path, DryRunActionCounts, DryRunLogEntry, InstallMode,
};

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Dry run simulation for content nodes
    pub(in crate::jobs::handlers::package_install) async fn dry_run_content_nodes(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        install_mode: InstallMode,
        sync_config: Option<&raisin_packages::SyncConfig>,
        logs: &mut Vec<DryRunLogEntry>,
        content_counts: &mut DryRunActionCounts,
        binary_counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        // Item to check: either a content node or binary asset
        enum ContentItem {
            ContentNode {
                workspace: String,
                derived_name: String,
                derived_path: String,
            },
            BinaryAsset {
                workspace: String,
                asset_path: String,
                filename: String,
                size: usize,
            },
        }

        // Collect content items from archive first (sync operation).
        //
        // A binary that sits inside a content node's own directory and is
        // referenced by one of that node's authored Resource properties is
        // *folded into* the node (ingested + rebound at install time), not
        // counted as a separate binary asset — mirroring `build_content_entries`.
        let items_to_check: Vec<ContentItem> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            // (workspace, node_path) -> filenames referenced by authored Resources.
            let mut resource_refs: HashMap<(String, String), Vec<String>> = HashMap::new();
            // Deferred binaries: (workspace, asset_path, filename, size, containing_node_path)
            let mut binary_candidates: Vec<(String, String, String, usize, String)> = Vec::new();

            for i in 0..archive.len() {
                let mut file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if !name.starts_with("content/") || file.is_dir() {
                    continue;
                }

                let path_parts: Vec<&str> = name.split('/').collect();
                if path_parts.len() < 3 {
                    continue;
                }

                let workspace = path_parts[1].to_string();
                let filename = path_parts.last().unwrap_or(&"").to_string();

                // Read content for size estimation and parsing
                let mut content_bytes = Vec::new();
                file.read_to_end(&mut content_bytes)
                    .map_err(|e| Error::storage(format!("Failed to read file {}: {}", name, e)))?;

                if filename == ".node.yaml" {
                    let content_def: ContentNodeDef = serde_yaml::from_slice(&content_bytes)
                        .map_err(|e| {
                            Error::Validation(format!("Invalid content YAML in {}: {}", name, e))
                        })?;

                    let derived_name = content_def.derive_name(&name);
                    let derived_path = derive_content_path(&name, &derived_name);

                    if let Some(props) = &content_def.properties {
                        for value in props.values() {
                            if let PropertyValue::Resource(resource) = value {
                                if resource.is_external == Some(true) {
                                    continue;
                                }
                                if let Some(fname) = resource_ref_filename(resource) {
                                    resource_refs
                                        .entry((workspace.clone(), derived_path.clone()))
                                        .or_default()
                                        .push(fname);
                                }
                            }
                        }
                    }

                    items.push(ContentItem::ContentNode {
                        workspace,
                        derived_name,
                        derived_path,
                    });
                } else if !filename.starts_with('.') && !filename.ends_with(".yaml") {
                    let parent_path = if path_parts.len() > 3 {
                        path_parts[2..path_parts.len() - 1].join("/")
                    } else {
                        String::new()
                    };

                    let asset_path = if parent_path.is_empty() {
                        format!("/{}", filename)
                    } else {
                        format!("/{}/{}", parent_path, filename)
                    };
                    let containing_node_path = format!("/{}", parent_path);

                    binary_candidates.push((
                        workspace,
                        asset_path,
                        filename,
                        content_bytes.len(),
                        containing_node_path,
                    ));
                }
            }

            // Emit only binaries that don't bind to an authored Resource.
            for (workspace, asset_path, filename, size, containing_node_path) in binary_candidates {
                let is_bundled = resource_refs
                    .get(&(workspace.clone(), containing_node_path))
                    .is_some_and(|refs| refs.iter().any(|f| *f == filename));
                if is_bundled {
                    continue;
                }
                items.push(ContentItem::BinaryAsset {
                    workspace,
                    asset_path,
                    filename,
                    size,
                });
            }

            items
        };

        // Now do async checks
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant_id, repo_id)?;
        tx.set_branch(branch)?;

        for item in items_to_check {
            match item {
                ContentItem::ContentNode {
                    workspace,
                    derived_name,
                    derived_path,
                } => {
                    let existing = tx.get_node_by_path(&workspace, &derived_path).await?;
                    let policy = resolve_install_policy_for_path(
                        install_mode,
                        sync_config,
                        &workspace,
                        &derived_path,
                    );
                    let install_mode = policy.mode;

                    let (action, message) = match (existing.is_some(), install_mode) {
                        (true, InstallMode::Skip) => {
                            content_counts.skip += 1;
                            (
                                "skip",
                                format!(
                                    "Content node at '{}' already exists, will skip",
                                    derived_path
                                ),
                            )
                        }
                        (true, InstallMode::Sync) | (true, InstallMode::Overwrite) => {
                            content_counts.update += 1;
                            (
                                "update",
                                format!("Content node at '{}' exists, will update", derived_path),
                            )
                        }
                        (false, _) => {
                            content_counts.create += 1;
                            (
                                "create",
                                format!(
                                    "Content node '{}' will be created at {}",
                                    derived_name, derived_path
                                ),
                            )
                        }
                    };

                    logs.push(DryRunLogEntry {
                        level: action.to_string(),
                        category: "content".to_string(),
                        path: derived_path,
                        message,
                        action: action.to_string(),
                        policy: policy.reason,
                    });
                }
                ContentItem::BinaryAsset {
                    workspace,
                    asset_path,
                    filename,
                    size,
                } => {
                    let existing = tx.get_node_by_path(&workspace, &asset_path).await?;
                    let policy = resolve_install_policy_for_path(
                        install_mode,
                        sync_config,
                        &workspace,
                        &asset_path,
                    );
                    let install_mode = policy.mode;

                    let (action, message) = match (existing.is_some(), install_mode) {
                        (true, InstallMode::Skip) => {
                            binary_counts.skip += 1;
                            (
                                "skip",
                                format!(
                                    "Binary asset at '{}' already exists, will skip",
                                    asset_path
                                ),
                            )
                        }
                        (true, InstallMode::Sync) | (true, InstallMode::Overwrite) => {
                            binary_counts.update += 1;
                            (
                                "update",
                                format!("Binary asset at '{}' exists, will update", asset_path),
                            )
                        }
                        (false, _) => {
                            binary_counts.create += 1;
                            (
                                "create",
                                format!(
                                    "Binary asset '{}' will be created ({} bytes)",
                                    filename, size
                                ),
                            )
                        }
                    };

                    logs.push(DryRunLogEntry {
                        level: action.to_string(),
                        category: "binary".to_string(),
                        path: format!("{}{}", workspace, asset_path),
                        message,
                        action: action.to_string(),
                        policy: policy.reason,
                    });
                }
            }
        }

        // Don't commit - just drop the transaction
        Ok(())
    }

    /// Dry run simulation for package assets (README.md, static/)
    pub(in crate::jobs::handlers::package_install) async fn dry_run_package_assets(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        package_name: &str,
        install_mode: InstallMode,
        logs: &mut Vec<DryRunLogEntry>,
        counts: &mut DryRunActionCounts,
    ) -> Result<()> {
        let workspace = "packages";

        enum PackageAsset {
            Readme {
                asset_path: String,
            },
            Static {
                asset_path: String,
                relative_path: String,
            },
        }

        let assets_to_check: Vec<PackageAsset> = {
            let cursor = Cursor::new(zip_data);
            let mut archive = ZipArchive::new(cursor)
                .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

            let mut items = Vec::new();
            for i in 0..archive.len() {
                let file = archive
                    .by_index(i)
                    .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

                let name = file.name().to_string();

                if name.eq_ignore_ascii_case("readme.md") && !file.is_dir() {
                    let asset_path = format!("/{}/README.md", package_name);
                    items.push(PackageAsset::Readme { asset_path });
                } else if name.starts_with("static/") && !file.is_dir() {
                    let relative_path = name.strip_prefix("static/").unwrap_or(&name).to_string();
                    if !relative_path.is_empty() {
                        let asset_path = format!("/{}/static/{}", package_name, relative_path);
                        items.push(PackageAsset::Static {
                            asset_path,
                            relative_path,
                        });
                    }
                }
            }
            items
        };

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant_id, repo_id)?;
        tx.set_branch(branch)?;

        for asset in assets_to_check {
            let (category_name, asset_path) = match &asset {
                PackageAsset::Readme { asset_path } => ("README.md", asset_path.clone()),
                PackageAsset::Static {
                    asset_path,
                    relative_path,
                } => (relative_path.as_str(), asset_path.clone()),
            };

            let existing = tx.get_node_by_path(workspace, &asset_path).await?;

            let label = if matches!(asset, PackageAsset::Readme { .. }) {
                "README.md".to_string()
            } else {
                format!("Static asset '{}'", category_name)
            };
            let (action, message) =
                Self::dry_run_action(existing.is_some(), install_mode, &label, "", counts);

            logs.push(DryRunLogEntry {
                level: action.to_string(),
                category: "package_asset".to_string(),
                path: asset_path,
                message,
                action: action.to_string(),
                policy: None,
            });
        }

        // Don't commit - just drop the transaction
        Ok(())
    }
}
