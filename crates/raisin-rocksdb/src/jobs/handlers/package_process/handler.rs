//! Package process handler
//!
//! Contains the `PackageProcessHandler` struct that orchestrates package processing:
//! retrieving the ZIP, extracting the manifest, updating node properties, and
//! extracting package assets.

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobRegistry, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

use super::manifest::{build_package_properties, PackageManifest};

/// Callback type for binary retrieval
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (resource_key)
/// Returns: Result<Vec<u8>> - the binary data
pub type BinaryRetrievalCallback = Arc<
    dyn Fn(
            String, // resource_key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for binary storage (writing)
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (data, content_type, extension, filename, tenant_context)
/// Returns: Result<StoredObject> - metadata about stored binary
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

/// Result of package processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageProcessResult {
    /// Package node ID
    pub package_node_id: String,
    /// Package name (from manifest)
    pub package_name: String,
    /// Package version (from manifest)
    pub package_version: String,
    /// Whether processing was successful
    pub success: bool,
}

/// Handler for package processing jobs
pub struct PackageProcessHandler<S: Storage> {
    pub(super) storage: Arc<S>,
    job_registry: Arc<JobRegistry>,
    binary_callback: Option<BinaryRetrievalCallback>,
    pub(super) binary_store_callback: Option<BinaryStorageCallback>,
}
impl<S: Storage + TransactionalStorage> PackageProcessHandler<S> {
    /// Create a new package process handler
    pub fn new(storage: Arc<S>, job_registry: Arc<JobRegistry>) -> Self {
        Self {
            storage,
            job_registry,
            binary_callback: None,
            binary_store_callback: None,
        }
    }

    /// Set the binary retrieval callback
    pub fn with_binary_callback(mut self, callback: BinaryRetrievalCallback) -> Self {
        self.binary_callback = Some(callback);
        self
    }

    /// Set the binary retrieval callback (mutable reference)
    pub fn set_binary_callback(&mut self, callback: BinaryRetrievalCallback) {
        self.binary_callback = Some(callback);
    }

    /// Set the binary storage callback (for storing extracted files)
    pub fn with_binary_store_callback(mut self, callback: BinaryStorageCallback) -> Self {
        self.binary_store_callback = Some(callback);
        self
    }

    /// Set the binary storage callback (mutable reference)
    pub fn set_binary_store_callback(&mut self, callback: BinaryStorageCallback) {
        self.binary_store_callback = Some(callback);
    }

    /// Handle package processing job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let package_node_id = match &job.job_type {
            JobType::PackageProcess { package_node_id } => package_node_id.as_str(),
            _ => {
                return Err(Error::Validation(
                    "Expected PackageProcess job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            package_node_id = %package_node_id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Starting package processing"
        );

        self.report_progress(&job.id, 0.1, "Retrieving package")
            .await;

        let resource_key = context
            .metadata
            .get("resource_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Validation("Missing resource_key in job context".to_string()))?
            .to_string();

        let binary_callback = self.binary_callback.as_ref().ok_or_else(|| {
            Error::Validation("Binary retrieval callback not configured".to_string())
        })?;

        let zip_data = binary_callback(resource_key).await?;

        self.report_progress(&job.id, 0.3, "Extracting manifest")
            .await;

        let manifest = Self::extract_manifest(&zip_data)?;

        tracing::debug!(
            job_id = %job.id,
            package_name = %manifest.name,
            package_version = %manifest.version,
            "Manifest extracted"
        );

        self.report_progress(&job.id, 0.6, "Updating node properties")
            .await;

        let properties = build_package_properties(&manifest);

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx.set_branch(&context.branch)?;
        tx.set_actor("package-processor")?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message("Process package manifest")?;

        let workspace = "packages";
        let node = tx.get_node(workspace, package_node_id).await?;

        if let Some(mut node) = node {
            let final_node_id = self
                .update_package_node(
                    &mut node,
                    context,
                    &manifest,
                    properties,
                    package_node_id,
                    workspace,
                    &job.id,
                )
                .await?;

            if self.binary_store_callback.is_some() {
                self.report_progress(&job.id, 0.9, "Extracting package assets")
                    .await;

                if let Err(e) = self
                    .extract_package_assets(
                        &zip_data,
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        &final_node_id,
                        &manifest.name,
                        &job.id,
                    )
                    .await
                {
                    tracing::warn!(
                        job_id = %job.id, error = %e,
                        "Failed to extract package assets (non-fatal)"
                    );
                }
            }

            tracing::info!(
                job_id = %job.id, package_node_id = %final_node_id,
                package_name = %manifest.name, package_version = %manifest.version,
                "Package processing completed"
            );

            self.report_progress(&job.id, 1.0, "Processing complete")
                .await;

            let result = PackageProcessResult {
                package_node_id: final_node_id,
                package_name: manifest.name,
                package_version: manifest.version,
                success: true,
            };

            Ok(Some(serde_json::to_value(result).unwrap_or_default()))
        } else {
            Err(Error::NotFound(format!(
                "Package node not found: {}",
                package_node_id
            )))
        }
    }

    /// Update package node based on upload type (large vs small) and existing packages
    async fn update_package_node(
        &self,
        node: &mut raisin_models::nodes::Node,
        context: &JobContext,
        manifest: &PackageManifest,
        properties: std::collections::HashMap<String, PropertyValue>,
        package_node_id: &str,
        workspace: &str,
        job_id: &JobId,
    ) -> Result<String> {
        let is_large_upload = context
            .metadata
            .get("large_upload")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || node
                .properties
                .get("large_upload")
                .map(|v| matches!(v, PropertyValue::Boolean(true)))
                .unwrap_or(false);

        let expected_path = format!("/{}", manifest.name);
        let current_path = node.path.clone();

        if is_large_upload && current_path != expected_path {
            tracing::info!(
                job_id = %job_id,
                current_path = %current_path,
                expected_path = %expected_path,
                "Large upload detected, checking for existing package"
            );

            self.report_progress(job_id, 0.7, "Checking for existing package")
                .await;

            let existing_package = {
                let check_tx = self.storage.begin_context().await?;
                check_tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
                check_tx.set_branch(&context.branch)?;
                check_tx.set_actor("package-processor")?;
                check_tx.set_auth_context(AuthContext::system())?;
                check_tx.get_node_by_path(workspace, &expected_path).await?
            };

            if let Some(mut existing) = existing_package {
                self.upsert_existing_package(
                    &mut existing,
                    node,
                    properties,
                    package_node_id,
                    workspace,
                    context,
                    job_id,
                )
                .await
            } else {
                self.rename_temp_node(
                    node,
                    &manifest.name,
                    &expected_path,
                    properties,
                    context,
                    workspace,
                    job_id,
                )
                .await
            }
        } else {
            self.update_small_upload(node, properties, context, workspace)
                .await
        }
    }

    /// Upsert into an existing package node and delete the temp upload node
    async fn upsert_existing_package(
        &self,
        existing: &mut raisin_models::nodes::Node,
        temp_node: &raisin_models::nodes::Node,
        properties: std::collections::HashMap<String, PropertyValue>,
        package_node_id: &str,
        workspace: &str,
        context: &JobContext,
        job_id: &JobId,
    ) -> Result<String> {
        tracing::info!(
            job_id = %job_id,
            existing_id = %existing.id,
            temp_id = %package_node_id,
            "Existing package found, performing upsert"
        );

        self.report_progress(job_id, 0.8, "Updating existing package")
            .await;

        if let Some(resource) = temp_node.properties.get("resource") {
            existing
                .properties
                .insert("resource".to_string(), resource.clone());
        }

        for (key, value) in properties {
            existing.properties.insert(key, value);
        }

        existing.properties.insert(
            "status".to_string(),
            PropertyValue::String("ready".to_string()),
        );
        existing.properties.insert(
            "upload_state".to_string(),
            PropertyValue::String("updated".to_string()),
        );
        existing
            .properties
            .insert("installed".to_string(), PropertyValue::Boolean(false));
        existing
            .properties
            .insert("progress".to_string(), PropertyValue::Float(1.0));
        existing.properties.remove("large_upload");

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx.set_branch(&context.branch)?;
        tx.set_actor("package-processor")?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message("Upsert existing package and delete temp node")?;
        tx.upsert_node(workspace, existing).await?;
        tx.delete_node(workspace, package_node_id).await?;
        tx.commit().await?;

        tracing::info!(
            job_id = %job_id,
            "Temp node deleted, existing package updated"
        );

        Ok(existing.id.clone())
    }

    /// Rename a temporary upload node to the correct package name/path
    async fn rename_temp_node(
        &self,
        node: &mut raisin_models::nodes::Node,
        package_name: &str,
        expected_path: &str,
        properties: std::collections::HashMap<String, PropertyValue>,
        context: &JobContext,
        workspace: &str,
        job_id: &JobId,
    ) -> Result<String> {
        tracing::info!(
            job_id = %job_id,
            package_name = %package_name,
            "No existing package, renaming temp node"
        );

        self.report_progress(job_id, 0.8, "Renaming package").await;

        node.name = package_name.to_string();
        node.path = expected_path.to_string();

        for (key, value) in properties {
            node.properties.insert(key, value);
        }

        node.properties.insert(
            "status".to_string(),
            PropertyValue::String("ready".to_string()),
        );
        node.properties
            .insert("progress".to_string(), PropertyValue::Float(1.0));
        node.properties.remove("large_upload");

        let tx2 = self.storage.begin_context().await?;
        tx2.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx2.set_branch(&context.branch)?;
        tx2.set_actor("package-processor")?;
        tx2.set_auth_context(AuthContext::system())?;
        tx2.set_message("Rename temp package node")?;
        tx2.upsert_node(workspace, node).await?;
        tx2.commit().await?;

        Ok(node.id.clone())
    }

    /// Update a small upload node (already has correct path) with manifest properties
    async fn update_small_upload(
        &self,
        node: &mut raisin_models::nodes::Node,
        properties: std::collections::HashMap<String, PropertyValue>,
        context: &JobContext,
        workspace: &str,
    ) -> Result<String> {
        for (key, value) in properties {
            node.properties.insert(key, value);
        }

        node.properties.insert(
            "status".to_string(),
            PropertyValue::String("ready".to_string()),
        );
        node.properties
            .insert("progress".to_string(), PropertyValue::Float(1.0));

        let tx3 = self.storage.begin_context().await?;
        tx3.set_tenant_repo(&context.tenant_id, &context.repo_id)?;
        tx3.set_branch(&context.branch)?;
        tx3.set_actor("package-processor")?;
        tx3.set_auth_context(AuthContext::system())?;
        tx3.set_message("Update package properties")?;
        tx3.upsert_node(workspace, node).await?;
        tx3.commit().await?;

        Ok(node.id.clone())
    }
    /// Report progress to job registry
    async fn report_progress(&self, job_id: &JobId, progress: f32, message: &str) {
        tracing::debug!(job_id = %job_id, progress = %progress, message = %message, "Package process progress");
        if let Err(e) = self.job_registry.update_progress(job_id, progress).await {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to update job progress");
        }
    }

    /// Extract manifest.yaml from ZIP
    fn extract_manifest(zip_data: &[u8]) -> Result<PackageManifest> {
        let cursor = Cursor::new(zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

        let mut manifest_file = archive
            .by_name("manifest.yaml")
            .map_err(|_| Error::Validation("Package must contain manifest.yaml".to_string()))?;

        let mut manifest_content = String::new();
        manifest_file
            .read_to_string(&mut manifest_content)
            .map_err(|e| Error::Validation(format!("Failed to read manifest.yaml: {}", e)))?;

        serde_yaml::from_str(&manifest_content)
            .map_err(|e| Error::Validation(format!("Invalid manifest.yaml format: {}", e)))
    }
}
