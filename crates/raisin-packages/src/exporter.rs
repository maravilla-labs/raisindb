// SPDX-License-Identifier: BSL-1.1

//! Package exporter - export installed content as .rap packages
//!
//! This module provides functionality to export the current state of
//! installed package content back to a .rap (Raisin Archive Package) file.

use std::collections::HashMap;
use std::io::{Cursor, Write};

use chrono::Utc;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::error::{PackageError, PackageResult};
use crate::manifest::Manifest;
use crate::sync::{ExportMode, ExportOptions};

/// Result of a package export operation
#[derive(Debug)]
pub struct ExportResult {
    /// Package name
    pub package_name: String,

    /// Package version
    pub package_version: String,

    /// The exported .rap file as bytes
    pub package_data: Vec<u8>,

    /// Number of files included
    pub files_included: usize,

    /// Paths that were filtered out
    pub paths_filtered: Vec<String>,

    /// Export timestamp
    pub exported_at: chrono::DateTime<Utc>,
}

/// Content to be exported
#[derive(Debug, Clone)]
pub struct ExportContent {
    /// Path within the package (e.g., "content/default/my-node/")
    pub package_path: String,

    /// Files in this content directory
    pub files: HashMap<String, Vec<u8>>,
}

/// Node type definition to export
#[derive(Debug, Clone)]
pub struct ExportNodeType {
    /// Node type name (e.g., "ai:Agent")
    pub name: String,

    /// YAML definition
    pub definition: String,
}

/// Mixin definition to export
#[derive(Debug, Clone)]
pub struct ExportMixin {
    /// Mixin name (e.g., "raisin:publishable")
    pub name: String,

    /// YAML definition
    pub definition: String,
}

/// Package exporter handles exporting installed content
pub struct PackageExporter {
    /// Base manifest (from original package or generated)
    manifest: Manifest,

    /// Export options
    options: ExportOptions,

    /// Mixins to include
    mixins: Vec<ExportMixin>,

    /// Node types to include
    node_types: Vec<ExportNodeType>,

    /// Content to include
    content: Vec<ExportContent>,

    /// Paths that were filtered out
    filtered_paths: Vec<String>,
}

impl PackageExporter {
    /// Create a new exporter with base manifest
    pub fn new(manifest: Manifest, options: ExportOptions) -> Self {
        Self {
            manifest,
            options,
            mixins: Vec::new(),
            node_types: Vec::new(),
            content: Vec::new(),
            filtered_paths: Vec::new(),
        }
    }

    /// Create exporter with default options
    pub fn with_manifest(manifest: Manifest) -> Self {
        Self::new(manifest, ExportOptions::default())
    }

    /// Set export options
    pub fn with_options(mut self, options: ExportOptions) -> Self {
        self.options = options;
        self
    }

    /// Add a mixin definition to export
    pub fn add_mixin(&mut self, name: String, definition: String) {
        self.mixins.push(ExportMixin { name, definition });
    }

    /// Add a node type definition to export
    pub fn add_node_type(&mut self, name: String, definition: String) {
        self.node_types.push(ExportNodeType { name, definition });
    }

    /// Add content to export
    pub fn add_content(&mut self, content: ExportContent) {
        self.content.push(content);
    }

    /// Add a single file to the content
    pub fn add_content_file(&mut self, package_path: &str, filename: &str, data: Vec<u8>) {
        // Find or create content entry
        if let Some(entry) = self
            .content
            .iter_mut()
            .find(|c| c.package_path == package_path)
        {
            entry.files.insert(filename.to_string(), data);
        } else {
            let mut files = HashMap::new();
            files.insert(filename.to_string(), data);
            self.content.push(ExportContent {
                package_path: package_path.to_string(),
                files,
            });
        }
    }

    /// Check if a path should be included based on filters
    pub fn should_include_path(&self, path: &str) -> bool {
        match self.options.export_mode {
            ExportMode::All => true,
            ExportMode::Filtered => {
                // Apply manifest sync filters if available
                if let Some(ref sync_config) = self.manifest.sync {
                    sync_config.should_sync_path(path)
                } else {
                    // No sync config, include everything
                    true
                }
            }
        }
    }

    /// Record a filtered path
    pub fn record_filtered(&mut self, path: String) {
        self.filtered_paths.push(path);
    }

    /// Get the manifest
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Get mutable manifest for updates
    pub fn manifest_mut(&mut self) -> &mut Manifest {
        &mut self.manifest
    }

    /// Build the .rap package
    pub fn build(self) -> PackageResult<ExportResult> {
        let mut buf = Vec::new();
        let mut files_included = 0;

        // Update version if specified
        let mut manifest = self.manifest;
        if let Some(ref new_version) = self.options.new_version {
            manifest.version = new_version.clone();
        }

        // Store for result
        let package_name = manifest.name.clone();
        let package_version = manifest.version.clone();

        {
            let cursor = Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(9));

            // Write manifest.yaml
            let manifest_yaml = serde_yaml::to_string(&manifest).map_err(|e| {
                PackageError::InvalidManifest(format!("Failed to serialize manifest: {}", e))
            })?;
            zip.start_file("manifest.yaml", options)
                .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;
            zip.write_all(manifest_yaml.as_bytes())
                .map_err(PackageError::IoError)?;
            files_included += 1;

            // Write mixins (before node types, since node types may reference them)
            if !self.mixins.is_empty() {
                zip.add_directory("mixins/", options)
                    .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;

                for mixin in &self.mixins {
                    let filename = format!(
                        "mixins/{}.yaml",
                        crate::namespace_encoding::encode_namespace(&mixin.name)
                    );
                    zip.start_file(&filename, options)
                        .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;
                    zip.write_all(mixin.definition.as_bytes())
                        .map_err(PackageError::IoError)?;
                    files_included += 1;
                }
            }

            // Write node types
            if !self.node_types.is_empty() {
                zip.add_directory("nodetypes/", options)
                    .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;

                for node_type in &self.node_types {
                    let filename = format!(
                        "nodetypes/{}.yaml",
                        crate::namespace_encoding::encode_namespace(&node_type.name)
                    );
                    zip.start_file(&filename, options)
                        .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;
                    zip.write_all(node_type.definition.as_bytes())
                        .map_err(PackageError::IoError)?;
                    files_included += 1;
                }
            }

            // Write content
            let mut written_dirs = std::collections::HashSet::new();
            for content in &self.content {
                // Ensure parent directories exist
                let parts: Vec<&str> = content
                    .package_path
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut current_path = String::new();
                for part in &parts {
                    current_path.push_str(part);
                    current_path.push('/');
                    if !written_dirs.contains(&current_path) {
                        zip.add_directory(&current_path, options)
                            .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;
                        written_dirs.insert(current_path.clone());
                    }
                }

                // Write files
                for (filename, data) in &content.files {
                    let file_path = format!("{}{}", content.package_path, filename);
                    zip.start_file(&file_path, options)
                        .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;
                    zip.write_all(data).map_err(PackageError::IoError)?;
                    files_included += 1;
                }
            }

            zip.finish()
                .map_err(|e| PackageError::IoError(std::io::Error::other(e)))?;
        }

        Ok(ExportResult {
            package_name,
            package_version,
            package_data: buf,
            files_included,
            paths_filtered: self.filtered_paths,
            exported_at: Utc::now(),
        })
    }
}

/// Builder for creating export content from node data
pub struct ContentBuilder {
    workspace: String,
    path: String,
    node_type: String,
    properties: serde_json::Value,
    files: HashMap<String, Vec<u8>>,
}

impl ContentBuilder {
    /// Create a new content builder
    pub fn new(workspace: &str, path: &str, node_type: &str) -> Self {
        Self {
            workspace: workspace.to_string(),
            path: path.to_string(),
            node_type: node_type.to_string(),
            properties: serde_json::json!({}),
            files: HashMap::new(),
        }
    }

    /// Set node properties
    pub fn with_properties(mut self, properties: serde_json::Value) -> Self {
        self.properties = properties;
        self
    }

    /// Add an associated file
    pub fn with_file(mut self, name: &str, content: Vec<u8>) -> Self {
        self.files.insert(name.to_string(), content);
        self
    }

    /// Build the export content
    pub fn build(self) -> ExportContent {
        let encoded_ws = crate::namespace_encoding::encode_namespace(&self.workspace);
        let package_path = format!("content/{}/{}/", encoded_ws, self.path);

        // Create node.yaml content
        let node_yaml = serde_json::json!({
            "node_type": self.node_type,
            "properties": self.properties,
        });
        let node_yaml_str = serde_yaml::to_string(&node_yaml).unwrap_or_default();

        let mut files = self.files;
        files.insert("node.yaml".to_string(), node_yaml_str.into_bytes());

        ExportContent {
            package_path,
            files,
        }
    }
}

/// Utility to compare package source with installed content
pub struct PackageComparator {
    /// Original package content hashes
    source_hashes: HashMap<String, String>,
}

impl PackageComparator {
    /// Create a new comparator from package browser
    pub fn from_package(browser: &crate::browser::PackageBrowser) -> PackageResult<Self> {
        let mut source_hashes = HashMap::new();

        for entry in browser.list_entries()? {
            if entry.entry_type == crate::browser::EntryType::File {
                let content = browser.read_file(&entry.path)?;
                let hash = crate::sync::compute_hash(&content);
                source_hashes.insert(entry.path, hash);
            }
        }

        Ok(Self { source_hashes })
    }

    /// Check if a path exists in the source package
    pub fn exists_in_source(&self, path: &str) -> bool {
        self.source_hashes.contains_key(path)
    }

    /// Get the source hash for a path
    pub fn get_source_hash(&self, path: &str) -> Option<&String> {
        self.source_hashes.get(path)
    }

    /// Check if content has been modified
    pub fn is_modified(&self, path: &str, current_content: &[u8]) -> bool {
        if let Some(source_hash) = self.source_hashes.get(path) {
            let current_hash = crate::sync::compute_hash(current_content);
            source_hash != &current_hash
        } else {
            // Not in source, so it's new (counts as modified for our purposes)
            true
        }
    }

    /// Get all source paths
    pub fn source_paths(&self) -> impl Iterator<Item = &String> {
        self.source_hashes.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_manifest() -> Manifest {
        Manifest {
            name: "test-export".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Test export package".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_exporter_basic() {
        let manifest = create_test_manifest();
        let mut exporter = PackageExporter::with_manifest(manifest);

        exporter.add_node_type(
            "test:Type".to_string(),
            "name: test:Type\ndescription: Test\n".to_string(),
        );

        let content = ContentBuilder::new("default", "test-node", "test:Type")
            .with_properties(serde_json::json!({"title": "Test"}))
            .with_file("index.js", b"export default function() {}".to_vec())
            .build();
        exporter.add_content(content);

        let result = exporter.build().unwrap();

        assert_eq!(result.package_name, "test-export");
        assert_eq!(result.package_version, "1.0.0");
        assert!(result.files_included >= 3); // manifest + nodetype + node.yaml + index.js
        assert!(!result.package_data.is_empty());
    }

    #[test]
    fn test_exporter_with_version_override() {
        let manifest = create_test_manifest();
        let options = ExportOptions {
            new_version: Some("2.0.0".to_string()),
            ..Default::default()
        };
        let exporter = PackageExporter::new(manifest, options);

        let result = exporter.build().unwrap();
        assert_eq!(result.package_version, "2.0.0");
    }

    #[test]
    fn test_content_builder() {
        let content = ContentBuilder::new("functions", "lib/my-func", "raisin:Function")
            .with_properties(serde_json::json!({
                "title": "My Function",
                "description": "Does things"
            }))
            .with_file("index.js", b"module.exports = () => 'hello';".to_vec())
            .build();

        assert_eq!(content.package_path, "content/functions/lib/my-func/");
        assert!(content.files.contains_key("node.yaml"));
        assert!(content.files.contains_key("index.js"));
    }

    #[test]
    fn test_exporter_filter_all() {
        let manifest = create_test_manifest();
        let options = ExportOptions {
            export_mode: ExportMode::All,
            ..Default::default()
        };
        let exporter = PackageExporter::new(manifest, options);

        assert!(exporter.should_include_path("/any/path"));
        assert!(exporter.should_include_path("/another/path"));
    }
}
