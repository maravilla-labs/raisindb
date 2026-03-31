// SPDX-License-Identifier: BSL-1.1

//! Vector embedding regeneration handler.
//!
//! Handles scanning existing embeddings for dimension mismatches and queuing
//! regeneration jobs via the unified job system. Implements API-level locking
//! to prevent concurrent regeneration operations.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use std::sync::Arc;

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

/// Regenerate embeddings for nodes with dimension mismatches.
///
/// POST /api/admin/management/database/:tenant/:repo/vector/regenerate
///
/// This operation:
/// 1. Scans embeddings in RocksDB for dimension mismatches
/// 2. Queues EmbeddingJobs to call the embedding provider API
/// 3. Reports progress via JobRegistry
/// 4. Implements API-level locking to prevent concurrent operations
#[cfg(feature = "storage-rocksdb")]
pub async fn regenerate_vector_embeddings(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    use raisin_storage::JobStatus;

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

    let embedding_storage = match &state.embedding_storage {
        Some(storage) => storage,
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Embedding storage not initialized".to_string(),
                }),
            ));
        }
    };

    let job_data_store = rocksdb_storage.job_data_store();
    let job_registry = rocksdb_storage.job_registry();

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    // API-LEVEL LOCKING: Check for existing running regeneration jobs
    tracing::info!("Checking for existing embedding regeneration jobs...");
    let existing_jobs = job_registry.list_jobs().await;

    for job in existing_jobs {
        if let raisin_storage::JobType::Custom(ref name) = job.job_type {
            if name == "EmbeddingRegeneration"
                && matches!(job.status, JobStatus::Running | JobStatus::Executing)
                && job.tenant.as_ref() == Some(&tenant)
            {
                tracing::warn!(
                    "Embedding regeneration already running for tenant '{}' (job: {})",
                    tenant,
                    job.id.0
                );
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
        }
    }

    tracing::info!(
        "Starting embedding regeneration for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    let job_id = job_registry
        .register_job(
            raisin_storage::JobType::Custom("EmbeddingRegeneration".to_string()),
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

    // Clone dependencies for async task
    let job_id_clone = job_id.clone();
    let emb_storage = Arc::clone(embedding_storage);
    let job_data_store = Arc::clone(job_data_store);
    let job_registry_clone = Arc::clone(job_registry);
    let config_repo = rocksdb_storage.tenant_embedding_config_repository();
    let tenant_clone = tenant.clone();
    let repo_clone = repo.clone();
    let branch_clone = branch.clone();
    let force = params.force;

    tokio::spawn(async move {
        run_embedding_regeneration(
            job_id_clone,
            emb_storage,
            job_data_store,
            job_registry_clone,
            config_repo,
            tenant_clone,
            repo_clone,
            branch_clone,
            force,
        )
        .await;
    });

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Embedding regeneration started for {}/{}/{}. \
             Jobs will be queued for nodes with dimension mismatches.",
            tenant, repo, branch
        ),
    }))
}

/// Background task that scans embeddings and queues regeneration jobs.
#[cfg(feature = "storage-rocksdb")]
async fn run_embedding_regeneration(
    job_id: raisin_storage::JobId,
    emb_storage: Arc<raisin_rocksdb::RocksDBEmbeddingStorage>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    config_repo: raisin_rocksdb::TenantEmbeddingConfigRepository,
    tenant: String,
    repo: String,
    branch: String,
    force: bool,
) {
    use raisin_embeddings::storage::TenantEmbeddingConfigStore;
    use raisin_embeddings::EmbeddingStorage;
    use raisin_storage::jobs::{JobContext, JobType};
    use std::collections::HashMap;

    let _ = job_registry.mark_running(&job_id).await;

    tracing::info!(
        "Regeneration task started for {}/{}/{}",
        tenant,
        repo,
        branch
    );

    // Get tenant config for expected dimensions
    let config_result = config_repo.get_config(&tenant);
    let expected_dims = match config_result {
        Ok(Some(config)) => {
            if !config.enabled {
                tracing::error!("Embeddings are disabled for tenant '{}'", tenant);
                let _ = job_registry
                    .mark_failed(
                        &job_id,
                        "Embeddings are disabled for this tenant".to_string(),
                    )
                    .await;
                return;
            }
            config.dimensions
        }
        Ok(None) => {
            tracing::error!("No embedding config found for tenant '{}'", tenant);
            let _ = job_registry
                .mark_failed(&job_id, "No embedding config found".to_string())
                .await;
            return;
        }
        Err(e) => {
            tracing::error!("Failed to get config: {}", e);
            let _ = job_registry
                .mark_failed(&job_id, format!("Failed to get config: {}", e))
                .await;
            return;
        }
    };

    tracing::info!("Expected dimensions: {}", expected_dims);

    // List all embeddings
    let embeddings_list = match emb_storage.list_embeddings(&tenant, &repo, &branch, "staff") {
        Ok(list) => list,
        Err(e) => {
            tracing::error!("Failed to list embeddings: {}", e);
            let _ = job_registry
                .mark_failed(&job_id, format!("Failed to list embeddings: {}", e))
                .await;
            return;
        }
    };

    let total_embeddings = embeddings_list.len();
    tracing::info!("Found {} embeddings to check", total_embeddings);

    if total_embeddings == 0 {
        tracing::info!("No embeddings found, nothing to regenerate");
        let result_json = serde_json::json!({
            "queued": 0,
            "skipped": 0,
            "errors": 0,
        });
        let _ = job_registry.set_result(&job_id, result_json).await;
        let _ = job_registry.mark_completed(&job_id).await;
        return;
    }

    let mut queued = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for (idx, (node_id, revision)) in embeddings_list.iter().enumerate() {
        match emb_storage.get_embedding(&tenant, &repo, &branch, "staff", node_id, Some(revision)) {
            Ok(Some(embedding_data)) => {
                if force || embedding_data.vector.len() != expected_dims {
                    if force && embedding_data.vector.len() == expected_dims {
                        tracing::info!(
                            "Force regeneration for {} (dimensions already match: {})",
                            node_id,
                            expected_dims
                        );
                    } else {
                        tracing::info!(
                            "Dimension mismatch for {}: expected {}, got {} - queuing job",
                            node_id,
                            expected_dims,
                            embedding_data.vector.len()
                        );
                    }

                    match job_registry
                        .register_job(
                            JobType::EmbeddingGenerate {
                                node_id: node_id.clone(),
                            },
                            Some(tenant.clone()),
                            None,
                            None,
                            None,
                        )
                        .await
                    {
                        Ok(embedding_job_id) => {
                            let context = JobContext {
                                tenant_id: tenant.clone(),
                                repo_id: repo.clone(),
                                branch: branch.clone(),
                                workspace_id: "staff".to_string(),
                                revision: *revision,
                                metadata: HashMap::new(),
                            };

                            if let Err(e) = job_data_store.put(&embedding_job_id, &context) {
                                tracing::error!(
                                    "Failed to store context for job {}: {}",
                                    embedding_job_id,
                                    e
                                );
                                errors += 1;
                            } else {
                                tracing::debug!(
                                    "Queued embedding job {} for node {} (revision {})",
                                    embedding_job_id,
                                    node_id,
                                    revision
                                );
                                queued += 1;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to register job for {}: {}", node_id, e);
                            errors += 1;
                        }
                    }
                } else {
                    skipped += 1;
                }
            }
            Ok(None) => {
                tracing::warn!(
                    "Embedding not found for node {}, revision {}",
                    node_id,
                    revision
                );
                errors += 1;
            }
            Err(e) => {
                tracing::error!("Failed to fetch embedding for {}: {}", node_id, e);
                errors += 1;
            }
        }

        // Report progress every 10 items or on last item
        if idx % 10 == 0 || idx == total_embeddings - 1 {
            let progress = (idx as f32 + 1.0) / total_embeddings as f32;
            let _ = job_registry.update_progress(&job_id, progress).await;

            tracing::debug!(
                "Regeneration progress: {}/{} ({:.1}%) - {} queued, {} skipped, {} errors",
                idx + 1,
                total_embeddings,
                progress * 100.0,
                queued,
                skipped,
                errors
            );
        }
    }

    tracing::info!(
        "Embedding regeneration scan completed: {} jobs queued, {} skipped, {} errors",
        queued,
        skipped,
        errors
    );

    let result_json = serde_json::json!({
        "queued": queued,
        "skipped": skipped,
        "errors": errors,
    });

    let _ = job_registry.set_result(&job_id, result_json).await;
    let _ = job_registry.mark_completed(&job_id).await;
}
