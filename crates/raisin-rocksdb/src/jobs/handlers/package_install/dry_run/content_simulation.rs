// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dry run simulation for content nodes and package assets

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::Storage;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use raisin_packages::namespace_encoding::decode_namespace;

use super::super::content_types::{
    derive_content_path, parse_asset_metadata_filename, resource_ref_filename, ContentNodeDef,
};
use super::super::handler::PackageInstallHandler;
use super::super::install_content::skill_md::{
    is_skill_md, refuse_skill_md_errors, skill_md_clashes, skill_md_to_node,
};
use super::super::translation::parse_translation_locale;
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
            // SKILL.md handling mirrors `collect_content_entries`: every YAML
            // definition's (workspace, file, node path), the parsed skills, and
            // the refusals — so a --check fails exactly where an install would.
            let mut yaml_keys: Vec<(String, String, String)> = Vec::new();
            let mut skill_keys: Vec<(String, String, String)> = Vec::new();
            let mut skill_items: Vec<ContentItem> = Vec::new();
            let mut skill_errors: Vec<String> = Vec::new();

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

                // Decode `_raisin__access_control` → `raisin:access_control`, the
                // same as the real collector. Without this the simulation
                // reported (and looked up) a workspace that does not exist, so
                // every node in a namespaced workspace previewed as "create"
                // no matter what was actually on the server.
                let workspace = decode_namespace(path_parts[1]);
                let filename = path_parts.last().unwrap_or(&"").to_string();

                // Read content for size estimation and parsing
                let mut content_bytes = Vec::new();
                file.read_to_end(&mut content_bytes)
                    .map_err(|e| Error::storage(format!("Failed to read file {}: {}", name, e)))?;

                // Node definitions: `.node.yaml` AND flat `{name}.yaml`, minus
                // translation overlays and asset metadata. This mirrors
                // `zip_collector::collect_content_entries`; previewing only
                // `.node.yaml` meant flat node files never appeared in a dry
                // run at all.
                //
                // `.node.yml` / `node.yml` count as folder definitions, as they
                // do in the collector (classification keys on `.yaml`, so the
                // spelling is normalised for those checks only).
                let is_folder_def_yml = filename == ".node.yml" || filename == "node.yml";
                let classify = match filename.strip_suffix(".yml") {
                    Some(stem) if is_folder_def_yml => format!("{stem}.yaml"),
                    _ => filename.clone(),
                };
                let is_node_def = (filename.ends_with(".yaml") || is_folder_def_yml)
                    && parse_translation_locale(&classify).is_none()
                    && parse_asset_metadata_filename(&classify).is_none();

                if is_skill_md(&filename) {
                    // A raisin:Skill node, never a binary asset — see
                    // `install_content::skill_md`.
                    match skill_md_to_node(&name, &content_bytes) {
                        Ok(node) => {
                            skill_keys.push((workspace.clone(), name.clone(), node.path.clone()));
                            skill_items.push(ContentItem::ContentNode {
                                workspace,
                                derived_name: node.name,
                                derived_path: node.path,
                            });
                        }
                        Err(reason) => skill_errors.push(reason),
                    }
                } else if is_node_def {
                    let content_def: ContentNodeDef = serde_yaml::from_slice(&content_bytes)
                        .map_err(|e| {
                            Error::Validation(format!("Invalid content YAML in {}: {}", name, e))
                        })?;

                    let derived_name = content_def.derive_name(&name);
                    let derived_path = derive_content_path(&name, &derived_name);
                    yaml_keys.push((workspace.clone(), name.clone(), derived_path.clone()));

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

            skill_errors.extend(skill_md_clashes(&skill_keys, &yaml_keys));
            refuse_skill_md_errors(skill_errors)?;
            items.extend(skill_items);

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
        // Same actor as the real install (node_installer.rs). Without it the
        // existence lookups below see nothing, and every existing node previews
        // as "create" — the dry run reported the opposite of what install did.
        tx.set_auth_context(AuthContext::system())?;

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
        // Same actor as the real install (node_installer.rs). Without it the
        // existence lookups below see nothing, and every existing node previews
        // as "create" — the dry run reported the opposite of what install did.
        tx.set_auth_context(AuthContext::system())?;

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
