// SPDX-License-Identifier: BSL-1.1

//! Package export and download handlers.
//!
//! Handles exporting installed packages as `.rap` files
//! and downloading previously exported packages.

use axum::{
    extract::{Extension, Path, State},
    response::{IntoResponse, Response},
    Json,
};
use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use std::collections::HashMap;

#[cfg(feature = "storage-rocksdb")]
use raisin_storage::JobType;

use crate::{error::ApiError, state::AppState};

use super::types::{ExportPackageRequest, ExportResponse};

/// Export a package as .rap file (current installed state).
///
/// POST /api/packages/{repo}/{branch}/head/{package_path}/raisin:export
pub async fn export_package(
    State(state): State<AppState>,
    Path((repo, branch, package_path)): Path<(String, String, String)>,
    Json(request): Json<ExportPackageRequest>,
) -> Result<Json<ExportResponse>, ApiError> {
    let tenant_id = "default";
    let workspace = "packages";

    let node_service = state.node_service_for_context(tenant_id, &repo, &branch, workspace, None);
    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let package_name = node.name.clone();

    create_export_job(
        &state,
        tenant_id,
        &repo,
        &branch,
        workspace,
        &package_name,
        &node.id,
        &package_path,
        &request,
    )
    .await
}

/// Export a package (internal command endpoint implementation).
pub(super) async fn export_package_impl(
    state: &AppState,
    repo: &str,
    branch: &str,
    package_path: &str,
    request: ExportPackageRequest,
    auth_context: Option<AuthContext>,
) -> Result<ExportResponse, ApiError> {
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

    create_export_job(
        state,
        tenant_id,
        repo,
        branch,
        workspace,
        &package_name,
        &node.id,
        package_path,
        &request,
    )
    .await
    .map(|Json(resp)| resp)
}

/// Download exported package implementation.
pub(super) async fn download_exported_package_impl(
    state: &AppState,
    _repo: &str,
    _branch: &str,
    _package_path: &str,
    job_id: &str,
    _auth_context: Option<AuthContext>,
) -> Result<Response, ApiError> {
    #[cfg(feature = "storage-rocksdb")]
    {
        let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
            ApiError::storage_error("RocksDB storage not available for job system")
        })?;

        let job_registry = rocksdb.job_registry();
        let job_id_typed = raisin_storage::jobs::JobId(job_id.to_string());
        let job = job_registry
            .get_job_info(&job_id_typed)
            .await
            .map_err(|e| ApiError::storage_error(format!("Failed to get job: {}", e)))?;

        if job.status != raisin_storage::jobs::JobStatus::Completed {
            return Err(ApiError::validation_failed(format!(
                "Export job is not complete. Status: {:?}",
                job.status
            )));
        }

        let result = job
            .result
            .ok_or_else(|| ApiError::storage_error("Export job completed but has no result"))?;

        let blob_key = result
            .get("blob_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ApiError::storage_error("Export job result missing blob_key"))?;

        let package_name = result
            .get("package_name")
            .and_then(|v| v.as_str())
            .unwrap_or("package");

        let zip_data = state.bin.get(blob_key).await.map_err(|e| {
            ApiError::storage_error(format!("Failed to retrieve exported package: {}", e))
        })?;

        let filename = format!("{}.rap", package_name);
        Ok((
            [
                (axum::http::header::CONTENT_TYPE, "application/zip"),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    &format!("attachment; filename=\"{}\"", filename),
                ),
            ],
            zip_data.to_vec(),
        )
            .into_response())
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        Err(ApiError::storage_error(
            "Package download requires RocksDB backend",
        ))
    }
}

/// Create a background job for package export.
async fn create_export_job(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    package_name: &str,
    node_id: &str,
    package_path: &str,
    request: &ExportPackageRequest,
) -> Result<Json<ExportResponse>, ApiError> {
    #[cfg(feature = "storage-rocksdb")]
    {
        let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
            ApiError::storage_error("RocksDB storage not available for job system")
        })?;

        let job_registry = rocksdb.job_registry();
        let job_data_store = rocksdb.job_data_store();

        let job_type = JobType::PackageExport {
            package_name: package_name.to_string(),
            package_node_id: node_id.to_string(),
            export_mode: request.export_mode.clone(),
            include_modifications: request.include_modifications,
        };

        let mut metadata = HashMap::new();
        metadata.insert("package_name".to_string(), serde_json::json!(package_name));
        metadata.insert(
            "export_mode".to_string(),
            serde_json::json!(request.export_mode),
        );

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
                ApiError::storage_error(format!("Failed to register export job: {}", e))
            })?;

        job_data_store
            .put(&job_id, &job_context)
            .map_err(|e| ApiError::storage_error(format!("Failed to store job context: {}", e)))?;

        tracing::info!(
            job_id = %job_id,
            package = %package_name,
            mode = %request.export_mode,
            "Created package export job"
        );

        let download_path = format!(
            "/api/packages/{}/{}/head/{}/raisin:download/{}",
            repo, branch, package_path, job_id
        );

        Ok(Json(ExportResponse {
            job_id: job_id.to_string(),
            status: "scheduled".to_string(),
            download_path,
        }))
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        Err(ApiError::storage_error(
            "Package export requires RocksDB backend with job system",
        ))
    }
}
