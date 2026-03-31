//! Package asset extraction from ZIP archives
//!
//! Handles extraction of README.md and static/* files from `.rap` packages,
//! storing them as raisin:Asset child nodes of the package node.

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::JobId;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::Arc;
use zip::ZipArchive;

use super::handler::PackageProcessHandler;

/// Compute SHA-256 hash of data for content deduplication
pub(crate) fn compute_content_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

impl<S: Storage + TransactionalStorage> PackageProcessHandler<S> {
    /// Extract package assets (README.md, static/* files) as children of the package node
    ///
    /// This function handles:
    /// - README.md at package root -> installed as raisin:Asset child with text/markdown mime type
    /// - static/* files -> installed as raisin:Asset children under a static/ folder
    ///
    /// These assets are stored in the `packages` workspace as children of the package node,
    /// allowing the admin console to render README with images.
    pub(super) async fn extract_package_assets(
        &self,
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        package_node_id: &str,
        package_name: &str,
        job_id: &JobId,
    ) -> Result<()> {
        let workspace = "packages";
        let binary_store = self.binary_store_callback.as_ref();

        // Skip if no binary storage callback configured
        let Some(binary_store) = binary_store else {
            tracing::warn!(
                job_id = %job_id,
                "Skipping package assets: no binary storage callback configured"
            );
            return Ok(());
        };

        // Collect README and static files from ZIP
        let cursor = Cursor::new(zip_data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| Error::Validation(format!("Invalid ZIP file: {}", e)))?;

        let mut readme_data: Option<Vec<u8>> = None;
        let mut static_files: Vec<(String, Vec<u8>)> = Vec::new(); // (relative_path, data)
        let mut teaser_background_url: Option<String> = None;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| Error::storage(format!("Failed to read ZIP entry: {}", e)))?;

            let name = file.name().to_string();

            // Check for README.md at root (case-insensitive)
            if name.eq_ignore_ascii_case("readme.md") && !file.is_dir() {
                let mut data = Vec::new();
                file.read_to_end(&mut data)
                    .map_err(|e| Error::storage(format!("Failed to read README.md: {}", e)))?;
                readme_data = Some(data);
                tracing::debug!(
                    job_id = %job_id,
                    "Found README.md in package root"
                );
            }
            // Check for static/ directory files
            else if name.starts_with("static/") && !file.is_dir() {
                let relative_path = name.strip_prefix("static/").unwrap_or(&name).to_string();
                if !relative_path.is_empty() {
                    let mut data = Vec::new();
                    file.read_to_end(&mut data).map_err(|e| {
                        Error::storage(format!("Failed to read static file {}: {}", name, e))
                    })?;

                    // Check for teaser background image
                    // Store just the relative path from package root (e.g., "static/teaser_background.png")
                    // The frontend will prepend the package path when building the full URL
                    if relative_path.starts_with("teaser_background.") {
                        teaser_background_url = Some(format!("static/{}", relative_path));
                    }

                    static_files.push((relative_path, data));
                    tracing::debug!(
                        job_id = %job_id,
                        file = %name,
                        "Found static file in package"
                    );
                }
            }
        }

        // Skip if no package assets found
        if readme_data.is_none() && static_files.is_empty() {
            tracing::debug!(
                job_id = %job_id,
                "No package assets (README.md or static/) found"
            );
            return Ok(());
        }

        // Begin transaction for package assets
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant_id, repo_id)?;
        tx.set_branch(branch)?;
        tx.set_actor("package-process")?;
        tx.set_message("Package process: adding package assets (README, static)")?;
        tx.set_auth_context(AuthContext::system())?;

        let mut assets_installed = 0;

        // Install README.md as raisin:Asset child of package
        if let Some(data) = readme_data {
            let readme_path = format!("/{}/README.md", package_name);
            let content_hash = compute_content_hash(&data);

            // Upload README to binary storage
            let stored = binary_store(
                data.clone(),
                Some("text/markdown".to_string()),
                Some("md".to_string()),
                Some("README.md".to_string()),
                None,
            )
            .await?;

            // Build Resource property
            use raisin_models::nodes::properties::value::Resource;
            let mut resource_metadata = HashMap::new();
            resource_metadata.insert(
                "storage_key".to_string(),
                PropertyValue::String(stored.key.clone()),
            );

            let resource = Resource {
                uuid: nanoid::nanoid!(),
                name: stored.name.clone(),
                size: Some(stored.size),
                mime_type: Some("text/markdown".to_string()),
                url: Some(stored.url.clone()),
                metadata: Some(resource_metadata),
                is_loaded: Some(true),
                is_external: Some(false),
                created_at: stored.created_at.into(),
                updated_at: stored.updated_at.into(),
            };

            let mut properties = HashMap::new();
            properties.insert(
                "title".to_string(),
                PropertyValue::String("README".to_string()),
            );
            properties.insert("file".to_string(), PropertyValue::Resource(resource));
            properties.insert(
                "file_type".to_string(),
                PropertyValue::String("text/markdown".to_string()),
            );
            properties.insert("file_size".to_string(), PropertyValue::Integer(stored.size));
            properties.insert(
                "content_hash".to_string(),
                PropertyValue::String(content_hash),
            );

            let readme_node = Node {
                id: nanoid::nanoid!(),
                node_type: "raisin:Asset".to_string(),
                name: "README.md".to_string(),
                path: readme_path.clone(),
                workspace: Some(workspace.to_string()),
                parent: Some(package_node_id.to_string()),
                properties,
                ..Default::default()
            };

            tx.upsert_deep_node(workspace, &readme_node, "raisin:Folder")
                .await?;
            assets_installed += 1;

            tracing::info!(
                job_id = %job_id,
                path = %readme_path,
                "Installed README.md as package asset"
            );
        }

        // Create static/ folder and install static files
        if !static_files.is_empty() {
            let static_folder_path = format!("/{}/static", package_name);

            // Create static folder if needed
            let folder_exists = tx
                .get_node_by_path(workspace, &static_folder_path)
                .await?
                .is_some();
            if !folder_exists {
                let static_folder = Node {
                    id: nanoid::nanoid!(),
                    node_type: "raisin:Folder".to_string(),
                    name: "static".to_string(),
                    path: static_folder_path.clone(),
                    workspace: Some(workspace.to_string()),
                    parent: Some(package_node_id.to_string()),
                    properties: HashMap::new(),
                    ..Default::default()
                };
                tx.upsert_deep_node(workspace, &static_folder, "raisin:Folder")
                    .await?;
                tracing::debug!(
                    job_id = %job_id,
                    path = %static_folder_path,
                    "Created static folder for package"
                );
            }

            // Install each static file as raisin:Asset
            for (relative_path, data) in static_files {
                let filename = relative_path.rsplit('/').next().unwrap_or(&relative_path);
                let asset_path = format!("/{}/static/{}", package_name, relative_path);
                let content_hash = compute_content_hash(&data);

                // Detect MIME type
                let mime_type = mime_guess::from_path(filename)
                    .first()
                    .map(|m| m.to_string());

                // Get file extension
                let ext = std::path::Path::new(filename)
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string());

                // Upload to binary storage
                let stored = binary_store(
                    data.clone(),
                    mime_type.clone(),
                    ext,
                    Some(filename.to_string()),
                    None,
                )
                .await?;

                // Build Resource property
                use raisin_models::nodes::properties::value::Resource;
                let mut resource_metadata = HashMap::new();
                resource_metadata.insert(
                    "storage_key".to_string(),
                    PropertyValue::String(stored.key.clone()),
                );

                let resource = Resource {
                    uuid: nanoid::nanoid!(),
                    name: stored.name.clone(),
                    size: Some(stored.size),
                    mime_type: mime_type.clone(),
                    url: Some(stored.url.clone()),
                    metadata: Some(resource_metadata),
                    is_loaded: Some(true),
                    is_external: Some(false),
                    created_at: stored.created_at.into(),
                    updated_at: stored.updated_at.into(),
                };

                let mut properties = HashMap::new();
                properties.insert(
                    "title".to_string(),
                    PropertyValue::String(filename.to_string()),
                );
                properties.insert("file".to_string(), PropertyValue::Resource(resource));
                properties.insert(
                    "file_type".to_string(),
                    PropertyValue::String(
                        mime_type.unwrap_or_else(|| "application/octet-stream".to_string()),
                    ),
                );
                properties.insert("file_size".to_string(), PropertyValue::Integer(stored.size));
                properties.insert(
                    "content_hash".to_string(),
                    PropertyValue::String(content_hash),
                );

                let asset_node = Node {
                    id: nanoid::nanoid!(),
                    node_type: "raisin:Asset".to_string(),
                    name: filename.to_string(),
                    path: asset_path.clone(),
                    workspace: Some(workspace.to_string()),
                    properties,
                    ..Default::default()
                };

                tx.upsert_deep_node(workspace, &asset_node, "raisin:Folder")
                    .await?;
                assets_installed += 1;

                tracing::debug!(
                    job_id = %job_id,
                    path = %asset_path,
                    "Installed static file as package asset"
                );
            }
        }

        // If we found a teaser background, update the package node
        if let Some(teaser_url) = teaser_background_url {
            if let Some(mut pkg_node) = tx
                .get_node_by_path(workspace, &format!("/{}", package_name))
                .await?
            {
                pkg_node.properties.insert(
                    "teaser_background_url".to_string(),
                    PropertyValue::String(teaser_url.clone()),
                );
                tx.upsert_deep_node(workspace, &pkg_node, "raisin:Folder")
                    .await?;
                tracing::info!(
                    job_id = %job_id,
                    teaser_url = %teaser_url,
                    "Set teaser_background_url on package node"
                );
            }
        }

        tx.commit().await?;

        tracing::info!(
            job_id = %job_id,
            package = %package_name,
            assets_installed = assets_installed,
            "Package assets extraction complete"
        );

        Ok(())
    }
}
