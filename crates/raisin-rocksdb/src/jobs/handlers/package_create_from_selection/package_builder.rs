//! Package building: ZIP creation, manifest, and node serialization

use super::manifest_types::PackageManifest;
use super::types::CollectedNode;
use super::PackageCreateFromSelectionHandler;
use raisin_error::{Error, Result};
use raisin_models::nodes::Node;

impl PackageCreateFromSelectionHandler {
    /// Build a .rap package from collected nodes
    ///
    /// The package format follows the install handler expectations:
    /// - Content files are under `content/{workspace}/{path}/...`
    /// - Folder nodes are written as `content/{workspace}/{path}/.node.yaml`
    /// - Asset nodes with embedded files are written as:
    ///   - `content/{workspace}/{parent}/.node.{filename}.yaml` (metadata)
    ///   - `content/{workspace}/{parent}/{filename}` (binary data)
    /// - Regular nodes are written as `content/{workspace}/{path}.yaml`
    ///
    /// Returns the path to the temp file containing the ZIP.
    /// This streams to disk to support large packages (40GB+) without loading into memory.
    pub(super) async fn build_package(
        &self,
        package_name: &str,
        package_version: &str,
        content_nodes: &[CollectedNode],
        node_type_nodes: &[CollectedNode],
    ) -> Result<std::path::PathBuf> {
        use std::io::Write;
        use tempfile::NamedTempFile;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        // Create temp file to stream ZIP to disk (supports large packages)
        let temp_file = NamedTempFile::new()
            .map_err(|e| Error::storage(format!("Failed to create temp file: {}", e)))?;

        // Use into_parts() to get both the File and TempPath separately
        // This prevents the TempPath from being dropped (which would delete the file)
        let (file, temp_path_handle) = temp_file.into_parts();

        {
            let mut zip = ZipWriter::new(file);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(9));

            // Build and write manifest
            let manifest = self.build_manifest(
                package_name,
                package_version,
                content_nodes,
                node_type_nodes,
            );
            let manifest_yaml = serde_yaml::to_string(&manifest)
                .map_err(|e| Error::storage(format!("Failed to serialize manifest: {}", e)))?;
            zip.start_file("manifest.yaml", options)
                .map_err(|e| Error::storage(format!("Failed to start manifest file: {}", e)))?;
            zip.write_all(manifest_yaml.as_bytes())
                .map_err(|e| Error::storage(format!("Failed to write manifest: {}", e)))?;

            // Write node type definitions (under nodetypes/ prefix)
            for collected in node_type_nodes {
                let file_path = format!("nodetypes{}.yaml", collected.node.path);
                let node_yaml = self.serialize_node(&collected.node)?;
                zip.start_file(&file_path, options).map_err(|e| {
                    Error::storage(format!("Failed to start file {}: {}", file_path, e))
                })?;
                zip.write_all(node_yaml.as_bytes()).map_err(|e| {
                    Error::storage(format!("Failed to write file {}: {}", file_path, e))
                })?;
            }

            // Write content nodes organized by workspace under content/ prefix
            for collected in content_nodes {
                self.write_content_node(&mut zip, options, collected)
                    .await?;
            }

            zip.finish()
                .map_err(|e| Error::storage(format!("Failed to finish ZIP: {}", e)))?;
        }

        // Persist the temp file by calling keep() - prevents deletion and returns PathBuf
        let temp_path = temp_path_handle
            .keep()
            .map_err(|e| Error::storage(format!("Failed to persist temp file: {}", e)))?;

        Ok(temp_path)
    }

    /// Write a single content node to the ZIP archive
    async fn write_content_node<W: std::io::Write + std::io::Seek>(
        &self,
        zip: &mut zip::ZipWriter<W>,
        options: zip::write::SimpleFileOptions,
        collected: &CollectedNode,
    ) -> Result<()> {
        use std::io::Write;

        let node = &collected.node;
        let workspace = &collected.workspace;
        let path = &node.path;

        // Check if this is an asset with embedded file
        if let Some(storage_key) = self.get_embedded_file_storage_key(node) {
            self.write_asset_node(zip, options, workspace, path, node, &storage_key)
                .await?;
        } else if is_folder_type(&node.node_type) || node.has_children.unwrap_or(false) {
            // Folder node - write as .node.yaml inside the folder
            let file_path = format!("content/{}{}/.node.yaml", workspace, path);
            let node_yaml = self.serialize_node(node)?;
            zip.start_file(&file_path, options).map_err(|e| {
                Error::storage(format!("Failed to start file {}: {}", file_path, e))
            })?;
            zip.write_all(node_yaml.as_bytes()).map_err(|e| {
                Error::storage(format!("Failed to write file {}: {}", file_path, e))
            })?;
        } else {
            // Regular node - write as {name}.yaml in parent directory
            let file_path = format!("content/{}{}.yaml", workspace, path);
            let node_yaml = self.serialize_node(node)?;
            zip.start_file(&file_path, options).map_err(|e| {
                Error::storage(format!("Failed to start file {}: {}", file_path, e))
            })?;
            zip.write_all(node_yaml.as_bytes()).map_err(|e| {
                Error::storage(format!("Failed to write file {}: {}", file_path, e))
            })?;
        }

        Ok(())
    }

    /// Write an asset node (metadata + binary file) to the ZIP archive
    async fn write_asset_node<W: std::io::Write + std::io::Seek>(
        &self,
        zip: &mut zip::ZipWriter<W>,
        options: zip::write::SimpleFileOptions,
        workspace: &str,
        path: &str,
        node: &Node,
        storage_key: &str,
    ) -> Result<()> {
        use std::io::Write;

        let filename = &node.name;
        let parent_path = get_parent_path(path);

        tracing::debug!(
            node_path = %path,
            node_name = %filename,
            storage_key = %storage_key,
            "Found asset with embedded binary file"
        );

        // 1. Write asset metadata as .node.{filename}.yaml
        let metadata_path = format!(
            "content/{}{}.node.{}.yaml",
            workspace, parent_path, filename
        );
        let node_yaml = self.serialize_node(node)?;
        zip.start_file(&metadata_path, options).map_err(|e| {
            Error::storage(format!("Failed to start file {}: {}", metadata_path, e))
        })?;
        zip.write_all(node_yaml.as_bytes()).map_err(|e| {
            Error::storage(format!("Failed to write file {}: {}", metadata_path, e))
        })?;

        // 2. Download and include actual binary if callback is available
        if let Some(ref binary_callback) = self.binary_retrieval_callback {
            match binary_callback(storage_key.to_string()).await {
                Ok(binary_data) => {
                    let binary_path = format!("content/{}{}{}", workspace, parent_path, filename);
                    zip.start_file(&binary_path, options).map_err(|e| {
                        Error::storage(format!(
                            "Failed to start binary file {}: {}",
                            binary_path, e
                        ))
                    })?;
                    zip.write_all(&binary_data).map_err(|e| {
                        Error::storage(format!(
                            "Failed to write binary file {}: {}",
                            binary_path, e
                        ))
                    })?;
                    tracing::info!(
                        file = %binary_path,
                        size = binary_data.len(),
                        "Included binary file in package"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        storage_key = %storage_key,
                        error = %e,
                        "Failed to retrieve binary file, skipping"
                    );
                }
            }
        } else {
            tracing::warn!(
                node_path = %path,
                storage_key = %storage_key,
                "Binary retrieval callback not configured, cannot include binary file in package"
            );
        }

        Ok(())
    }

    /// Build the package manifest
    pub(super) fn build_manifest(
        &self,
        package_name: &str,
        package_version: &str,
        content_nodes: &[CollectedNode],
        node_type_nodes: &[CollectedNode],
    ) -> PackageManifest {
        PackageManifest::build(
            package_name,
            package_version,
            content_nodes,
            node_type_nodes,
        )
    }

    /// Serialize a node to YAML for inclusion in package
    pub(super) fn serialize_node(&self, node: &Node) -> Result<String> {
        let json_value = serde_json::to_value(node)
            .map_err(|e| Error::storage(format!("Failed to serialize node to JSON: {}", e)))?;

        serde_yaml::to_string(&json_value)
            .map_err(|e| Error::storage(format!("Failed to convert node to YAML: {}", e)))
    }
}

/// Check if a node type represents a folder
fn is_folder_type(node_type: &str) -> bool {
    node_type.ends_with(":Folder") || node_type == "raisin:Folder"
}

/// Get the parent path from a node path
///
/// For example:
/// - "/lib/raisin/handler/index.js" -> "/lib/raisin/handler/"
/// - "/lib/raisin/handler" -> "/lib/raisin/"
/// - "/handler" -> "/"
fn get_parent_path(path: &str) -> &str {
    if let Some(last_slash_idx) = path.rfind('/') {
        if last_slash_idx == 0 {
            "/"
        } else {
            &path[..last_slash_idx + 1]
        }
    } else {
        "/"
    }
}
