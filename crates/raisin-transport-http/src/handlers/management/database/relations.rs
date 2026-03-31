// SPDX-License-Identifier: BSL-1.1

//! Relation index integrity handlers.
//!
//! Endpoints for verifying and repairing orphaned relations in the global
//! relation index. Verification reports statistics without changes, while
//! repair writes tombstones for relations pointing to deleted nodes.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};

use raisin_storage::jobs::JobType;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Verify relation index integrity.
///
/// POST /api/admin/management/database/:tenant/:repo/relations/verify
///
/// Scans the global relation index for orphaned relations (relations pointing
/// to deleted/tombstoned nodes) and reports statistics without making changes.
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_relation_integrity(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    let rocksdb_storage = match &state.rocksdb_storage {
        Some(storage) => storage,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "RocksDB storage not initialized".to_string(),
                }),
            ));
        }
    };

    let job_registry = rocksdb_storage.job_registry();
    let job_data_store = rocksdb_storage.job_data_store();
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting relation integrity verification for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(
            JobType::RelationConsistencyCheck { repair: false },
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    // Store job context
    use raisin_storage::jobs::JobContext;
    use std::collections::HashMap;

    let context = JobContext {
        tenant_id: tenant.clone(),
        repo_id: repo.clone(),
        branch: branch.clone(),
        workspace_id: "".to_string(), // Not workspace-specific
        revision: raisin_hlc::HLC::new(0, 0),
        metadata: HashMap::new(),
    };

    job_data_store.put(&job_id, &context).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to store job context: {}", e),
            }),
        )
    })?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Relation integrity verification started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}

/// Repair relation index integrity.
///
/// POST /api/admin/management/database/:tenant/:repo/relations/repair
///
/// Scans the global relation index for orphaned relations and writes tombstones
/// for any relations pointing to deleted/tombstoned nodes.
#[cfg(feature = "storage-rocksdb")]
pub async fn repair_relation_integrity(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    use raisin_storage::BranchRepository;

    let rocksdb_storage = match &state.rocksdb_storage {
        Some(storage) => storage,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "RocksDB storage not initialized".to_string(),
                }),
            ));
        }
    };

    let job_registry = rocksdb_storage.job_registry();
    let job_data_store = rocksdb_storage.job_data_store();
    let branch_repo = rocksdb_storage.branches_impl();
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::info!(
        "Starting relation integrity repair for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    // Get current HEAD revision for tombstone writes
    let revision = branch_repo
        .get_head(&tenant, &repo, &branch)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to get HEAD revision: {}", e),
                }),
            )
        })?;

    let job_id = job_registry
        .register_job(
            JobType::RelationConsistencyCheck { repair: true },
            Some(tenant.clone()),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to register job: {}", e),
                }),
            )
        })?;

    // Store job context with revision for tombstone writes
    use raisin_storage::jobs::JobContext;
    use std::collections::HashMap;

    let context = JobContext {
        tenant_id: tenant.clone(),
        repo_id: repo.clone(),
        branch: branch.clone(),
        workspace_id: "".to_string(), // Not workspace-specific
        revision,
        metadata: HashMap::new(),
    };

    job_data_store.put(&job_id, &context).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to store job context: {}", e),
            }),
        )
    })?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Relation integrity repair started for {}/{}/{}",
            tenant, repo, branch
        ),
    }))
}
