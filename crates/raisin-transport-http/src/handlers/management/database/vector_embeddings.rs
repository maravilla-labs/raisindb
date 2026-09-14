// SPDX-License-Identifier: BSL-1.1

//! Vector embedding regeneration handler.
//!
//! Queues a `VectorRegenerate` job; `MaintenanceJobHandler` scans the tenant's
//! stored embeddings for dimension mismatches and queues an `EmbeddingGenerate`
//! job per node. The handler used to register a `Custom` job and run the scan
//! in a detached task, which the worker pool then failed for having no handler.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Regenerate embeddings for nodes with dimension mismatches.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/regenerate
///
/// `?force=true` re-embeds every node, not only mismatched ones. One
/// regeneration per tenant at a time.
#[cfg(feature = "storage-rocksdb")]
pub async fn regenerate_vector_embeddings(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    use raisin_storage::jobs::JobType;
    use raisin_storage::JobStatus;

    let rocksdb_storage = state.rocksdb_storage.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "RocksDB storage not initialized".to_string(),
            }),
        )
    })?;

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    let already_running = rocksdb_storage
        .job_registry()
        .list_jobs()
        .await
        .into_iter()
        .any(|job| {
            job.job_type == JobType::VectorRegenerate
                && job.tenant == tenant
                && matches!(
                    job.status,
                    JobStatus::Scheduled | JobStatus::Running | JobStatus::Executing
                )
        });
    if already_running {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: format!(
                    "Embedding regeneration already running for tenant '{}'. \
                     Please wait for the current operation to complete.",
                    tenant
                ),
            }),
        ));
    }

    tracing::info!(
        "Queueing embedding regeneration for {}/{}/{} (force: {})",
        tenant,
        repo,
        branch,
        params.force
    );

    let mut metadata = std::collections::HashMap::new();
    if params.force {
        metadata.insert(
            raisin_rocksdb::META_FORCE.to_string(),
            serde_json::Value::Bool(true),
        );
    }
    let job_id = super::vector::queue_vector_job(
        &state,
        JobType::VectorRegenerate,
        &tenant,
        &repo,
        &branch,
        metadata,
    )
    .await?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Embedding regeneration started for {}/{}/{}. \
             Jobs will be queued for nodes with dimension mismatches.",
            tenant, repo, branch
        ),
    }))
}
