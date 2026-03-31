//! Package update application logic.

use std::collections::HashMap;

use chrono::Utc;
use raisin_binary::BinaryStorage;
use raisin_core::package_init::BuiltinPackageInfo;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::system_updates::{AppliedDefinition, PendingUpdate, ResourceType};
use raisin_storage::{NodeRepository, Storage, StorageScope, SystemUpdateRepository};

use crate::error::ApiError;
use crate::state::AppState;

/// Apply a single Package system update.
///
/// Creates a ZIP from the embedded package files, stores the binary,
/// creates or updates the package node, and queues an installation job.
pub(super) async fn apply_package_update(
    state: &AppState,
    system_update_repo: &impl SystemUpdateRepository,
    rocksdb: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    update: &PendingUpdate,
    packages: &[BuiltinPackageInfo],
) -> Result<bool, ApiError> {
    let Some(package_info) = packages.iter().find(|p| p.manifest.name == update.name) else {
        return Ok(false);
    };

    // Get the embedded directory for this package
    let Some(package_dir) = raisin_core::package_init::get_builtin_package_dir(&update.name) else {
        tracing::warn!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            package = %update.name,
            "Could not find embedded directory for builtin package"
        );
        return Ok(false);
    };

    // Create ZIP from embedded files
    let zip_data = raisin_core::package_init::create_package_zip(package_dir)
        .map_err(|e| ApiError::internal(format!("Failed to create package ZIP: {}", e)))?;

    // Store the binary
    let filename = format!(
        "{}-{}.rap",
        package_info.manifest.name, package_info.manifest.version
    );
    let stored = state
        .bin
        .put_bytes(
            &zip_data,
            Some("application/zip"),
            Some("rap"),
            Some(&filename),
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to store package file: {}", e)))?;

    // Check if package already exists by path
    let package_path = format!("/{}", package_info.manifest.name);
    let existing = state
        .storage()
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, "packages"),
            &package_path,
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to check existing package: {}", e)))?;

    let node_id = existing
        .as_ref()
        .map(|n| n.id.clone())
        .unwrap_or_else(|| format!("package-{}", package_info.manifest.name));

    let install_mode = if existing.is_some() {
        "overwrite"
    } else {
        "skip"
    };

    // Build package node properties
    let properties =
        build_package_properties(package_info, &stored.key, &stored.url, zip_data.len());

    persist_package_node(
        state,
        tenant_id,
        repo_id,
        branch,
        package_info,
        existing,
        &node_id,
        properties,
    )
    .await?;

    // Queue installation job
    let job_id = queue_install_job(
        rocksdb,
        package_info,
        &node_id,
        &stored.key,
        install_mode,
        tenant_id,
        repo_id,
        branch,
    )
    .await?;

    // Record the applied hash
    system_update_repo
        .set_applied(
            tenant_id,
            repo_id,
            ResourceType::Package,
            &package_info.manifest.name,
            AppliedDefinition {
                content_hash: package_info.content_hash.clone(),
                applied_version: None, // Package version is a string, not i32
                applied_at: Utc::now(),
                applied_by: "admin".to_string(),
            },
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to record applied hash: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        package = %update.name,
        job_id = %job_id,
        "Applied Package system update and queued installation job"
    );

    Ok(true)
}

/// Build the properties HashMap for a package node.
fn build_package_properties(
    package_info: &BuiltinPackageInfo,
    resource_key: &str,
    resource_url: &str,
    zip_size: usize,
) -> HashMap<String, PropertyValue> {
    let mut properties = HashMap::new();
    properties.insert(
        "name".to_string(),
        PropertyValue::String(package_info.manifest.name.clone()),
    );
    properties.insert(
        "version".to_string(),
        PropertyValue::String(package_info.manifest.version.clone()),
    );

    if let Some(title) = &package_info.manifest.title {
        properties.insert("title".to_string(), PropertyValue::String(title.clone()));
    }
    if let Some(description) = &package_info.manifest.description {
        properties.insert(
            "description".to_string(),
            PropertyValue::String(description.clone()),
        );
    }
    if let Some(author) = &package_info.manifest.author {
        properties.insert("author".to_string(), PropertyValue::String(author.clone()));
    }
    if let Some(license) = &package_info.manifest.license {
        properties.insert(
            "license".to_string(),
            PropertyValue::String(license.clone()),
        );
    }
    properties.insert(
        "icon".to_string(),
        PropertyValue::String(package_info.manifest.icon.clone()),
    );
    properties.insert(
        "color".to_string(),
        PropertyValue::String(package_info.manifest.color.clone()),
    );
    if !package_info.manifest.keywords.is_empty() {
        properties.insert(
            "keywords".to_string(),
            PropertyValue::Array(
                package_info
                    .manifest
                    .keywords
                    .iter()
                    .map(|k| PropertyValue::String(k.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(category) = &package_info.manifest.category {
        properties.insert(
            "category".to_string(),
            PropertyValue::String(category.clone()),
        );
    }

    // Mark as builtin
    properties.insert("builtin".to_string(), PropertyValue::Boolean(true));

    // Set installed to false (will be set to true after installation job completes)
    properties.insert("installed".to_string(), PropertyValue::Boolean(false));

    // Add resource reference with the new ZIP key/url
    let mut resource_obj = HashMap::new();
    resource_obj.insert(
        "key".to_string(),
        PropertyValue::String(resource_key.to_string()),
    );
    resource_obj.insert(
        "url".to_string(),
        PropertyValue::String(resource_url.to_string()),
    );
    resource_obj.insert(
        "mime_type".to_string(),
        PropertyValue::String("application/zip".to_string()),
    );
    resource_obj.insert("size".to_string(), PropertyValue::Integer(zip_size as i64));
    properties.insert("resource".to_string(), PropertyValue::Object(resource_obj));

    properties
}

/// Create or update the package node in storage.
async fn persist_package_node(
    state: &AppState,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    package_info: &BuiltinPackageInfo,
    existing: Option<raisin_models::nodes::Node>,
    node_id: &str,
    properties: HashMap<String, PropertyValue>,
) -> Result<(), ApiError> {
    if let Some(existing_node) = existing {
        // UPDATE existing node with new properties (including the new resource reference)
        let updated_node = raisin_models::nodes::Node {
            id: existing_node.id.clone(),
            node_type: existing_node.node_type.clone(),
            name: package_info.manifest.name.clone(),
            path: format!("/{}", package_info.manifest.name),
            workspace: Some("packages".to_string()),
            properties,
            created_at: existing_node.created_at,
            updated_at: Some(Utc::now()),
            ..Default::default()
        };

        state
            .storage()
            .nodes()
            .update(
                StorageScope::new(tenant_id, repo_id, branch, "packages"),
                updated_node,
                raisin_storage::node_operations::UpdateNodeOptions::default(),
            )
            .await
            .map_err(|e| ApiError::internal(format!("Failed to update package node: {}", e)))?;

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            package = %package_info.manifest.name,
            "Updated existing package node with new resource"
        );
    } else {
        // CREATE new node
        let node = raisin_models::nodes::Node {
            id: node_id.to_string(),
            node_type: "raisin:Package".to_string(),
            name: package_info.manifest.name.clone(),
            path: format!("/{}", package_info.manifest.name),
            workspace: Some("packages".to_string()),
            properties,
            ..Default::default()
        };

        state
            .storage()
            .nodes()
            .create(
                StorageScope::new(tenant_id, repo_id, branch, "packages"),
                node,
                raisin_storage::node_operations::CreateNodeOptions::default(),
            )
            .await
            .map_err(|e| ApiError::internal(format!("Failed to create package node: {}", e)))?;
    }
    Ok(())
}

/// Queue a package installation job via the unified job queue.
async fn queue_install_job(
    rocksdb: &RocksDBStorage,
    package_info: &BuiltinPackageInfo,
    node_id: &str,
    resource_key: &str,
    install_mode: &str,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<String, ApiError> {
    let job_type = raisin_storage::JobType::PackageInstall {
        package_name: package_info.manifest.name.clone(),
        package_version: package_info.manifest.version.clone(),
        package_node_id: node_id.to_string(),
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "resource_key".to_string(),
        serde_json::Value::String(resource_key.to_string()),
    );
    metadata.insert(
        "install_mode".to_string(),
        serde_json::Value::String(install_mode.to_string()),
    );

    let job_context = raisin_storage::jobs::JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: "packages".to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata,
    };

    let job_registry = rocksdb.job_registry();
    let job_data_store = rocksdb.job_data_store();

    let job_id = job_registry
        .register_job(job_type, Some(tenant_id.to_string()), None, None, None)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to register job: {}", e)))?;

    job_data_store
        .put(&job_id, &job_context)
        .map_err(|e| ApiError::internal(format!("Failed to store job context: {}", e)))?;

    Ok(job_id.to_string())
}
