//! Main handle method for package creation from selection

use super::manifest_types::PackageManifest;
use super::types::{CollectedNode, PackageCreateFromSelectionResult, SelectedPath};
use super::PackageCreateFromSelectionHandler;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope, UpdateNodeOptions};
use std::collections::HashMap;

impl PackageCreateFromSelectionHandler {
    /// Handle package creation job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract parameters from JobType
        let (package_name, package_version, include_node_types) = match &job.job_type {
            JobType::PackageCreateFromSelection {
                package_name,
                package_version,
                include_node_types,
            } => (
                package_name.as_str(),
                package_version.as_str(),
                *include_node_types,
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected PackageCreateFromSelection job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            package_name = %package_name,
            package_version = %package_version,
            include_node_types = %include_node_types,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Starting package creation from selection"
        );

        // Extract selected paths from context metadata
        let selected_paths: Vec<SelectedPath> = context
            .metadata
            .get("selected_paths")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if selected_paths.is_empty() {
            return Err(Error::Validation(
                "No paths selected for package creation".to_string(),
            ));
        }

        // Require binary storage callback
        let binary_store = self.binary_store_callback.as_ref().ok_or_else(|| {
            Error::Validation("Binary storage callback not configured".to_string())
        })?;

        self.report_progress(&job.id, 0.1, "Loading selected content")
            .await;

        // Collect nodes from all selected paths
        let branch = &context.branch;
        let mut all_nodes: Vec<CollectedNode> = Vec::new();
        let mut node_types_used: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (idx, selected) in selected_paths.iter().enumerate() {
            let progress = 0.1 + (0.5 * (idx as f32 / selected_paths.len() as f32));
            self.report_progress(
                &job.id,
                progress,
                &format!("Loading from {}: {}", selected.workspace, selected.path),
            )
            .await;

            // Determine if recursive
            let (path, recursive) = if selected.path.ends_with("/*") {
                (selected.path.trim_end_matches("/*"), true)
            } else {
                (selected.path.as_str(), false)
            };

            // Load nodes at this path
            let nodes = self
                .load_nodes_recursive(
                    &context.tenant_id,
                    &context.repo_id,
                    branch,
                    &selected.workspace,
                    path,
                    recursive,
                )
                .await?;

            for node in nodes {
                if !node.node_type.is_empty() {
                    node_types_used.insert(node.node_type.clone());
                }
                all_nodes.push(CollectedNode {
                    workspace: selected.workspace.clone(),
                    node,
                });
            }
        }

        if all_nodes.is_empty() {
            return Err(Error::Validation(
                "No nodes found at selected paths".to_string(),
            ));
        }

        self.report_progress(&job.id, 0.6, "Loading node type definitions")
            .await;

        // Optionally load node type definitions
        let node_repo = self.storage.nodes();
        let mut node_type_defs: Vec<CollectedNode> = Vec::new();
        if include_node_types && !node_types_used.is_empty() {
            for node_type_name in &node_types_used {
                let path = format!("/{}", node_type_name);
                if let Ok(Some(node)) = node_repo
                    .get_by_path(
                        StorageScope::new(
                            &context.tenant_id,
                            &context.repo_id,
                            branch,
                            "nodetypes",
                        ),
                        &path,
                        None,
                    )
                    .await
                {
                    node_type_defs.push(CollectedNode {
                        workspace: "nodetypes".to_string(),
                        node,
                    });
                }
            }
        }

        self.report_progress(&job.id, 0.7, "Building package").await;

        // Build the package (writes to temp file to support large packages)
        let package_temp_path = self
            .build_package(package_name, package_version, &all_nodes, &node_type_defs)
            .await?;

        self.report_progress(&job.id, 0.9, "Storing package file")
            .await;

        // Read temp file back to Vec<u8> for the binary store callback
        // TODO: Use streaming path-based upload for large packages
        let package_data = std::fs::read(&package_temp_path)
            .map_err(|e| Error::storage(format!("Failed to read temp package file: {}", e)))?;

        // Clean up temp file
        let _ = std::fs::remove_file(&package_temp_path);

        // Store the package
        let filename = format!("{}-{}.rap", package_name, package_version);
        let stored = binary_store(
            package_data.clone(),
            Some("application/zip".to_string()),
            Some("rap".to_string()),
            Some(filename),
            None,
        )
        .await?;

        self.report_progress(&job.id, 0.95, "Creating package node")
            .await;

        // Create a raisin:Package node in the packages workspace
        self.create_or_update_package_node(
            context,
            package_name,
            package_version,
            &stored.key,
            &stored.url,
            package_data.len(),
        )
        .await?;

        tracing::info!(
            job_id = %job.id,
            node_id = %format!("package-{}", package_name),
            "Package node created in packages workspace"
        );

        self.report_progress(&job.id, 1.0, "Package created").await;

        let result = PackageCreateFromSelectionResult {
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
            nodes_included: all_nodes.len(),
            node_types_included: node_type_defs.len(),
            blob_key: stored.key,
            download_url: stored.url,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        tracing::info!(
            job_id = %job.id,
            package_name = %result.package_name,
            nodes = %result.nodes_included,
            node_types = %result.node_types_included,
            blob_key = %result.blob_key,
            "Package creation completed"
        );

        Ok(Some(serde_json::to_value(result).unwrap_or_default()))
    }

    /// Create or update the package node in the packages workspace
    async fn create_or_update_package_node(
        &self,
        context: &JobContext,
        package_name: &str,
        package_version: &str,
        blob_key: &str,
        download_url: &str,
        package_size: usize,
    ) -> Result<()> {
        let node_repo = self.storage.nodes();
        let node_id = format!("package-{}", package_name);

        let mut properties = HashMap::new();
        properties.insert(
            "name".to_string(),
            PropertyValue::String(package_name.to_string()),
        );
        properties.insert(
            "version".to_string(),
            PropertyValue::String(package_version.to_string()),
        );
        properties.insert("installed".to_string(), PropertyValue::Boolean(false));
        properties.insert(
            "description".to_string(),
            PropertyValue::String("Package created from selected content".to_string()),
        );

        // Add resource reference as Object
        let mut resource_obj = HashMap::new();
        resource_obj.insert(
            "key".to_string(),
            PropertyValue::String(blob_key.to_string()),
        );
        resource_obj.insert(
            "url".to_string(),
            PropertyValue::String(download_url.to_string()),
        );
        resource_obj.insert(
            "mime_type".to_string(),
            PropertyValue::String("application/zip".to_string()),
        );
        resource_obj.insert(
            "size".to_string(),
            PropertyValue::Integer(package_size as i64),
        );
        properties.insert("resource".to_string(), PropertyValue::Object(resource_obj));

        let package_node = Node {
            id: node_id,
            node_type: "raisin:Package".to_string(),
            name: package_name.to_string(),
            path: format!("/{}", package_name),
            workspace: Some("packages".to_string()),
            properties,
            ..Default::default()
        };

        let create_options = CreateNodeOptions {
            validate_schema: false,
            validate_parent_allows_child: false,
            validate_workspace_allows_type: false,
            operation_meta: None,
        };

        let package_path = format!("/{}", package_name);
        match node_repo
            .create_deep_node(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    "packages",
                ),
                &package_path,
                package_node.clone(),
                "raisin:Folder",
                create_options.clone(),
            )
            .await
        {
            Ok(_) => {}
            Err(e) if e.to_string().contains("already exists") => {
                node_repo
                    .update(
                        StorageScope::new(
                            &context.tenant_id,
                            &context.repo_id,
                            &context.branch,
                            "packages",
                        ),
                        package_node,
                        UpdateNodeOptions {
                            validate_schema: false,
                            allow_type_change: false,
                            operation_meta: None,
                        },
                    )
                    .await
                    .map_err(|e| Error::storage(format!("Failed to update package node: {}", e)))?;
            }
            Err(e) => {
                return Err(Error::storage(format!(
                    "Failed to create package node: {}",
                    e
                )));
            }
        }

        Ok(())
    }
}
