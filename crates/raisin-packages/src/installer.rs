// SPDX-License-Identifier: BSL-1.1

//! Package installer - install and uninstall packages to repositories

use std::collections::HashMap;

use chrono::Utc;

use crate::browser::PackageBrowser;
use crate::error::{PackageError, PackageResult};
use crate::manifest::Manifest;
use crate::patcher::WorkspacePatcher;

/// Result of a package installation
#[derive(Debug)]
pub struct InstallResult {
    /// Package name
    pub package_name: String,

    /// Package version
    pub package_version: String,

    /// Mixins registered (installed before node types)
    pub mixins_registered: Vec<String>,

    /// Node types registered
    pub node_types_registered: Vec<String>,

    /// Workspaces patched
    pub workspaces_patched: Vec<String>,

    /// Content nodes created
    pub content_nodes_created: Vec<String>,

    /// Installation timestamp
    pub installed_at: chrono::DateTime<Utc>,
}

/// Result of a package uninstallation
#[derive(Debug)]
pub struct UninstallResult {
    /// Package name
    pub package_name: String,

    /// Content nodes removed
    pub content_nodes_removed: Vec<String>,

    /// Whether node types were removed (false if other packages use them)
    pub node_types_removed: bool,
}

/// Content node to be created during installation
#[derive(Debug, Clone)]
pub struct ContentNode {
    /// Workspace to create in
    pub workspace: String,

    /// Path within the workspace
    pub path: String,

    /// Node type
    pub node_type: String,

    /// Node properties
    pub properties: serde_json::Value,

    /// Child nodes
    pub children: Vec<ContentNode>,

    /// Associated files (for functions with code files)
    pub files: HashMap<String, Vec<u8>>,
}

/// Package installer handles installation and uninstallation
pub struct PackageInstaller {
    /// The package data (ZIP bytes)
    package_data: Vec<u8>,

    /// Browser for the package
    browser: PackageBrowser,

    /// Parsed manifest
    manifest: Manifest,
}

impl PackageInstaller {
    /// Create a new installer from package data
    pub fn new(package_data: Vec<u8>) -> PackageResult<Self> {
        let browser = PackageBrowser::new(package_data.clone());
        let manifest = browser.manifest()?;
        manifest.validate()?;

        Ok(Self {
            package_data,
            browser,
            manifest,
        })
    }

    /// Get the package manifest
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get the package browser
    pub fn browser(&self) -> &PackageBrowser {
        &self.browser
    }

    /// Get mixin definitions from the package.
    /// Mixins should be installed BEFORE node types, since node types may reference them.
    pub fn get_mixins(&self) -> PackageResult<Vec<(String, serde_json::Value)>> {
        let mut mixins = Vec::new();

        for path in self.browser.list_mixins()? {
            let content = self.browser.read_file_string(&path)?;
            let parsed: serde_json::Value = serde_yaml::from_str(&content).map_err(|e| {
                PackageError::InvalidManifest(format!("Invalid mixin {}: {}", path, e))
            })?;

            let name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PackageError::InvalidManifest(format!("Missing name in {}", path)))?
                .to_string();

            mixins.push((name, parsed));
        }

        Ok(mixins)
    }

    /// Get node type definitions from the package
    pub fn get_node_types(&self) -> PackageResult<Vec<(String, serde_json::Value)>> {
        let mut node_types = Vec::new();

        for path in self.browser.list_nodetypes()? {
            let content = self.browser.read_file_string(&path)?;
            let parsed: serde_json::Value = serde_yaml::from_str(&content).map_err(|e| {
                PackageError::InvalidManifest(format!("Invalid nodetype {}: {}", path, e))
            })?;

            let name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PackageError::InvalidManifest(format!("Missing name in {}", path)))?
                .to_string();

            node_types.push((name, parsed));
        }

        Ok(node_types)
    }

    /// Get workspace patcher
    pub fn get_patcher(&self) -> WorkspacePatcher {
        WorkspacePatcher::from_manifest_patches(&self.manifest.workspace_patches)
    }

    /// Parse content from the package
    pub fn get_content(&self) -> PackageResult<Vec<ContentNode>> {
        let mut content_nodes = Vec::new();

        // List content workspaces
        let workspaces = self.browser.list_content_workspaces()?;

        for workspace in workspaces {
            let workspace_content = self.parse_workspace_content(&workspace)?;
            content_nodes.extend(workspace_content);
        }

        Ok(content_nodes)
    }

    /// Parse content for a specific workspace
    fn parse_workspace_content(&self, workspace: &str) -> PackageResult<Vec<ContentNode>> {
        let encoded_ws = crate::namespace_encoding::encode_namespace(workspace);
        let base_path = format!("content/{}/", encoded_ws);
        self.parse_directory_content(workspace, &base_path, "")
    }

    /// Recursively parse directory content
    fn parse_directory_content(
        &self,
        workspace: &str,
        base_path: &str,
        relative_path: &str,
    ) -> PackageResult<Vec<ContentNode>> {
        let full_path = format!("{}{}", base_path, relative_path);
        let entries = self.browser.list_directory(&full_path)?;

        let mut nodes = Vec::new();

        for entry in entries {
            if entry.entry_type == crate::browser::EntryType::Directory {
                // Check if this directory is a node (has node.yaml)
                let node_yaml_path = format!("{}node.yaml", entry.path);
                if self.browser.exists(&node_yaml_path)? {
                    let node = self.parse_node_directory(workspace, &entry.path)?;
                    nodes.push(node);
                } else {
                    // Just a regular directory, recurse into it
                    let sub_relative = entry.path.trim_start_matches(base_path);
                    let sub_nodes =
                        self.parse_directory_content(workspace, base_path, sub_relative)?;
                    nodes.extend(sub_nodes);
                }
            }
        }

        Ok(nodes)
    }

    /// Parse a directory that represents a node
    fn parse_node_directory(&self, workspace: &str, dir_path: &str) -> PackageResult<ContentNode> {
        let node_yaml_path = format!("{}node.yaml", dir_path);
        let node_yaml = self.browser.read_file_string(&node_yaml_path)?;

        let node_def: serde_json::Value = serde_yaml::from_str(&node_yaml).map_err(|e| {
            PackageError::InvalidManifest(format!("Invalid node.yaml in {}: {}", dir_path, e))
        })?;

        let node_type = node_def
            .get("node_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                PackageError::InvalidManifest(format!("Missing node_type in {}", dir_path))
            })?
            .to_string();

        let properties = node_def
            .get("properties")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Extract path from directory name
        let node_name = dir_path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("");

        // Collect associated files (for functions, these are code files)
        let mut files = HashMap::new();
        let entries = self.browser.list_directory(dir_path)?;

        for entry in entries {
            if entry.entry_type == crate::browser::EntryType::File {
                let filename = entry.path.trim_start_matches(dir_path).to_string();

                // Skip node.yaml and hidden files
                if filename == "node.yaml" || filename.starts_with('.') {
                    continue;
                }

                let file_content = self.browser.read_file(&entry.path)?;
                files.insert(filename, file_content);
            }
        }

        // TODO: Parse children from node_def or subdirectories

        Ok(ContentNode {
            workspace: workspace.to_string(),
            path: node_name.to_string(),
            node_type,
            properties,
            children: Vec::new(),
            files,
        })
    }

    /// Generate install result (after external installation logic)
    pub fn create_install_result(
        &self,
        mixins_registered: Vec<String>,
        node_types_registered: Vec<String>,
        workspaces_patched: Vec<String>,
        content_nodes_created: Vec<String>,
    ) -> InstallResult {
        InstallResult {
            package_name: self.manifest.name.clone(),
            package_version: self.manifest.version.clone(),
            mixins_registered,
            node_types_registered,
            workspaces_patched,
            content_nodes_created,
            installed_at: Utc::now(),
        }
    }

    /// Generate uninstall result
    pub fn create_uninstall_result(
        &self,
        content_nodes_removed: Vec<String>,
        node_types_removed: bool,
    ) -> UninstallResult {
        UninstallResult {
            package_name: self.manifest.name.clone(),
            content_nodes_removed,
            node_types_removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn create_test_package() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let options = SimpleFileOptions::default();

            // Add manifest
            zip.start_file("manifest.yaml", options).unwrap();
            zip.write_all(
                br#"
name: test-package
version: 1.0.0
provides:
  nodetypes:
    - test:Type
  content:
    - default/test-node
"#,
            )
            .unwrap();

            // Add a nodetype
            zip.add_directory("nodetypes/", options).unwrap();
            zip.start_file("nodetypes/test_type.yaml", options).unwrap();
            zip.write_all(
                br#"
name: test:Type
description: Test type
"#,
            )
            .unwrap();

            // Add content
            zip.add_directory("content/", options).unwrap();
            zip.add_directory("content/default/", options).unwrap();
            zip.add_directory("content/default/test-node/", options)
                .unwrap();
            zip.start_file("content/default/test-node/node.yaml", options)
                .unwrap();
            zip.write_all(
                br#"
node_type: test:Type
properties:
  title: Test Node
"#,
            )
            .unwrap();
            zip.start_file("content/default/test-node/index.js", options)
                .unwrap();
            zip.write_all(b"export function handler() { return 'hello'; }")
                .unwrap();

            zip.finish().unwrap();
        }
        buf
    }

    #[test]
    fn test_installer_creation() {
        let data = create_test_package();
        let installer = PackageInstaller::new(data).unwrap();

        assert_eq!(installer.manifest().name, "test-package");
        assert_eq!(installer.manifest().version, "1.0.0");
    }

    #[test]
    fn test_get_node_types() {
        let data = create_test_package();
        let installer = PackageInstaller::new(data).unwrap();

        let node_types = installer.get_node_types().unwrap();
        assert_eq!(node_types.len(), 1);
        assert_eq!(node_types[0].0, "test:Type");
    }

    #[test]
    fn test_get_mixins() {
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let options = SimpleFileOptions::default();

            zip.start_file("manifest.yaml", options).unwrap();
            zip.write_all(
                br#"
name: mixin-package
version: 1.0.0
provides:
  mixins:
    - raisin:publishable
  nodetypes:
    - test:Article
"#,
            )
            .unwrap();

            // Add a mixin
            zip.add_directory("mixins/", options).unwrap();
            zip.start_file("mixins/raisin_publishable.yaml", options)
                .unwrap();
            zip.write_all(
                br#"
name: raisin:publishable
description: Adds publish workflow properties
"#,
            )
            .unwrap();

            // Add a nodetype that references the mixin
            zip.add_directory("nodetypes/", options).unwrap();
            zip.start_file("nodetypes/test_article.yaml", options)
                .unwrap();
            zip.write_all(
                br#"
name: test:Article
description: Article type
mixins:
  - raisin:publishable
"#,
            )
            .unwrap();

            zip.finish().unwrap();
        }

        let installer = PackageInstaller::new(buf).unwrap();

        let mixins = installer.get_mixins().unwrap();
        assert_eq!(mixins.len(), 1);
        assert_eq!(mixins[0].0, "raisin:publishable");

        let node_types = installer.get_node_types().unwrap();
        assert_eq!(node_types.len(), 1);
        assert_eq!(node_types[0].0, "test:Article");

        assert_eq!(installer.manifest().provides.mixins.len(), 1);
        assert_eq!(
            installer.manifest().provides.mixins[0],
            "raisin:publishable"
        );
    }

    #[test]
    fn test_get_mixins_empty_package() {
        let data = create_test_package();
        let installer = PackageInstaller::new(data).unwrap();

        // Package without mixins/ directory should return empty vec
        let mixins = installer.get_mixins().unwrap();
        assert!(mixins.is_empty());
    }

    #[test]
    fn test_get_content() {
        let data = create_test_package();
        let installer = PackageInstaller::new(data).unwrap();

        let content = installer.get_content().unwrap();
        assert!(!content.is_empty());

        let node = &content[0];
        assert_eq!(node.workspace, "default");
        assert_eq!(node.node_type, "test:Type");
        assert!(node.files.contains_key("index.js"));
    }
}
