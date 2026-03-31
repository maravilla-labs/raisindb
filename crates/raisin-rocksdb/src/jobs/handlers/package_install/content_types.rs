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

//! Internal content types and utility functions for package installation

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum nesting depth for .rap files within .rap files
pub(super) const MAX_NESTING_DEPTH: usize = 3;

/// Batch size for committing content nodes (commit every N nodes)
pub(super) const CONTENT_BATCH_SIZE: usize = 100;

/// Statistics for installation operation
#[derive(Default)]
pub(super) struct InstallStats {
    pub mixins_installed: usize,
    pub mixins_skipped: usize,
    pub node_types_installed: usize,
    pub node_types_skipped: usize,
    pub archetypes_installed: usize,
    pub archetypes_skipped: usize,
    pub element_types_installed: usize,
    pub element_types_skipped: usize,
    pub workspaces_installed: usize,
    pub workspaces_skipped: usize,
    pub patches_applied: usize,
    pub content_nodes_created: usize,
    pub content_nodes_skipped: usize,
    pub content_nodes_synced: usize,
    pub nested_packages_installed: usize,
    pub binary_files_installed: usize,
    pub package_assets_installed: usize,
    pub translations_applied: usize,
    pub translations_skipped: usize,
}

/// Content node definition from YAML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ContentNodeDef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub node_type: String,
    /// Node name - optional, derived from file path if not provided
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent node name (not full path)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Optional archetype for specialized rendering
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, PropertyValue>>,
}

impl ContentNodeDef {
    /// Derive node name from file path if not explicitly set.
    /// Priority:
    /// 1. Explicit `name` field
    /// 2. `properties.name` (for nodes where name is a property)
    /// 3. Parent folder name (for `node.yaml` or `.node.yaml` folder definition files)
    /// 4. Filename without extension (for `something.yaml` files)
    pub fn derive_name(&self, file_path: &str) -> String {
        // 1. Check explicit name field
        if let Some(name) = &self.name {
            return name.clone();
        }

        // 2. Check properties.name
        if let Some(props) = &self.properties {
            if let Some(PropertyValue::String(name)) = props.get("name") {
                return name.clone();
            }
        }

        // 3. Derive from file path
        let path = std::path::Path::new(file_path);
        let filename = path.file_stem().unwrap_or_default().to_string_lossy();

        if filename == "node" || filename == ".node" {
            // For node.yaml or .node.yaml, use parent folder name
            if let Some(parent) = path.parent() {
                if let Some(folder_name) = parent.file_name() {
                    return folder_name.to_string_lossy().to_string();
                }
            }
        }

        // 4. Use filename without extension
        filename.to_string()
    }
}

/// Asset file definition from YAML
/// Pattern: `.node.{filename}.{ext}.yaml` provides metadata for `{filename}.{ext}`
/// Example: `.node.index.js.yaml` defines metadata for `index.js`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AssetFileDef {
    /// Node type (defaults to raisin:Asset)
    #[serde(default = "default_asset_type")]
    pub node_type: String,
    /// Title (defaults to filename)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Additional properties
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, PropertyValue>>,
}

fn default_asset_type() -> String {
    "raisin:Asset".to_string()
}

impl Default for AssetFileDef {
    fn default() -> Self {
        Self {
            node_type: default_asset_type(),
            title: None,
            description: None,
            properties: None,
        }
    }
}

/// Content entry collected from package ZIP
#[derive(Debug)]
pub(super) enum ContentEntry {
    /// A node definition from YAML
    NodeDef {
        workspace: String,
        yaml_path: String,
        node: Box<Node>,
    },
    /// A binary file to be stored as raisin:Asset
    BinaryFile {
        workspace: String,
        zip_path: String,
        filename: String,
        parent_path: String,
        data: Vec<u8>,
        metadata: Box<Option<AssetFileDef>>,
        content_hash: String,
    },
    /// A translation file for a content node
    TranslationFile {
        workspace: String,
        base_node_yaml_path: String,
        locale: String,
        overlay: LocaleOverlay,
    },
}

/// Parse filename from asset metadata YAML pattern
/// `.node.index.js.yaml` -> Some("index.js")
/// `.node.yaml` -> None (folder definition, not asset metadata)
pub(super) fn parse_asset_metadata_filename(yaml_filename: &str) -> Option<String> {
    const PREFIX: &str = ".node.";
    const SUFFIX: &str = ".yaml";
    const MIN_LEN: usize = PREFIX.len() + SUFFIX.len() + 1;

    if yaml_filename.len() < MIN_LEN {
        return None;
    }

    if yaml_filename.starts_with(PREFIX) && yaml_filename.ends_with(SUFFIX) {
        let inner = &yaml_filename[PREFIX.len()..yaml_filename.len() - SUFFIX.len()];
        if !inner.is_empty() {
            return Some(inner.to_string());
        }
    }
    None
}

/// Compute SHA-256 hash of binary data for change detection
pub(super) fn compute_content_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Derive node path from content file path structure.
///
/// Examples:
/// - `content/functions/mynode.yaml` -> `/mynode`
/// - `content/functions/agents/.node.yaml` -> `/agents`
pub(super) fn derive_content_path(file_path: &str, node_name: &str) -> String {
    let parts: Vec<&str> = file_path.split('/').collect();

    if parts.len() <= 3 {
        return format!("/{}", node_name);
    }

    let filename = parts.last().unwrap_or(&"");
    let is_folder_node = filename.starts_with(".node");
    let path_dirs = &parts[2..parts.len() - 1];

    if path_dirs.is_empty() {
        format!("/{}", node_name)
    } else if is_folder_node {
        if path_dirs.len() == 1 {
            format!("/{}", node_name)
        } else {
            format!(
                "/{}/{}",
                path_dirs[..path_dirs.len() - 1].join("/"),
                node_name
            )
        }
    } else {
        format!("/{}/{}", path_dirs.join("/"), node_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_asset_metadata_filename() {
        assert_eq!(
            parse_asset_metadata_filename(".node.index.js.yaml"),
            Some("index.js".to_string())
        );
        assert_eq!(
            parse_asset_metadata_filename(".node.script.ts.yaml"),
            Some("script.ts".to_string())
        );
        assert_eq!(parse_asset_metadata_filename(".node.yaml"), None);
        assert_eq!(parse_asset_metadata_filename(""), None);
        assert_eq!(parse_asset_metadata_filename("node.index.js.yaml"), None);
        assert_eq!(parse_asset_metadata_filename(".node.index.js.yml"), None);
    }
}
