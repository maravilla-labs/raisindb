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

//! ZIP archive iteration and content entry collection
//!
//! Handles Phase 1 (raw file collection from ZIP) and Phase 2 (building
//! typed [`ContentEntry`] values from the raw data).

use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use raisin_storage::jobs::JobId;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use raisin_models::translations::LocaleOverlay;

use crate::jobs::handlers::package_install::content_types::{
    compute_content_hash, derive_content_path, parse_asset_metadata_filename, AssetFileDef,
    ContentEntry, ContentNodeDef,
};
use crate::jobs::handlers::package_install::handler::PackageInstallHandler;
use crate::jobs::handlers::package_install::translation::{
    derive_base_node_path, parse_translation_locale, yaml_to_overlay,
};

/// Intermediate struct for collecting raw entries from ZIP
pub(in crate::jobs::handlers::package_install) struct CollectedEntries {
    pub yaml_nodes: Vec<(String, String, ContentNodeDef)>,
    pub binary_files: Vec<(String, String, String, Vec<u8>)>,
    /// Translation files: (workspace, yaml_path, locale, overlay)
    pub translation_files: Vec<(String, String, String, LocaleOverlay)>,
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Collect raw content entries from ZIP archive
    ///
    /// Iterates every file inside the `content/` directory of the archive and
    /// categorises each entry as one of:
    /// - YAML node definition (`.yaml`)
    /// - Asset metadata file (`.node.{filename}.yaml`)
    /// - Binary file (everything else, excluding hidden files)
    pub(in crate::jobs::handlers::package_install) fn collect_content_entries(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        job_id: &JobId,
    ) -> Result<(CollectedEntries, HashMap<String, AssetFileDef>)> {
        let mut yaml_nodes: Vec<(String, String, ContentNodeDef)> = Vec::new();
        let mut binary_files: Vec<(String, String, String, Vec<u8>)> = Vec::new();
        let mut asset_metadata: HashMap<String, AssetFileDef> = HashMap::new();
        let mut translation_files: Vec<(String, String, String, LocaleOverlay)> = Vec::new();

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Only process content/ directory
            if !name.starts_with("content/") || file.is_dir() {
                continue;
            }

            // Parse path structure: content/{workspace}/{path...}/{file}
            let path_parts: Vec<&str> = name.split('/').collect();
            if path_parts.len() < 3 {
                tracing::warn!(
                    job_id = %job_id,
                    file = %name,
                    "Invalid content path structure, skipping"
                );
                continue;
            }

            // Decode namespace encoding: _raisin__access_control → raisin:access_control
            let raw_ws = path_parts[1];
            let workspace = if raw_ws.starts_with('_') && !raw_ws.starts_with("__") {
                if let Some(pos) = raw_ws[1..].find("__") {
                    format!("{}:{}", &raw_ws[1..pos + 1], &raw_ws[pos + 3..])
                } else {
                    raw_ws.to_string()
                }
            } else {
                raw_ws.to_string()
            };
            let filename = path_parts.last().unwrap_or(&"").to_string();

            // Build parent path (everything between workspace and filename)
            let parent_path = if path_parts.len() > 3 {
                path_parts[2..path_parts.len() - 1].join("/")
            } else {
                String::new()
            };

            // Read file content
            let mut content_bytes = Vec::new();
            file.read_to_end(&mut content_bytes)
                .map_err(|e| Error::storage(format!("Failed to read file {}: {}", name, e)))?;

            if filename.ends_with(".yaml") {
                // Check if this is a translation file (.node.{locale}.yaml or {name}.{locale}.yaml)
                if let Some(locale) = parse_translation_locale(&filename) {
                    let content = String::from_utf8(content_bytes)
                        .map_err(|_| Error::Validation(format!("Invalid UTF-8 in {}", name)))?;
                    let json_value: serde_json::Value =
                        serde_yaml::from_str(&content).map_err(|e| {
                            Error::Validation(format!(
                                "Invalid translation YAML in {}: {}",
                                name, e
                            ))
                        })?;
                    match yaml_to_overlay(json_value) {
                        Ok(overlay) => {
                            tracing::debug!(
                                job_id = %job_id,
                                file = %name,
                                locale = %locale,
                                "Found translation file"
                            );
                            translation_files.push((workspace, name.clone(), locale, overlay));
                        }
                        Err(e) => {
                            tracing::warn!(
                                job_id = %job_id,
                                file = %name,
                                error = %e,
                                "Failed to parse translation file, skipping"
                            );
                        }
                    }
                } else if let Some(target_filename) = parse_asset_metadata_filename(&filename) {
                    // Check if this is asset metadata (.node.{filename}.yaml)
                    // This is metadata for an associated file
                    let metadata: AssetFileDef =
                        serde_yaml::from_slice(&content_bytes).map_err(|e| {
                            Error::Validation(format!(
                                "Invalid asset metadata YAML in {}: {}",
                                name, e
                            ))
                        })?;
                    let key = format!("{}/{}/{}", workspace, parent_path, target_filename);
                    asset_metadata.insert(key, metadata);
                    tracing::debug!(
                        job_id = %job_id,
                        file = %name,
                        target_filename = %target_filename,
                        "Found asset metadata file"
                    );
                } else {
                    // Regular node definition (.node.yaml or {name}.yaml)
                    let content = String::from_utf8(content_bytes)
                        .map_err(|_| Error::Validation(format!("Invalid UTF-8 in {}", name)))?;
                    let content_def: ContentNodeDef =
                        serde_yaml::from_str(&content).map_err(|e| {
                            Error::Validation(format!("Invalid content YAML in {}: {}", name, e))
                        })?;
                    yaml_nodes.push((workspace, name.clone(), content_def));
                }
            } else if !filename.starts_with('.') {
                // Binary file (skip hidden files other than .node.*.yaml which we already handled)
                binary_files.push((workspace, parent_path, filename, content_bytes));
            }
        }

        Ok((
            CollectedEntries {
                yaml_nodes,
                binary_files,
                translation_files,
            },
            asset_metadata,
        ))
    }

    /// Build [`ContentEntry`] list from collected raw entries
    ///
    /// Converts YAML definitions into `ContentEntry::NodeDef` and binary files
    /// into `ContentEntry::BinaryFile`, then sorts by path depth so parent
    /// folders are created before their children.
    pub(in crate::jobs::handlers::package_install) fn build_content_entries(
        &self,
        collected: CollectedEntries,
        mut asset_metadata: HashMap<String, AssetFileDef>,
        job_id: &JobId,
    ) -> Result<Vec<ContentEntry>> {
        let mut entries: Vec<ContentEntry> = Vec::new();

        // Add YAML-defined nodes
        for (workspace, yaml_path, content_def) in collected.yaml_nodes {
            let derived_name = content_def.derive_name(&yaml_path);
            let derived_path = derive_content_path(&yaml_path, &derived_name);

            tracing::debug!(
                job_id = %job_id,
                file = %yaml_path,
                node_name = %derived_name,
                node_path = %derived_path,
                workspace = %workspace,
                "Derived content node path from file structure"
            );

            let node = Node {
                id: content_def.id.unwrap_or_else(|| nanoid::nanoid!()),
                node_type: content_def.node_type,
                name: derived_name,
                path: derived_path,
                workspace: Some(workspace.clone()),
                parent: content_def.parent,
                archetype: content_def.archetype,
                properties: content_def.properties.unwrap_or_default(),
                ..Default::default()
            };

            entries.push(ContentEntry::NodeDef {
                workspace,
                yaml_path,
                node: Box::new(node),
            });
        }

        // Add binary files as asset entries
        for (workspace, parent_path, filename, data) in collected.binary_files {
            let metadata_key = format!("{}/{}/{}", workspace, parent_path, filename);
            let metadata = asset_metadata.remove(&metadata_key);
            let content_hash = compute_content_hash(&data);
            let zip_path = if parent_path.is_empty() {
                format!("content/{}/{}", workspace, filename)
            } else {
                format!("content/{}/{}/{}", workspace, parent_path, filename)
            };

            tracing::debug!(
                job_id = %job_id,
                file = %zip_path,
                has_metadata = metadata.is_some(),
                hash = %content_hash,
                "Found binary file for asset creation"
            );

            entries.push(ContentEntry::BinaryFile {
                workspace,
                zip_path,
                filename,
                parent_path,
                data,
                metadata: Box::new(metadata),
                content_hash,
            });
        }

        // Add translation file entries (these must come after NodeDefs)
        for (workspace, yaml_path, locale, overlay) in collected.translation_files {
            let base_node_yaml_path = derive_base_node_path(&yaml_path);
            tracing::debug!(
                job_id = %job_id,
                file = %yaml_path,
                locale = %locale,
                base_path = %base_node_yaml_path,
                "Adding translation entry"
            );
            entries.push(ContentEntry::TranslationFile {
                workspace,
                base_node_yaml_path,
                locale,
                overlay,
            });
        }

        // Topological sort: referenced nodes first, circular refs flagged for two-pass
        let sorted = super::reference_sort::sort_by_references(entries);

        if !sorted.circular.is_empty() {
            tracing::warn!(
                job_id = %job_id,
                count = sorted.circular.len(),
                "Found nodes with circular references — will use two-pass install"
            );
        }

        let mut result = Vec::with_capacity(
            sorted.ordered.len() + sorted.circular.len() + sorted.other.len(),
        );
        result.extend(sorted.ordered);
        result.extend(sorted.circular);
        result.extend(sorted.other);

        Ok(result)
    }
}
