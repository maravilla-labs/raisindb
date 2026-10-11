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

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::types::{ExportPackageRequest, ExportResponse};

/// Export a package as .rap file (current installed state).
///
/// POST /api/packages/{repo}/{branch}/head/{package_path}/raisin:export
pub async fn export_package(
    State(state): State<AppState>,
    Path((repo, branch, package_path)): Path<(String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(request): Json<ExportPackageRequest>,
) -> Result<Json<ExportResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    super::require_export_access(auth_context.as_ref())?;
    let tenant_id = tenant_info.tenant_id.as_str();
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, &branch, workspace, auth_context.clone());
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
        auth_context.as_ref(),
    )
    .await
}

/// Export a package (internal command endpoint implementation).
pub(super) async fn export_package_impl(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    package_path: &str,
    request: ExportPackageRequest,
    auth_context: Option<AuthContext>,
) -> Result<ExportResponse, ApiError> {
    // E3: an export produces a downloadable archive; it is gated exactly like
    // create-from-selection.
    super::require_export_access(auth_context.as_ref())?;
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context.clone());
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
        auth_context.as_ref(),
    )
    .await
    .map(|Json(resp)| resp)
}

/// Download exported package implementation.
///
/// E3: checked on EVERY request, before the job registry is consulted: the
/// caller must be allowed to export packages, and the job must be a package
/// job of the caller's own tenant and repository. Any job that fails the
/// scoping is answered exactly like one that does not exist, so a job id from
/// another tenant reveals nothing.
pub(super) async fn download_exported_package_impl(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    _branch: &str,
    _package_path: &str,
    job_id: &str,
    auth_context: Option<AuthContext>,
) -> Result<Response, ApiError> {
    super::require_export_access(auth_context.as_ref())?;

    #[cfg(feature = "storage-rocksdb")]
    {
        use raisin_core::services::package_export_access;

        let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
            ApiError::storage_error("RocksDB storage not available for job system")
        })?;

        let not_found = || ApiError::not_found(format!("Export job '{}' not found", job_id));

        let job_registry = rocksdb.job_registry();
        let job_id_typed = raisin_storage::jobs::JobId(job_id.to_string());
        let job = job_registry
            .get_job_info(&job_id_typed)
            .await
            .map_err(|_| not_found())?;

        let is_package_job = matches!(
            job.job_type,
            JobType::PackageExport { .. } | JobType::PackageCreateFromSelection { .. }
        );
        // A finished job's context is deleted by the worker, so the result's
        // stamp is the authority; a job still in flight answers from its
        // context. A completed job with neither is refused (fail closed).
        let job_repo = match job.result.as_ref() {
            Some(result) => package_export_access::result_repo(result).map(str::to_string),
            None => rocksdb
                .job_data_store()
                .get(&job.tenant, &job_id_typed)
                .ok()
                .flatten()
                .map(|ctx| ctx.repo_id),
        };
        if !package_export_access::is_downloadable_job(
            tenant_id,
            repo,
            &job.tenant,
            job_repo.as_deref(),
            is_package_job,
        ) {
            tracing::warn!(
                job_id = %job_id,
                request_tenant = %tenant_id,
                request_repo = %repo,
                "Package download refused: job is not a package job of this tenant/repository"
            );
            return Err(not_found());
        }

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
        let _ = (state, tenant_id, repo, job_id);
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
    requester: Option<&AuthContext>,
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
        if let Some(requester) = requester {
            metadata.extend(
                raisin_core::services::package_export_access::requester_metadata(requester),
            );
        }

        let job_context = raisin_storage::jobs::JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo.to_string(),
            branch: branch.to_string(),
            workspace_id: workspace.to_string(),
            revision: raisin_hlc::HLC::now(),
            metadata,
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        job_data_store
            .put(&job_id, &job_context)
            .map_err(|e| ApiError::storage_error(format!("Failed to store job context: {}", e)))?;

        job_registry
            .register_job_with_id(
                job_id.clone(),
                job_type,
                tenant_id.to_string(),
                None,
                None,
                None,
            )
            .await
            .map_err(|e| {
                ApiError::storage_error(format!("Failed to register export job: {}", e))
            })?;

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
