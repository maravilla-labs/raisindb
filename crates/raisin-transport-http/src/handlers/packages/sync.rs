// SPDX-License-Identifier: BSL-1.1

//! Package sync status, diff, and create-from-selection handlers.
//!
//! Provides endpoints for comparing installed package state with
//! source archives, generating diffs, and creating new packages
//! from selected content paths.

use axum::{
    extract::{Extension, Path, State},
    Json,
};
use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

#[cfg(feature = "storage-rocksdb")]
use raisin_storage::JobType;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::types::{
    CreateFromSelectionRequest, CreateFromSelectionResponse, DiffResponse, SyncStatusResponse,
    SyncSummary,
};

/// Get sync status for a package.
///
/// GET /api/packages/{repo}/{branch}/head/{package_path}/raisin:sync-status
pub async fn get_sync_status(
    State(state): State<AppState>,
    Path((repo, branch, package_path)): Path<(String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<SyncStatusResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let result = get_sync_status_impl(
        &state,
        &tenant_info.tenant_id,
        &repo,
        &branch,
        &package_path,
        auth_context,
    )
    .await?;
    Ok(Json(result))
}

/// Get diff for a specific file in a package.
///
/// GET /api/packages/{repo}/{branch}/head/{package_path}/raisin:diff/{file_path}
pub async fn get_package_diff(
    State(state): State<AppState>,
    Path((repo, branch, package_path, file_path)): Path<(String, String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<DiffResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = tenant_info.tenant_id.as_str();
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, &branch, workspace, auth_context);
    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

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
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .ok_or_else(|| ApiError::validation_failed("Resource has no key"))?;

    let zip_data = state.bin.get(resource_key).await.map_err(|e| {
        ApiError::storage_error(format!("Failed to retrieve package binary: {}", e))
    })?;

    let cursor = Cursor::new(zip_data.to_vec());
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {}", e)))?;

    let server_content = match archive.by_name(&file_path) {
        Ok(mut file) => {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)
                .map_err(|e| ApiError::storage_error(format!("Failed to read file: {}", e)))?;
            String::from_utf8(contents).ok()
        }
        Err(_) => None,
    };

    let local_content: Option<String> = None;

    let unified_diff = match (&local_content, &server_content) {
        (Some(local), Some(server)) => Some(generate_simple_diff(local, server, &file_path)),
        _ => None,
    };

    Ok(Json(DiffResponse {
        path: file_path,
        diff_type: "text".to_string(),
        local_content,
        server_content,
        unified_diff,
    }))
}

/// Create a new package from selected content paths.
///
/// POST /api/packages/{repo}/{branch}/head/raisin:create-from-selection
pub async fn create_package_from_selection(
    State(state): State<AppState>,
    Path((repo, branch)): Path<(String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(request): Json<CreateFromSelectionRequest>,
) -> Result<Json<CreateFromSelectionResponse>, ApiError> {
    // E3: refuse before validating anything, so an unauthorised caller learns
    // nothing about the request shape or the repository.
    let auth_context = auth.map(|Extension(ctx)| ctx);
    super::require_export_access(auth_context.as_ref())?;
    // Only reachable with a context: the gate refuses `None`.
    let Some(requester) = auth_context else {
        return Err(ApiError::unauthorized("package exports require signing in"));
    };

    let tenant_id = tenant_info.tenant_id.as_str();

    if request.name.is_empty() {
        return Err(ApiError::validation_failed("Package name is required"));
    }
    if request.selected_paths.is_empty() {
        return Err(ApiError::validation_failed(
            "At least one path must be selected",
        ));
    }

    let selected_count = request.selected_paths.len();

    #[cfg(feature = "storage-rocksdb")]
    {
        use raisin_rocksdb::package_build::{
            PackageBuildRequest, RecordRefWire, SelectedPath as JobPath,
        };

        let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
            ApiError::storage_error("RocksDB storage not available for job system")
        })?;

        let build = PackageBuildRequest {
            name: request.name.clone(),
            version: request.version.clone(),
            selected_paths: request
                .selected_paths
                .iter()
                .map(|p| JobPath {
                    workspace: p.workspace.clone(),
                    path: p.path.clone(),
                })
                .collect(),
            include_node_types: request.include_node_types,
            record: request.record.as_ref().map(|r| RecordRefWire {
                workspace: r.workspace.clone(),
                path: r.path.clone(),
            }),
            register_package: request.register_package,
            idempotency_key: request.idempotency_key.clone(),
            title: request.title.clone(),
            description: request.description.clone(),
            author: request.author.clone(),
        };
        // One submit path for HTTP and the function binding: validation, the
        // reserved-name guard, idempotency, the requester record (E3).
        let submitted = raisin_rocksdb::package_build::submit_package_build(
            rocksdb, tenant_id, &repo, &branch, &build, &requester,
        )
        .await
        .map_err(ApiError::from)?;

        tracing::info!(
            job_id = %submitted.job_id,
            package = %request.name,
            version = %request.version,
            paths_count = selected_count,
            existing = submitted.existing,
            "Created package-from-selection job"
        );

        let download_path = format!(
            "/api/packages/{}/{}/head/{}/raisin:download/{}",
            repo, branch, request.name, submitted.job_id
        );

        Ok(Json(CreateFromSelectionResponse {
            job_id: submitted.job_id,
            status: "scheduled".to_string(),
            download_path,
            selected_count,
            existing: submitted.existing,
        }))
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (requester, selected_count, tenant_id, state, repo, branch);
        Err(ApiError::storage_error(
            "Package creation requires RocksDB backend with job system",
        ))
    }
}

/// Get sync status for a package (implementation).
pub(super) async fn get_sync_status_impl(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    package_path: &str,
    auth_context: Option<AuthContext>,
) -> Result<SyncStatusResponse, ApiError> {
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context);
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

    let installed = node
        .properties
        .get("installed")
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false);

    let status = if installed { "synced" } else { "not_installed" };

    Ok(SyncStatusResponse {
        package_name,
        package_version: version,
        status: status.to_string(),
        files: Vec::new(),
        summary: SyncSummary {
            synced: 0,
            modified: 0,
            local_only: 0,
            server_only: 0,
            conflict: 0,
        },
    })
}

/// Generate a simple unified diff between two strings.
fn generate_simple_diff(local: &str, server: &str, filename: &str) -> String {
    let local_lines: Vec<&str> = local.lines().collect();
    let server_lines: Vec<&str> = server.lines().collect();

    let mut result = format!("--- local/{}\n+++ server/{}\n", filename, filename);

    let max_len = local_lines.len().max(server_lines.len());
    for i in 0..max_len {
        let local_line = local_lines.get(i).unwrap_or(&"");
        let server_line = server_lines.get(i).unwrap_or(&"");

        if local_line == server_line {
            result.push_str(&format!(" {}\n", local_line));
        } else {
            if i < local_lines.len() {
                result.push_str(&format!("-{}\n", local_line));
            }
            if i < server_lines.len() {
                result.push_str(&format!("+{}\n", server_line));
            }
        }
    }

    result
}
