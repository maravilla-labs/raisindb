// SPDX-License-Identifier: BSL-1.1

//! `VectorRegenerate`: find stored embeddings whose dimensions no longer match
//! the tenant's embedding config and queue an `EmbeddingGenerate` job for each.
//!
//! The endpoint used to register a `Custom("EmbeddingRegeneration")` job and
//! run this scan in a detached task. The worker pool claimed that job too, had
//! no handler for a custom type and failed it, so the job an operator watched
//! reported failure (or flipped between states) while the scan ran unobserved.
//! Here the worker that owns the job runs the scan and its return value is the
//! job result.

use std::collections::HashMap;

use raisin_embeddings::storage::TenantEmbeddingConfigStore;
use raisin_embeddings::EmbeddingStorage;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobType};
use serde_json::{json, Value};

use crate::{RocksDBEmbeddingStorage, RocksDBStorage};

/// Metadata key: re-embed every node, not only those with mismatched dimensions.
pub const META_FORCE: &str = "force";

pub(super) async fn regenerate(
    storage: &RocksDBStorage,
    job: &JobInfo,
    context: &JobContext,
) -> Result<Value> {
    let (tenant, repo, branch) = (
        context.tenant_id.as_str(),
        context.repo_id.as_str(),
        context.branch.as_str(),
    );
    if repo.is_empty() || branch.is_empty() {
        return Err(Error::Validation(
            "vector regenerate needs a repository and branch".to_string(),
        ));
    }
    let force = context
        .metadata
        .get(META_FORCE)
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let expected_dims = match storage
        .tenant_embedding_config_repository()
        .get_config(tenant)
        .map_err(|e| Error::storage(format!("failed to read embedding config: {e}")))?
    {
        Some(config) if config.enabled => config.dimensions,
        Some(_) => {
            return Ok(
                json!({ "status": "disabled", "detail": "embeddings are disabled for this tenant" }),
            )
        }
        None => {
            return Ok(
                json!({ "status": "not_configured", "detail": "embeddings are not configured for this tenant" }),
            )
        }
    };

    let embeddings = RocksDBEmbeddingStorage::new(storage.db().clone());
    let registry = storage.job_registry();

    // Every workspace on the branch: the embedding job writes under the
    // workspace the node lives in, so each entry keeps its own.
    let mut entries = Vec::new();
    for workspace in embeddings.list_workspaces(tenant, repo, branch)? {
        for (node_id, revision) in embeddings.list_embeddings(tenant, repo, branch, &workspace)? {
            entries.push((workspace.clone(), node_id, revision));
        }
    }

    let total = entries.len();
    let (mut queued, mut skipped, mut errors) = (0usize, 0usize, 0usize);
    for (idx, (workspace, node_id, revision)) in entries.iter().enumerate() {
        match embeddings.get_embedding(tenant, repo, branch, workspace, node_id, Some(revision)) {
            Ok(Some(data)) if force || data.vector.len() != expected_dims => {
                // `force` travels with the job: the embedding handler skips a
                // node whose stored vector already matches the current spec.
                let mut metadata = HashMap::new();
                if force {
                    metadata.insert(
                        raisin_embeddings::FORCE_REEMBED_KEY.to_string(),
                        Value::Bool(true),
                    );
                }
                let embed_context = JobContext {
                    tenant_id: tenant.to_string(),
                    repo_id: repo.to_string(),
                    branch: branch.to_string(),
                    workspace_id: workspace.clone(),
                    revision: *revision,
                    metadata,
                };
                // Context first, so dispatch never sees the job without it.
                let embed_job = JobId::new();
                let queued_ok = storage
                    .job_data_store()
                    .put(&embed_job, &embed_context)
                    .is_ok()
                    && registry
                        .register_job_with_id(
                            embed_job,
                            JobType::EmbeddingGenerate {
                                node_id: node_id.clone(),
                            },
                            tenant.to_string(),
                            None,
                            None,
                            None,
                        )
                        .await
                        .is_ok();
                if queued_ok {
                    queued += 1;
                } else {
                    tracing::error!(node_id = %node_id, "failed to queue embedding regeneration");
                    errors += 1;
                }
            }
            Ok(Some(_)) => skipped += 1,
            Ok(None) => errors += 1,
            Err(e) => {
                tracing::error!(node_id = %node_id, error = %e, "failed to read embedding");
                errors += 1;
            }
        }
        if idx % 100 == 0 || idx + 1 == total {
            let _ = registry
                .update_progress(&job.id, (idx + 1) as f32 / total as f32)
                .await;
        }
    }

    Ok(json!({
        "expected_dimensions": expected_dims,
        "force": force,
        "checked": total,
        "queued": queued,
        "skipped": skipped,
        "errors": errors,
    }))
}
