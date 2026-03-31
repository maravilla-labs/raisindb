//! Package export job handler
//!
//! This module handles background export of installed packages as .rap files.
//! For packages that have an existing resource (original .rap), it returns that.
//! For packages without a resource, it can rebuild from installed content.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobRegistry, JobType};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::package_install::translation::overlay_to_yaml;

/// Callback type for retrieving binary data from blob storage
pub type BinaryRetrievalCallback = Arc<
    dyn Fn(
            String, // key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for storing exported package binary
pub type BinaryStorageCallback = Arc<
    dyn Fn(
            Vec<u8>,        // data
            Option<String>, // content_type
            Option<String>, // extension
            Option<String>, // original_name
            Option<String>, // tenant_context
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<raisin_binary::StoredObject>> + Send>,
        > + Send
        + Sync,
>;

/// Result of package export operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageExportResult {
    /// Package name
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// Number of files exported
    pub files_exported: usize,
    /// Blob key for downloading the package (used by download handler)
    pub blob_key: String,
    /// URL to download the package
    pub download_url: String,
    /// Export timestamp
    pub exported_at: String,
}

/// Handler for package export jobs
pub struct PackageExportHandler<S: Storage> {
    storage: Arc<S>,
    job_registry: Arc<JobRegistry>,
    binary_retrieval_callback: Option<BinaryRetrievalCallback>,
    binary_store_callback: Option<BinaryStorageCallback>,
}

impl<S: Storage + TransactionalStorage> PackageExportHandler<S> {
    /// Create a new package export handler
    pub fn new(storage: Arc<S>, job_registry: Arc<JobRegistry>) -> Self {
        Self {
            storage,
            job_registry,
            binary_retrieval_callback: None,
            binary_store_callback: None,
        }
    }

    /// Set the binary retrieval callback
    pub fn with_binary_callback(mut self, callback: BinaryRetrievalCallback) -> Self {
        self.binary_retrieval_callback = Some(callback);
        self
    }

    /// Set the binary storage callback
    pub fn with_binary_store_callback(mut self, callback: BinaryStorageCallback) -> Self {
        self.binary_store_callback = Some(callback);
        self
    }

    /// Handle package export job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract parameters from JobType
        let (package_name, package_node_id, _export_mode, _include_modifications) =
            match &job.job_type {
                JobType::PackageExport {
                    package_name,
                    package_node_id,
                    export_mode,
                    include_modifications,
                } => (
                    package_name.as_str(),
                    package_node_id.as_str(),
                    export_mode.as_str(),
                    *include_modifications,
                ),
                _ => {
                    return Err(Error::Validation(
                        "Expected PackageExport job type".to_string(),
                    ))
                }
            };

        tracing::info!(
            job_id = %job.id,
            package_name = %package_name,
            package_node_id = %package_node_id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Starting package export"
        );

        // Report progress: starting
        self.report_progress(&job.id, 0.1, "Loading package information")
            .await;

        // Get the package node
        let node_repo = self.storage.nodes();
        let workspace = "packages";
        let branch = &context.branch;

        let package_node = node_repo
            .get(
                StorageScope::new(&context.tenant_id, &context.repo_id, branch, workspace),
                package_node_id,
                None,
            )
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!("Package node not found: {}", package_node_id))
            })?;

        // Get version from package node
        let version = package_node
            .properties
            .get("version")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "1.0.0".to_string());

        // Report progress
        self.report_progress(&job.id, 0.3, "Checking package resource")
            .await;

        // Check if package has an existing resource (original .rap file)
        let existing_blob_key = self.get_resource_key(&package_node);

        if let Some(blob_key) = existing_blob_key {
            // Package has existing resource - just return that key
            tracing::info!(
                job_id = %job.id,
                package_name = %package_name,
                blob_key = %blob_key,
                "Package has existing resource, returning blob key"
            );

            self.report_progress(&job.id, 1.0, "Export complete").await;

            let result = PackageExportResult {
                package_name: package_name.to_string(),
                package_version: version,
                files_exported: 0, // Not counting for existing resource
                blob_key,
                download_url: String::new(), // Will be constructed by download handler
                exported_at: chrono::Utc::now().to_rfc3339(),
            };

            return Ok(Some(serde_json::to_value(result).unwrap_or_default()));
        }

        // No existing resource - need to rebuild from installed content
        // This requires binary storage callback
        let binary_store = self.binary_store_callback.as_ref().ok_or_else(|| {
            Error::Validation(
                "Binary storage callback not configured for package rebuild".to_string(),
            )
        })?;

        self.report_progress(&job.id, 0.5, "Rebuilding package from installed content")
            .await;

        // Build a minimal package with manifest
        let package_data = self.build_minimal_package(&package_node)?;

        self.report_progress(&job.id, 0.9, "Storing package file")
            .await;

        // Store the rebuilt package
        let filename = format!("{}-{}.rap", package_name, version);
        let stored = binary_store(
            package_data,
            Some("application/zip".to_string()),
            Some("rap".to_string()),
            Some(filename),
            None,
        )
        .await?;

        self.report_progress(&job.id, 1.0, "Export complete").await;

        let result = PackageExportResult {
            package_name: package_name.to_string(),
            package_version: version,
            files_exported: 1,
            blob_key: stored.key,
            download_url: stored.url,
            exported_at: chrono::Utc::now().to_rfc3339(),
        };

        tracing::info!(
            job_id = %job.id,
            package_name = %result.package_name,
            blob_key = %result.blob_key,
            "Package export completed"
        );

        Ok(Some(serde_json::to_value(result).unwrap_or_default()))
    }

    /// Get the resource key from a package node if it exists
    fn get_resource_key(&self, node: &Node) -> Option<String> {
        let resource = node.properties.get("resource")?;

        let resource_obj = match resource {
            PropertyValue::Object(obj) => obj,
            _ => return None,
        };

        let key = resource_obj.get("key")?;

        match key {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Build a minimal package with just the manifest
    fn build_minimal_package(&self, node: &Node) -> Result<Vec<u8>> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let manifest = self.build_manifest_from_node(node)?;

        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(9));

            // Write manifest.yaml
            let manifest_yaml = serde_yaml::to_string(&manifest)
                .map_err(|e| Error::storage(format!("Failed to serialize manifest: {}", e)))?;
            zip.start_file("manifest.yaml", options)
                .map_err(|e| Error::storage(format!("Failed to start manifest file: {}", e)))?;
            zip.write_all(manifest_yaml.as_bytes())
                .map_err(|e| Error::storage(format!("Failed to write manifest: {}", e)))?;

            zip.finish()
                .map_err(|e| Error::storage(format!("Failed to finish ZIP: {}", e)))?;
        }

        Ok(buf)
    }

    /// Build a manifest from package node properties
    fn build_manifest_from_node(&self, node: &Node) -> Result<PackageManifest> {
        let get_string = |key: &str| -> Option<String> {
            node.properties.get(key).and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
        };

        let get_string_array = |key: &str| -> Option<Vec<String>> {
            node.properties.get(key).and_then(|v| match v {
                PropertyValue::Array(arr) => {
                    let strings: Vec<String> = arr
                        .iter()
                        .filter_map(|item| match item {
                            PropertyValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    if strings.is_empty() {
                        None
                    } else {
                        Some(strings)
                    }
                }
                _ => None,
            })
        };

        let name = get_string("name").unwrap_or_else(|| node.name.clone());
        let version = get_string("version").unwrap_or_else(|| "1.0.0".to_string());

        let mut manifest = PackageManifest {
            name,
            version,
            title: get_string("title"),
            description: get_string("description"),
            author: get_string("author"),
            license: get_string("license"),
            icon: get_string("icon"),
            color: get_string("color"),
            keywords: get_string_array("keywords"),
            category: get_string("category"),
            provides: None,
            dependencies: None,
            locales: get_string_array("locales"),
        };

        // Extract provides from node properties
        if let Some(PropertyValue::Object(provides_obj)) = node.properties.get("provides") {
            let mut provides = PackageProvides {
                nodetypes: None,
                workspaces: None,
                content: None,
            };

            if let Some(PropertyValue::Array(arr)) = provides_obj.get("nodetypes") {
                provides.nodetypes = Some(
                    arr.iter()
                        .filter_map(|v| match v {
                            PropertyValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                );
            }

            if let Some(PropertyValue::Array(arr)) = provides_obj.get("workspaces") {
                provides.workspaces = Some(
                    arr.iter()
                        .filter_map(|v| match v {
                            PropertyValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                );
            }

            if let Some(PropertyValue::Array(arr)) = provides_obj.get("content") {
                provides.content = Some(
                    arr.iter()
                        .filter_map(|v| match v {
                            PropertyValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                );
            }

            manifest.provides = Some(provides);
        }

        Ok(manifest)
    }

    /// Write translation files for a content node into a ZIP archive.
    ///
    /// Queries all locales for the given node and writes each as a
    /// `.node.{locale}.yaml` file alongside the base node's YAML path.
    pub(crate) async fn write_translations_to_zip<W: std::io::Write + std::io::Seek>(
        tx: &dyn TransactionalContext,
        workspace: &str,
        node_id: &str,
        base_yaml_dir: &str,
        zip: &mut zip::ZipWriter<W>,
        options: zip::write::SimpleFileOptions,
    ) -> Result<usize> {
        let locales = tx.list_translations_for_node(workspace, node_id).await?;

        let mut count = 0;
        for locale in &locales {
            if let Some(overlay) = tx.get_translation(workspace, node_id, locale).await? {
                let yaml_value = overlay_to_yaml(&overlay);
                let yaml_str = serde_yaml::to_string(&yaml_value).map_err(|e| {
                    Error::storage(format!("Failed to serialize translation YAML: {}", e))
                })?;

                let file_path = format!("{}/.node.{}.yaml", base_yaml_dir, locale);
                zip.start_file(&file_path, options).map_err(|e| {
                    Error::storage(format!(
                        "Failed to start translation file {}: {}",
                        file_path, e
                    ))
                })?;
                std::io::Write::write_all(zip, yaml_str.as_bytes()).map_err(|e| {
                    Error::storage(format!(
                        "Failed to write translation file {}: {}",
                        file_path, e
                    ))
                })?;
                count += 1;
            }
        }

        Ok(count)
    }

    /// Report progress to job registry
    async fn report_progress(&self, job_id: &JobId, progress: f32, message: &str) {
        tracing::debug!(job_id = %job_id, progress = %progress, message = %message, "Package export progress");
        if let Err(e) = self.job_registry.update_progress(job_id, progress).await {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to update job progress");
        }
    }
}

/// Simplified package manifest for export
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PackageManifest {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provides: Option<PackageProvides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Vec<PackageDependency>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    locales: Option<Vec<String>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PackageProvides {
    #[serde(skip_serializing_if = "Option::is_none")]
    nodetypes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspaces: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<Vec<String>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PackageDependency {
    name: String,
    version: String,
}
