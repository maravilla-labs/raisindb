// SPDX-License-Identifier: BSL-1.1

//! RocksDB index rebuild handler.
//!
//! Provides the endpoint for rebuilding property, reference, and child_order
//! indexes stored in RocksDB column families.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use std::sync::Arc;

use raisin_storage::jobs::JobType;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse, ReindexRequest};

/// Rebuild RocksDB indexes (property, reference, child_order).
///
/// POST /api/admin/management/database/:tenant/:repo/reindex/start
#[cfg(feature = "storage-rocksdb")]
pub async fn reindex_start(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
    Json(req): Json<ReindexRequest>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    use raisin_rocksdb::management::async_indexing;
    use raisin_storage::IndexType;

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
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    // Validate and parse index types
    let mut index_types = Vec::new();
    for index_type_str in &req.index_types {
        match index_type_str.as_str() {
            "property" => index_types.push(IndexType::Property),
            "reference" => index_types.push(IndexType::Reference),
            "child_order" => index_types.push(IndexType::ChildOrder),
            "all" => {
                index_types.clear();
                index_types.push(IndexType::All);
                break;
            }
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!(
                            "Invalid index type: '{}'. Valid types: 'all', 'property', 'reference', 'child_order'",
                            index_type_str
                        ),
                    }),
                ));
            }
        }
    }

    if index_types.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "At least one index type must be specified".to_string(),
            }),
        ));
    }

    tracing::info!(
        "Starting RocksDB index rebuild for {}/{}/{}/{} - types: {:?}",
        tenant,
        repo,
        branch,
        req.workspace,
        index_types
    );

    let job_id = job_registry
        .register_job(
            JobType::IndexRebuild,
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

    let storage = Arc::clone(rocksdb_storage);
    let job_registry_clone = Arc::clone(job_registry);
    let job_id_clone = job_id.clone();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    let workspace_clone = req.workspace.clone();

    tokio::spawn(async move {
        run_reindex(
            storage,
            job_registry_clone,
            job_id_clone,
            tenant_clone,
            repo_clone,
            branch_clone,
            workspace_clone,
            index_types,
        )
        .await;
    });

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "RocksDB reindex started for {}/{}/{}/{} - types: {:?}",
            tenant, repo, branch, req.workspace, req.index_types
        ),
    }))
}

/// Background task that processes each index type sequentially.
#[cfg(feature = "storage-rocksdb")]
async fn run_reindex(
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_id: raisin_storage::JobId,
    tenant: String,
    repo: String,
    branch: String,
    workspace: String,
    index_types: Vec<raisin_storage::IndexType>,
) {
    use raisin_rocksdb::management::async_indexing;
    use raisin_storage::IndexType;

    let _ = job_registry.mark_running(&job_id).await;

    let total_types = index_types.len();
    let mut cumulative_stats = raisin_storage::RebuildStats {
        index_type: IndexType::All,
        items_processed: 0,
        errors: 0,
        duration_ms: 0,
        success: true,
    };

    for (idx, index_type) in index_types.iter().enumerate() {
        tracing::info!(
            "Rebuilding {:?} indexes ({}/{}) for {}/{}/{}/{}",
            index_type,
            idx + 1,
            total_types,
            tenant,
            repo,
            branch,
            workspace
        );

        let base_progress = idx as f32 / total_types as f32;
        let _ = job_registry.update_progress(&job_id, base_progress).await;

        match async_indexing::rebuild_indexes(
            &storage,
            &tenant,
            &repo,
            &branch,
            &workspace,
            *index_type,
        )
        .await
        {
            Ok(stats) => {
                cumulative_stats.items_processed += stats.items_processed;
                cumulative_stats.errors += stats.errors;
                cumulative_stats.duration_ms += stats.duration_ms;
                cumulative_stats.success = cumulative_stats.success && stats.success;

                tracing::info!(
                    "Completed {:?} rebuild: {} items, {} errors in {}ms",
                    index_type,
                    stats.items_processed,
                    stats.errors,
                    stats.duration_ms
                );
            }
            Err(e) => {
                cumulative_stats.success = false;
                cumulative_stats.errors += 1;

                tracing::error!("Failed to rebuild {:?} indexes: {}", index_type, e);

                let _ = job_registry
                    .mark_failed(
                        &job_id,
                        format!("Failed to rebuild {:?} indexes: {}", index_type, e),
                    )
                    .await;
                return;
            }
        }
    }

    let result_json = serde_json::to_value(&cumulative_stats).unwrap_or_default();
    let _ = job_registry.set_result(&job_id, result_json).await;
    let _ = job_registry.mark_completed(&job_id).await;

    tracing::info!(
        "RocksDB reindex completed for {}/{}/{}/{}: {} total items, {} errors in {}ms",
        tenant,
        repo,
        branch,
        workspace,
        cumulative_stats.items_processed,
        cumulative_stats.errors,
        cumulative_stats.duration_ms
    );
}
