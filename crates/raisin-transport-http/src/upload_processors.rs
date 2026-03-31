// SPDX-License-Identifier: BSL-1.1

//! Upload processors for NodeType-specific file upload handling
//!
//! This module provides a trait-based system for customizing upload behavior
//! based on the target node type. For example, `raisin:Package` uploads
//! extract manifest.yaml from the ZIP to get the package name and metadata.

use raisin_models::nodes::properties::value::PropertyValue;
use raisin_packages::Manifest;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use zip::ZipArchive;

use crate::error::ApiError;

/// Result of processing an uploaded file for a specific node type
#[derive(Debug, Clone)]
pub struct ProcessedUpload {
    /// Node ID to use (overrides default nanoid generation)
    pub node_id: Option<String>,
    /// Node name to use (overrides filename-derived name)
    pub node_name: Option<String>,
    /// Additional properties extracted from the upload
    pub properties: HashMap<String, PropertyValue>,
    /// Property name for storing the binary (default: "file")
    pub resource_property: String,
    /// How to store the resource
    pub storage_format: StorageFormat,
}

/// How to store the uploaded resource in node properties
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFormat {
    /// Store as PropertyValue::Resource (standard for assets)
    Resource,
    /// Store as PropertyValue::Object with key/url/mime_type/size fields
    Object,
}

impl Default for ProcessedUpload {
    fn default() -> Self {
        Self {
            node_id: None,
            node_name: None,
            properties: HashMap::new(),
            resource_property: "file".to_string(),
            storage_format: StorageFormat::Resource,
        }
    }
}

/// Trait for processors that handle uploads for specific node types
///
/// Note: This trait is synchronous because manifest extraction doesn't require
/// async operations. The file data is already buffered before processing.
pub trait UploadProcessor: Send + Sync {
    /// Check if this processor handles the given node type
    fn handles(&self, node_type: &str) -> bool;

    /// Process the uploaded file data and extract node metadata
    ///
    /// # Arguments
    /// * `file_data` - The raw bytes of the uploaded file
    /// * `file_name` - Original filename if provided
    /// * `path` - The target node path from the URL
    ///
    /// # Returns
    /// ProcessedUpload containing node ID, name, and properties to set
    fn process(
        &self,
        file_data: &[u8],
        file_name: Option<&str>,
        path: &str,
    ) -> Result<ProcessedUpload, ApiError>;
}

/// Registry of upload processors
pub struct UploadProcessorRegistry {
    processors: Vec<Arc<dyn UploadProcessor>>,
}

impl Default for UploadProcessorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl UploadProcessorRegistry {
    /// Create a new registry with built-in processors
    pub fn new() -> Self {
        let mut registry = Self {
            processors: Vec::new(),
        };
        // Register built-in processors
        registry.register(Arc::new(PackageUploadProcessor));
        registry
    }

    /// Register a custom upload processor
    pub fn register(&mut self, processor: Arc<dyn UploadProcessor>) {
        self.processors.push(processor);
    }

    /// Get the processor for a given node type, if any
    pub fn get_processor(&self, node_type: &str) -> Option<&dyn UploadProcessor> {
        self.processors
            .iter()
            .find(|p| p.handles(node_type))
            .map(|p| p.as_ref())
    }

    /// Check if any processor handles the given node type
    pub fn has_processor(&self, node_type: &str) -> bool {
        self.processors.iter().any(|p| p.handles(node_type))
    }
}

// ============================================================================
// Built-in Processors
// ============================================================================

/// Upload processor for raisin:Package nodes
///
/// Extracts manifest.yaml from the ZIP to get the package name and metadata.
/// The package name from manifest is used as node name for upsert handling.
pub struct PackageUploadProcessor;

impl UploadProcessor for PackageUploadProcessor {
    fn handles(&self, node_type: &str) -> bool {
        node_type == "raisin:Package"
    }

    fn process(
        &self,
        file_data: &[u8],
        _file_name: Option<&str>,
        _path: &str,
    ) -> Result<ProcessedUpload, ApiError> {
        // Extract manifest from ZIP to get package name and metadata
        let cursor = Cursor::new(file_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP archive: {}", e)))?;

        let manifest = Manifest::from_zip(&mut archive).map_err(|e| {
            ApiError::validation_failed(format!("Failed to read manifest.yaml: {}", e))
        })?;

        // Validate manifest
        manifest
            .validate()
            .map_err(|e| ApiError::validation_failed(format!("Invalid manifest: {}", e)))?;

        // Use manifest name as the package name (not filename)
        let package_name = manifest.name.clone();

        // Check for teaser background in static/
        let teaser_background_url = find_teaser_background(&mut archive);

        // Build properties from manifest
        let mut properties = HashMap::new();

        // Required properties
        properties.insert("name".to_string(), PropertyValue::String(manifest.name));
        properties.insert(
            "version".to_string(),
            PropertyValue::String(manifest.version),
        );

        // Optional properties from manifest
        if let Some(title) = manifest.title {
            properties.insert("title".to_string(), PropertyValue::String(title));
        }
        if let Some(description) = manifest.description {
            properties.insert(
                "description".to_string(),
                PropertyValue::String(description),
            );
        }
        if let Some(author) = manifest.author {
            properties.insert("author".to_string(), PropertyValue::String(author));
        }
        if let Some(license) = manifest.license {
            properties.insert("license".to_string(), PropertyValue::String(license));
        }
        if let Some(category) = manifest.category {
            properties.insert("category".to_string(), PropertyValue::String(category));
        }

        // Icon and color (have defaults)
        properties.insert("icon".to_string(), PropertyValue::String(manifest.icon));
        properties.insert("color".to_string(), PropertyValue::String(manifest.color));

        // Keywords as array
        if !manifest.keywords.is_empty() {
            properties.insert(
                "keywords".to_string(),
                PropertyValue::Array(
                    manifest
                        .keywords
                        .into_iter()
                        .map(PropertyValue::String)
                        .collect(),
                ),
            );
        }

        // Status tracking
        properties.insert(
            "status".to_string(),
            PropertyValue::String("processing".to_string()),
        );
        properties.insert("installed".to_string(), PropertyValue::Boolean(false));

        // Upload state will be set by repo handler based on whether package exists
        // Default to "new" - repo handler will change to "updated" if package exists
        properties.insert(
            "upload_state".to_string(),
            PropertyValue::String("new".to_string()),
        );

        // Teaser background URL (path within package, to be resolved after upload)
        if let Some(teaser_path) = teaser_background_url {
            properties.insert(
                "teaser_background_url".to_string(),
                PropertyValue::String(teaser_path),
            );
        }

        Ok(ProcessedUpload {
            node_id: None, // Will be resolved in repo handler for upsert
            node_name: Some(package_name),
            properties,
            resource_property: "resource".to_string(),
            storage_format: StorageFormat::Object,
        })
    }
}

/// Check if the ZIP contains a teaser background image in static/
fn find_teaser_background<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Option<String> {
    // Check for common teaser background patterns
    let patterns = [
        "static/teaser_background.png",
        "static/teaser_background.jpg",
        "static/teaser_background.jpeg",
        "static/teaser-background.png",
        "static/teaser-background.jpg",
        "static/teaser-background.jpeg",
    ];

    for pattern in patterns {
        if archive.by_name(pattern).is_ok() {
            return Some(pattern.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_package_processor() {
        let registry = UploadProcessorRegistry::new();
        assert!(registry.has_processor("raisin:Package"));
        assert!(!registry.has_processor("raisin:Asset"));
        assert!(!registry.has_processor("unknown:Type"));
    }

    #[test]
    fn test_package_processor_handles() {
        let processor = PackageUploadProcessor;
        assert!(processor.handles("raisin:Package"));
        assert!(!processor.handles("raisin:Asset"));
    }
}
