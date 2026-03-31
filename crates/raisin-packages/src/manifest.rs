// SPDX-License-Identifier: BSL-1.1

//! Package manifest parsing and validation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use zip::ZipArchive;

use crate::error::{PackageError, PackageResult};
use crate::sync_config::SyncConfig;

/// Package manifest (manifest.yaml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Package name (unique identifier)
    pub name: String,

    /// Semantic version string
    pub version: String,

    /// Human-readable title
    #[serde(default)]
    pub title: Option<String>,

    /// Package description
    #[serde(default)]
    pub description: Option<String>,

    /// Package author
    #[serde(default)]
    pub author: Option<String>,

    /// License identifier
    #[serde(default)]
    pub license: Option<String>,

    /// Lucide icon name for UI display
    #[serde(default = "default_icon")]
    pub icon: String,

    /// Hex color for UI display
    #[serde(default = "default_color")]
    pub color: String,

    /// Keywords for search
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Package category
    #[serde(default)]
    pub category: Option<String>,

    /// Whether this is a builtin package (auto-installed on repository creation)
    #[serde(default)]
    pub builtin: Option<bool>,

    /// Package dependencies
    #[serde(default)]
    pub dependencies: Vec<Dependency>,

    /// What this package provides
    #[serde(default)]
    pub provides: Provides,

    /// Workspace patches
    #[serde(default)]
    pub workspace_patches: HashMap<String, WorkspacePatch>,

    /// Sync configuration for bidirectional synchronization
    #[serde(default)]
    pub sync: Option<SyncConfig>,
}

fn default_icon() -> String {
    "package".to_string()
}

fn default_color() -> String {
    "#6366F1".to_string()
}

/// Package dependency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Dependency package name
    pub name: String,

    /// Version requirement (semver)
    pub version: String,
}

/// What the package provides
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Provides {
    /// Node types provided
    #[serde(default)]
    pub nodetypes: Vec<String>,

    /// Mixins provided
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mixins: Vec<String>,

    /// Workspaces created or modified
    #[serde(default)]
    pub workspaces: Vec<String>,

    /// Content paths installed
    #[serde(default)]
    pub content: Vec<String>,
}

/// Workspace patch configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspacePatch {
    /// Node types to add to allowed_node_types
    #[serde(default)]
    pub allowed_node_types: AllowedNodeTypesPatch,

    /// Default node type for auto-created folders in this workspace
    /// If not specified, defaults to "raisin:Folder"
    #[serde(default)]
    pub default_folder_type: Option<String>,
}

/// Patch for allowed_node_types
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AllowedNodeTypesPatch {
    /// Node types to add
    #[serde(default)]
    pub add: Vec<String>,
}

impl Manifest {
    /// Parse manifest from YAML string
    pub fn from_yaml(yaml: &str) -> PackageResult<Self> {
        serde_yaml::from_str(yaml).map_err(PackageError::YamlError)
    }

    /// Parse manifest from bytes
    pub fn from_bytes(bytes: &[u8]) -> PackageResult<Self> {
        let yaml =
            std::str::from_utf8(bytes).map_err(|e| PackageError::InvalidManifest(e.to_string()))?;
        Self::from_yaml(yaml)
    }

    /// Extract manifest from a ZIP archive
    pub fn from_zip<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>) -> PackageResult<Self> {
        let mut manifest_file = zip
            .by_name("manifest.yaml")
            .map_err(|_| PackageError::ManifestNotFound)?;

        let mut contents = String::new();
        manifest_file
            .read_to_string(&mut contents)
            .map_err(PackageError::IoError)?;

        Self::from_yaml(&contents)
    }

    /// Validate the manifest
    pub fn validate(&self) -> PackageResult<()> {
        if self.name.is_empty() {
            return Err(PackageError::InvalidManifest("name is required".into()));
        }

        if self.version.is_empty() {
            return Err(PackageError::InvalidManifest("version is required".into()));
        }

        // Validate package name format (alphanumeric, hyphens, underscores)
        if !self
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(PackageError::InvalidManifest(
                "name must contain only alphanumeric characters, hyphens, and underscores".into(),
            ));
        }

        Ok(())
    }

    /// Convert manifest to properties for raisin:Package node
    pub fn to_node_properties(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "version": self.version,
            "title": self.title,
            "description": self.description,
            "author": self.author,
            "license": self.license,
            "icon": self.icon,
            "color": self.color,
            "keywords": self.keywords,
            "category": self.category,
            "dependencies": self.dependencies,
            "provides": self.provides,
            "workspace_patches": self.workspace_patches,
            "installed": false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest() {
        let yaml = r##"
name: ai-tools
version: 1.0.0
title: RaisinDB AI Tools
description: AI agents and chat support
author: RaisinDB Team
license: MIT
icon: bot
color: "#8B5CF6"
keywords:
  - ai
  - agents
  - chat
category: ai
dependencies:
  - name: core-functions
    version: ">=1.0.0"
provides:
  nodetypes:
    - ai:Agent
    - ai:Chat
  workspaces:
    - functions
  content:
    - functions/lib/raisin/agent-handler
workspace_patches:
  functions:
    allowed_node_types:
      add:
        - ai:Agent
        - ai:Chat
"##;

        let manifest = Manifest::from_yaml(yaml).unwrap();
        assert_eq!(manifest.name, "ai-tools");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.icon, "bot");
        assert_eq!(manifest.keywords.len(), 3);
        assert_eq!(manifest.provides.nodetypes.len(), 2);
        assert!(manifest.workspace_patches.contains_key("functions"));
    }

    #[test]
    fn test_validate_manifest() {
        let manifest = Manifest {
            name: "valid-package".into(),
            version: "1.0.0".into(),
            ..Default::default()
        };
        assert!(manifest.validate().is_ok());

        let invalid = Manifest {
            name: "".into(),
            version: "1.0.0".into(),
            ..Default::default()
        };
        assert!(invalid.validate().is_err());
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            title: None,
            description: None,
            author: None,
            license: None,
            icon: default_icon(),
            color: default_color(),
            keywords: Vec::new(),
            category: None,
            builtin: None,
            dependencies: Vec::new(),
            provides: Provides::default(),
            workspace_patches: HashMap::new(),
            sync: None,
        }
    }
}
