// SPDX-License-Identifier: BSL-1.1

//! Package install/uninstall handlers.
//!
//! Handles installing packages by creating background jobs,
//! uninstalling packages, and dry run previews.

use axum::{
    extract::{Extension, Path, State},
    Json,
};
use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use std::collections::HashMap;

#[cfg(feature = "storage-rocksdb")]
use raisin_storage::JobType;

use crate::{error::ApiError, state::AppState};

use super::types::{
    ActionCounts, DryRunLogEntry, DryRunResponse, DryRunSummary, InstallMode, InstallResponse,
};

/// Install a package (extract and apply node types, workspaces, content).
///
/// Creates a background job to process the installation asynchronously.
///
/// POST /api/repos/{repo}/packages/{name}/install
pub async fn install_package(
    State(state): State<AppState>,
    Path((repo, package_name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<InstallResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = "default";
    let branch = "main";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, branch, workspace, auth_context);
    let node_id = format!("package-{}", package_name);

    let node = node_service
        .get(&node_id)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_name)))?;

    let already_installed = node
        .properties
        .get("installed")
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false);

    let version = node
        .properties
        .get("version")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    if already_installed {
        let installed_at = node.properties.get("installed_at").and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            PropertyValue::Date(dt) => Some(dt.to_string()),
            _ => None,
        });

        return Ok(Json(InstallResponse {
            package_name,
            version,
            installed: true,
            installed_at,
            job_id: None,
        }));
    }

    let resource = node
        .properties
        .get("resource")
        .ok_or_else(|| ApiError::validation_failed("Package has no resource"))?;

    let resource_obj = match resource {
        PropertyValue::Object(obj) => obj,
        _ => {
            return Err(ApiError::validation_failed(
                "Resource is not a valid object",
            ))
        }
    };

    let resource_key = resource_obj
        .get("key")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| ApiError::validation_failed("Resource has no key"))?;

    create_install_job(
        &state,
        tenant_id,
        &repo,
        branch,
        workspace,
        &package_name,
        &version,
        &node_id,
        &resource_key,
        None, // no install_mode override
    )
    .await
}

/// Uninstall a package (mark as not installed).
///
/// POST /api/repos/{repo}/packages/{name}/uninstall
pub async fn uninstall_package(
    State(state): State<AppState>,
    Path((repo, package_name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<InstallResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = "default";
    let branch = "main";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, branch, workspace, auth_context);
    let node_id = format!("package-{}", package_name);

    let mut node = node_service
        .get(&node_id)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_name)))?;

    let version = node
        .properties
        .get("version")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    node.properties
        .insert("installed".to_string(), PropertyValue::Boolean(false));
    node.properties.remove("installed_at");

    node_service
        .upsert(node)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to update package node: {}", e)))?;

    Ok(Json(InstallResponse {
        package_name,
        version,
        installed: false,
        installed_at: None,
        job_id: None,
    }))
}

/// Install a package via the unified command endpoint.
pub(super) async fn install_package_impl(
    state: &AppState,
    repo: &str,
    branch: &str,
    package_path: &str,
    mode: InstallMode,
    auth_context: Option<AuthContext>,
) -> Result<InstallResponse, ApiError> {
    let tenant_id = "default";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context);

    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let package_name = node.name.clone();

    let already_installed = node
        .properties
        .get("installed")
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false);

    let version = node
        .properties
        .get("version")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    if already_installed && mode == InstallMode::Skip {
        let installed_at = node.properties.get("installed_at").and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            PropertyValue::Date(dt) => Some(dt.to_string()),
            _ => None,
        });

        return Ok(InstallResponse {
            package_name,
            version,
            installed: true,
            installed_at,
            job_id: None,
        });
    }

    let resource = node
        .properties
        .get("resource")
        .ok_or_else(|| ApiError::validation_failed("Package has no resource"))?;

    let resource_obj = match resource {
        PropertyValue::Object(obj) => obj,
        _ => {
            return Err(ApiError::validation_failed(
                "Resource is not a valid object",
            ))
        }
    };

    let resource_key = resource_obj
        .get("key")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| ApiError::validation_failed("Resource has no key"))?;

    create_install_job(
        state,
        tenant_id,
        repo,
        branch,
        workspace,
        &package_name,
        &version,
        &node.id,
        &resource_key,
        Some(mode),
    )
    .await
    .map(|Json(resp)| resp)
}

/// Dry run implementation - simulates installation without making changes.
pub(super) async fn dry_run_impl(
    state: &AppState,
    repo: &str,
    branch: &str,
    package_path: &str,
    mode: InstallMode,
    auth_context: Option<AuthContext>,
) -> Result<DryRunResponse, ApiError> {
    let tenant_id = "default";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context.clone());

    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let package_name = node.name.clone();
    let version = node
        .properties
        .get("version")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    let resource = node
        .properties
        .get("resource")
        .ok_or_else(|| ApiError::validation_failed("Package has no resource"))?;

    let resource_obj = match resource {
        PropertyValue::Object(obj) => obj,
        _ => {
            return Err(ApiError::validation_failed(
                "Resource is not a valid object",
            ))
        }
    };

    let resource_key = resource_obj
        .get("key")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| ApiError::validation_failed("Resource has no key"))?;

    let zip_data = state.bin.get(&resource_key).await.map_err(|e| {
        ApiError::storage_error(format!("Failed to retrieve package binary: {}", e))
    })?;

    #[cfg(feature = "storage-rocksdb")]
    {
        use raisin_binary::BinaryStorage;
        use raisin_rocksdb::{PackageInstallHandler, PackageInstallMode};
        use std::sync::Arc;

        let rocksdb = state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| ApiError::storage_error("RocksDB storage not available"))?;

        let handler_mode = match mode {
            InstallMode::Skip => PackageInstallMode::Skip,
            InstallMode::Overwrite => PackageInstallMode::Overwrite,
            InstallMode::Sync => PackageInstallMode::Sync,
        };

        let handler =
            PackageInstallHandler::new(Arc::clone(rocksdb), rocksdb.job_registry().clone());

        let dry_run_result = handler
            .dry_run(tenant_id, repo, branch, &zip_data, handler_mode)
            .await
            .map_err(|e| ApiError::storage_error(format!("Dry run failed: {}", e)))?;

        let logs: Vec<DryRunLogEntry> = dry_run_result
            .logs
            .into_iter()
            .map(|log| DryRunLogEntry {
                level: log.level,
                category: log.category,
                path: log.path,
                message: log.message,
                action: log.action,
            })
            .collect();

        let summary = DryRunSummary {
            node_types: ActionCounts {
                create: dry_run_result.summary.node_types.create,
                update: dry_run_result.summary.node_types.update,
                skip: dry_run_result.summary.node_types.skip,
            },
            archetypes: ActionCounts {
                create: dry_run_result.summary.archetypes.create,
                update: dry_run_result.summary.archetypes.update,
                skip: dry_run_result.summary.archetypes.skip,
            },
            element_types: ActionCounts {
                create: dry_run_result.summary.element_types.create,
                update: dry_run_result.summary.element_types.update,
                skip: dry_run_result.summary.element_types.skip,
            },
            workspaces: ActionCounts {
                create: dry_run_result.summary.workspaces.create,
                update: dry_run_result.summary.workspaces.update,
                skip: dry_run_result.summary.workspaces.skip,
            },
            content_nodes: ActionCounts {
                create: dry_run_result.summary.content_nodes.create,
                update: dry_run_result.summary.content_nodes.update,
                skip: dry_run_result.summary.content_nodes.skip,
            },
            binary_files: ActionCounts {
                create: dry_run_result.summary.binary_files.create,
                update: dry_run_result.summary.binary_files.update,
                skip: dry_run_result.summary.binary_files.skip,
            },
            package_assets: ActionCounts {
                create: dry_run_result.summary.package_assets.create,
                update: dry_run_result.summary.package_assets.update,
                skip: dry_run_result.summary.package_assets.skip,
            },
        };

        Ok(DryRunResponse {
            package_name,
            package_version: version,
            mode,
            logs,
            summary,
        })
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        Err(ApiError::storage_error("Dry run requires RocksDB backend"))
    }
}

/// Create a background job for package installation.
async fn create_install_job(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    package_name: &str,
    version: &str,
    node_id: &str,
    resource_key: &str,
    install_mode: Option<InstallMode>,
) -> Result<Json<InstallResponse>, ApiError> {
    #[cfg(feature = "storage-rocksdb")]
    {
        let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
            ApiError::storage_error("RocksDB storage not available for job system")
        })?;

        let job_registry = rocksdb.job_registry();
        let job_data_store = rocksdb.job_data_store();

        let job_type = JobType::PackageInstall {
            package_name: package_name.to_string(),
            package_version: version.to_string(),
            package_node_id: node_id.to_string(),
        };

        let mut metadata = HashMap::new();
        metadata.insert("resource_key".to_string(), serde_json::json!(resource_key));
        metadata.insert("package_name".to_string(), serde_json::json!(package_name));
        if let Some(mode) = install_mode {
            metadata.insert("install_mode".to_string(), serde_json::json!(mode));
        }

        let job_context = raisin_storage::jobs::JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo.to_string(),
            branch: branch.to_string(),
            workspace_id: workspace.to_string(),
            revision: raisin_hlc::HLC::now(),
            metadata,
        };

        let job_id = job_registry
            .register_job(job_type, Some(tenant_id.to_string()), None, None, None)
            .await
            .map_err(|e| {
                ApiError::storage_error(format!("Failed to register install job: {}", e))
            })?;

        job_data_store
            .put(&job_id, &job_context)
            .map_err(|e| ApiError::storage_error(format!("Failed to store job context: {}", e)))?;

        tracing::info!(
            job_id = %job_id,
            package = %package_name,
            version = %version,
            "Created package installation job"
        );

        Ok(Json(InstallResponse {
            package_name: package_name.to_string(),
            version: version.to_string(),
            installed: false,
            installed_at: None,
            job_id: Some(job_id.to_string()),
        }))
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        Err(ApiError::storage_error(
            "Package installation requires RocksDB backend with job system",
        ))
    }
}
