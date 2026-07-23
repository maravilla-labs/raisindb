// SPDX-License-Identifier: BSL-1.1

//! Upload processors for NodeType-specific file upload handling
//!
//! This module provides a trait-based system for customizing upload behavior
//! based on the target node type. For example, `raisin:Package` uploads
//! extract manifest.yaml from the ZIP to get the package name and metadata.

use raisin_models::nodes::properties::value::PropertyValue;
use raisin_packages::{Manifest, SyncConfig};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use zip::ZipArchive;

use crate::error::ApiError;

/// Serialize a package's `sync:` policy into a compact property value for
/// display in the package list/details UI: the top-level default mode plus
/// each root filter's mode, so a "custom sync policy" badge can be shown
/// without running a dry run.
fn sync_policy_summary(sync: &SyncConfig) -> PropertyValue {
    let mode_str = |mode: raisin_packages::SyncMode| {
        serde_json::to_value(mode)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };

    let mut summary = HashMap::new();
    summary.insert(
        "default_mode".to_string(),
        PropertyValue::String(mode_str(sync.defaults.mode)),
    );
    summary.insert(
        "filters".to_string(),
        PropertyValue::Array(
            sync.filters
                .iter()
                .map(|filter| {
                    let mut entry = HashMap::new();
                    entry.insert(
                        "root".to_string(),
                        PropertyValue::String(filter.root.clone()),
                    );
                    if let Some(mode) = filter.mode {
                        entry.insert("mode".to_string(), PropertyValue::String(mode_str(mode)));
                    }
                    PropertyValue::Object(entry)
                })
                .collect(),
        ),
    );

    PropertyValue::Object(summary)
}

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

        // Read the package's optional `.raisin-sync.yaml` per-path sync/install
        // policy, if it ships one at its root (beside `manifest.yaml`).
        let sync_config = read_sync_config(&mut archive)?;

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

        // Sync policy summary, for surfacing "this package has a custom
        // install/sync policy" in the package list/details UI without
        // needing a dry run.
        if let Some(sync) = &sync_config {
            properties.insert("sync_policy".to_string(), sync_policy_summary(sync));
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

/// Read and parse the package's optional `.raisin-sync.yaml` per-path
/// sync/install policy. Returns `Ok(None)` when the file is absent — most
/// packages don't ship one.
fn read_sync_config<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<SyncConfig>, ApiError> {
    let mut file = match archive.by_name(raisin_packages::SYNC_CONFIG_FILENAME) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };

    let mut content = String::new();
    std::io::Read::read_to_string(&mut file, &mut content).map_err(|e| {
        ApiError::validation_failed(format!(
            "Failed to read {}: {}",
            raisin_packages::SYNC_CONFIG_FILENAME,
            e
        ))
    })?;

    let config = serde_yaml::from_str(&content).map_err(|e| {
        ApiError::validation_failed(format!(
            "Invalid {} format: {}",
            raisin_packages::SYNC_CONFIG_FILENAME,
            e
        ))
    })?;

    Ok(Some(config))
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

    #[test]
    fn sync_policy_summary_reports_default_and_filter_modes() {
        use raisin_packages::{SyncDefaults, SyncFilter, SyncMode};

        let sync = SyncConfig {
            defaults: SyncDefaults {
                mode: SyncMode::Skip,
                ..SyncDefaults::default()
            },
            filters: vec![SyncFilter {
                root: "/functions".to_string(),
                mode: Some(SyncMode::Replace),
                direction: None,
                filter_type: Default::default(),
                include: Vec::new(),
                exclude: Vec::new(),
                on_conflict: None,
                properties: None,
            }],
            ..SyncConfig::default()
        };

        let PropertyValue::Object(summary) = sync_policy_summary(&sync) else {
            panic!("expected an Object PropertyValue");
        };
        assert_eq!(
            summary.get("default_mode"),
            Some(&PropertyValue::String("skip".to_string()))
        );
        let PropertyValue::Array(filters) = summary.get("filters").unwrap() else {
            panic!("expected filters to be an Array");
        };
        assert_eq!(filters.len(), 1);
        let PropertyValue::Object(filter) = &filters[0] else {
            panic!("expected filter entry to be an Object");
        };
        assert_eq!(
            filter.get("root"),
            Some(&PropertyValue::String("/functions".to_string()))
        );
        assert_eq!(
            filter.get("mode"),
            Some(&PropertyValue::String("replace".to_string()))
        );
    }

    /// End-to-end: a `.rap` shipping `.raisin-sync.yaml` at its root gets a
    /// `sync_policy` property on upload; one without the file gets none.
    #[test]
    fn process_reads_raisin_sync_yaml_from_package_root() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        fn build_package(sync_yaml: Option<&str>) -> Vec<u8> {
            let mut buf = Vec::new();
            {
                let cursor = Cursor::new(&mut buf);
                let mut zip = ZipWriter::new(cursor);
                let options = SimpleFileOptions::default();

                zip.start_file("manifest.yaml", options).unwrap();
                zip.write_all(b"name: sync-test\nversion: 1.0.0\n").unwrap();

                if let Some(yaml) = sync_yaml {
                    zip.start_file(".raisin-sync.yaml", options).unwrap();
                    zip.write_all(yaml.as_bytes()).unwrap();
                }

                zip.finish().unwrap();
            }
            buf
        }

        let processor = PackageUploadProcessor;

        // With .raisin-sync.yaml
        let with_sync = build_package(Some(
            "defaults:\n  mode: skip\nfilters:\n  - root: /functions\n    mode: replace\n",
        ));
        let processed = processor.process(&with_sync, None, "/").unwrap();
        let PropertyValue::Object(summary) = processed
            .properties
            .get("sync_policy")
            .expect("sync_policy property should be set")
        else {
            panic!("expected an Object PropertyValue");
        };
        assert_eq!(
            summary.get("default_mode"),
            Some(&PropertyValue::String("skip".to_string()))
        );

        // Without .raisin-sync.yaml — no sync_policy property at all
        let without_sync = build_package(None);
        let processed = processor.process(&without_sync, None, "/").unwrap();
        assert!(!processed.properties.contains_key("sync_policy"));
    }

    /// `raisin:Package` is a `strict: true` NodeType — a real upload 400s if
    /// `PackageUploadProcessor` writes any property the schema doesn't declare
    /// (this is exactly how `sync_policy` slipped past unit tests once before:
    /// they exercised `process()` in isolation without checking the property
    /// actually round-trips through the strict schema). Guard against it
    /// recurring by asserting every key `process()` can produce is declared
    /// on the real, embedded `raisin:Package` NodeType.
    #[test]
    fn processed_properties_are_all_declared_on_strict_raisin_package_schema() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let package_node_type = raisin_core::nodetype_init::load_global_nodetypes()
            .into_iter()
            .find(|nt| nt.name == "raisin:Package")
            .expect("raisin:Package global NodeType should be embedded");
        assert_eq!(
            package_node_type.strict,
            Some(true),
            "test assumes raisin:Package is strict; update this test if that changes"
        );
        let declared: std::collections::HashSet<&str> = package_node_type
            .properties
            .as_ref()
            .expect("raisin:Package should declare properties")
            .iter()
            .filter_map(|p| p.name.as_deref())
            .collect();

        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let options = SimpleFileOptions::default();

            zip.start_file("manifest.yaml", options).unwrap();
            zip.write_all(
                b"name: schema-check\nversion: 1.0.0\ntitle: t\ndescription: d\nauthor: a\nlicense: MIT\nkeywords: [x]\ncategory: c\n",
            )
            .unwrap();

            zip.start_file(".raisin-sync.yaml", options).unwrap();
            zip.write_all(b"defaults:\n  mode: skip\nfilters:\n  - root: /functions\n    mode: replace\n")
                .unwrap();

            zip.start_file("static/teaser_background.png", options)
                .unwrap();
            zip.write_all(b"not-a-real-png").unwrap();

            zip.finish().unwrap();
        }

        let processed = PackageUploadProcessor.process(&buf, None, "/").unwrap();

        for key in processed.properties.keys() {
            assert!(
                declared.contains(key.as_str()),
                "PackageUploadProcessor writes property '{}' but raisin:Package (strict) doesn't declare it — \
                 a real upload will 400 with VALIDATION_FAILED. Add it to crates/raisin-core/global_nodetypes/raisin_package.yaml.",
                key
            );
        }
    }
}
