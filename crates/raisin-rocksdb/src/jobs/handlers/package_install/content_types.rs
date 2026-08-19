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

use raisin_models::nodes::properties::value::Resource;
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
    /// Per-entry rejections, one message each.
    ///
    /// Content entries no longer abort the install on the first bad node, so
    /// this is how the operator learns *which* ones were refused and why.
    /// Non-empty means the install failed, but only after every other entry
    /// had its chance.
    pub content_errors: Vec<String>,
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
    /// Derive the node name from the file path.
    ///
    /// Priority:
    /// 1. Explicit top-level `name:` field
    /// 2. Parent folder name (for `node.yaml` / `.node.yaml` folder definitions)
    /// 3. Filename without extension (for `something.yaml` files)
    ///
    /// **`properties.name` is deliberately NOT consulted.** The file's location
    /// is structure; `properties.name` is content, and letting content decide
    /// structure produced a split-brain: this installer used to place
    /// `roles/author/.node.yaml` (with `properties.name: "Author"`) at
    /// `/roles/Author`, while the CLI live-sync mapper
    /// (`packages/raisindb-cli/src/sync/mapping.ts`) and the translation
    /// lookup ([`super::translation::derive_node_name_from_base_path`]) both
    /// derive it from the directory and target `/roles/author`. One source file
    /// became two nodes, and for a node type with a `unique: true` property
    /// (e.g. `raisin:Role.role_id`) the second write failed the unique check and
    /// took the whole install batch down with it.
    ///
    /// If you are tempted to re-add `properties.name` here, note that three
    /// other places already derive the same answer from the path alone; a
    /// fourth opinion is what caused the bug.
    pub fn derive_name(&self, file_path: &str) -> String {
        // 1. Explicit name field wins - the author said so directly.
        if let Some(name) = &self.name {
            return name.clone();
        }

        // 2/3. Otherwise the filesystem location is authoritative.
        Self::name_from_path(file_path)
    }

    /// The node name implied by a content file's location alone.
    ///
    /// `.../roles/author/.node.yaml` → `author`; `.../roles/editor.yaml` →
    /// `editor`. This is the same rule
    /// [`super::translation::derive_node_name_from_base_path`] applies.
    pub(super) fn name_from_path(file_path: &str) -> String {
        let path = std::path::Path::new(file_path);
        let filename = path.file_stem().unwrap_or_default().to_string_lossy();

        if filename == "node" || filename == ".node" {
            // For node.yaml or .node.yaml, use the parent folder name
            if let Some(parent) = path.parent() {
                if let Some(folder_name) = parent.file_name() {
                    return folder_name.to_string_lossy().to_string();
                }
            }
        }

        filename.to_string()
    }

    /// The name this definition would have produced under the pre-fix rule,
    /// when that differs from the name now derived from the path.
    ///
    /// Used by the installer to adopt a node an earlier install placed at the
    /// legacy path instead of creating a duplicate beside it. Returns `None`
    /// when an explicit `name:` was given (that always won, so nothing moved),
    /// when there is no `properties.name`, or when the two agree.
    pub(super) fn legacy_property_name(&self, file_path: &str) -> Option<String> {
        if self.name.is_some() {
            return None;
        }

        let legacy = match self.properties.as_ref()?.get("name")? {
            PropertyValue::String(name) => name.clone(),
            _ => return None,
        };

        if legacy == Self::name_from_path(file_path) {
            return None;
        }

        Some(legacy)
    }
}

#[cfg(test)]
mod derive_name_tests {
    use super::*;

    fn def(name: Option<&str>, prop_name: Option<&str>) -> ContentNodeDef {
        ContentNodeDef {
            id: None,
            node_type: "raisin:Role".to_string(),
            name: name.map(|s| s.to_string()),
            parent: None,
            archetype: None,
            properties: prop_name.map(|n| {
                let mut m = HashMap::new();
                m.insert("name".to_string(), PropertyValue::String(n.to_string()));
                m
            }),
        }
    }

    #[test]
    fn folder_name_wins_over_property_name() {
        // The regression this fix exists for: a role's display name must not
        // relocate the node. `raisin-auth` ships exactly this shape.
        let d = def(None, Some("Author"));
        assert_eq!(
            d.derive_name("content/_raisin__access_control/roles/author/.node.yaml"),
            "author"
        );
    }

    #[test]
    fn file_stem_wins_over_property_name() {
        let d = def(None, Some("Content Editor"));
        assert_eq!(
            d.derive_name("content/_raisin__access_control/roles/editor.yaml"),
            "editor"
        );
    }

    #[test]
    fn explicit_name_field_still_wins() {
        let d = def(Some("explicit"), Some("Author"));
        assert_eq!(
            d.derive_name("content/ws/roles/author/.node.yaml"),
            "explicit"
        );
    }

    #[test]
    fn undotted_node_yaml_also_uses_folder() {
        let d = def(None, None);
        assert_eq!(d.derive_name("content/ws/roles/viewer/node.yaml"), "viewer");
    }

    #[test]
    fn agrees_with_translation_lookup() {
        // These two must not drift: a translation overlay finds its base node
        // by path alone, so a name derived any other way is unreachable.
        for p in [
            "content/ws/roles/author/.node.yaml",
            "content/ws/roles/editor.yaml",
            "content/ws/about.yaml",
        ] {
            assert_eq!(
                def(None, Some("Display Name")).derive_name(p),
                super::super::translation::derive_node_name_from_base_path(p),
                "derive_name and derive_node_name_from_base_path disagree for {p}"
            );
        }
    }

    #[test]
    fn legacy_property_name_reports_the_moved_node() {
        let d = def(None, Some("Author"));
        assert_eq!(
            d.legacy_property_name("content/ws/roles/author/.node.yaml"),
            Some("Author".to_string())
        );
    }

    #[test]
    fn legacy_property_name_is_none_when_nothing_moved() {
        // No properties.name at all
        assert_eq!(
            def(None, None).legacy_property_name("content/ws/roles/author/.node.yaml"),
            None
        );
        // properties.name already equals the path-derived name
        assert_eq!(
            def(None, Some("author")).legacy_property_name("content/ws/roles/author/.node.yaml"),
            None
        );
        // explicit name: always won, so the node never sat elsewhere
        assert_eq!(
            def(Some("author"), Some("Author"))
                .legacy_property_name("content/ws/roles/author/.node.yaml"),
            None
        );
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

/// A binary shipped in the package that belongs to an authored node's
/// `Resource` property rather than becoming its own standalone `raisin:Asset`
/// child node.
///
/// Matched during collection: a binary file that sits inside a `NodeDef`'s own
/// directory and whose filename is referenced by one of that node's authored
/// `Resource` properties (via `storage_key`/`url`/`name`). On install the bytes
/// are ingested into blob storage and the authored `Resource` is rebound to the
/// resulting [`raisin_binary::StoredObject`] — see `rebind_resource`.
#[derive(Debug)]
pub(super) struct BundledBinary {
    /// Property name on the node holding the authored `Resource` to rebind.
    pub property: String,
    /// Original filename of the bundled binary (used for MIME/extension).
    pub filename: String,
    /// Raw bytes of the binary to ingest.
    pub data: Vec<u8>,
}

/// Content entry collected from package ZIP
#[derive(Debug)]
pub(super) enum ContentEntry {
    /// A node definition from YAML
    NodeDef {
        workspace: String,
        yaml_path: String,
        node: Box<Node>,
        /// Path this node occupied under the pre-fix naming rule, when that
        /// differs from `node.path`.
        ///
        /// Before `derive_name` was made path-authoritative, `properties.name`
        /// decided the node's location, so `roles/author/.node.yaml` with
        /// `properties.name: "Author"` installed to `/roles/Author`. Installing
        /// the same package again would now create a *second* node at
        /// `/roles/author` beside the first. The installer probes this path and
        /// adopts the node it finds instead. `None` when nothing moved.
        legacy_path: Option<String>,
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

/// Extract the bundle-relative filename an authored `Resource` refers to, so a
/// sibling binary in the package can be matched to the property it should
/// populate.
///
/// Prefers the `storage_key` metadata, then `url`, then `name`, and returns the
/// basename. Returns `None` if none is present.
pub(super) fn resource_ref_filename(resource: &Resource) -> Option<String> {
    let raw = resource
        .metadata
        .as_ref()
        .and_then(|m| m.get("storage_key"))
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| resource.url.clone())
        .or_else(|| resource.name.clone())?;

    let basename = raw.rsplit('/').next().unwrap_or(&raw);
    if basename.is_empty() {
        None
    } else {
        Some(basename.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::timestamp::StorageTimestamp;

    fn base_resource() -> Resource {
        Resource {
            uuid: "u".to_string(),
            name: None,
            size: None,
            mime_type: None,
            url: None,
            metadata: None,
            is_loaded: None,
            is_external: None,
            created_at: StorageTimestamp::now(),
            updated_at: StorageTimestamp::now(),
        }
    }

    #[test]
    fn ref_filename_prefers_storage_key_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "storage_key".to_string(),
            PropertyValue::String("logo-a1b2c3d4.png".to_string()),
        );
        let r = Resource {
            name: Some("ignored".to_string()),
            url: Some("also-ignored".to_string()),
            metadata: Some(metadata),
            ..base_resource()
        };
        assert_eq!(
            resource_ref_filename(&r),
            Some("logo-a1b2c3d4.png".to_string())
        );
    }

    #[test]
    fn ref_filename_falls_back_to_url_then_name() {
        let by_url = Resource {
            url: Some("hero.jpg".to_string()),
            ..base_resource()
        };
        assert_eq!(resource_ref_filename(&by_url), Some("hero.jpg".to_string()));

        let by_name = Resource {
            name: Some("doc.pdf".to_string()),
            ..base_resource()
        };
        assert_eq!(resource_ref_filename(&by_name), Some("doc.pdf".to_string()));
    }

    #[test]
    fn ref_filename_returns_basename_of_a_path() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "storage_key".to_string(),
            PropertyValue::String("nested/dir/file.bin".to_string()),
        );
        let r = Resource {
            metadata: Some(metadata),
            ..base_resource()
        };
        assert_eq!(resource_ref_filename(&r), Some("file.bin".to_string()));
    }

    #[test]
    fn ref_filename_none_when_unset() {
        assert_eq!(resource_ref_filename(&base_resource()), None);
    }

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
