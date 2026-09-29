// SPDX-License-Identifier: BSL-1.1

//! `VectorRegenerate`: queue an `EmbeddingGenerate` job for every node on the
//! branch whose embedding is wrong or missing:
//! - a stored embedding whose dimensions no longer match the tenant's
//!   embedding config (or every stored one, with `force`);
//! - an embedding-eligible node with no stored embedding at all — typically
//!   one whose job died at max retries while the embedder was down. Scanning
//!   only the embeddings column family never saw those, and nothing else
//!   would ever queue them again short of an edit to the node.
//!
//! Eligibility is the node-event trigger's own rule
//! ([`index_settings_for`]), so regenerate queues exactly the nodes an edit
//! would.
//!
//! The endpoint used to register a `Custom("EmbeddingRegeneration")` job and
//! run this scan in a detached task. The worker pool claimed that job too, had
//! no handler for a custom type and failed it, so the job an operator watched
//! reported failure (or flipped between states) while the scan ran unobserved.
//! Here the worker that owns the job runs the scan and its return value is the
//! job result.

use std::collections::{HashMap, HashSet};

use raisin_embeddings::storage::TenantEmbeddingConfigStore;
use raisin_embeddings::EmbeddingStorage;
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::types::NodeType;
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobType};
use raisin_storage::{
    BranchRepository, BranchScope, ListOptions, NodeRepository, NodeTypeRepository, Storage,
    StorageScope,
};
use serde_json::{json, Value};

use crate::jobs::event_handler::index_helpers::index_settings_for;
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
        for (node_id, revision, dims) in
            embeddings.list_embedding_dimensions(tenant, repo, branch, &workspace)?
        {
            entries.push((workspace.clone(), node_id, revision, dims));
        }
    }

    // Nodes with no stored embedding at all, and eligible for one.
    let embedded: HashSet<(&str, &str)> = entries
        .iter()
        .map(|(workspace, node_id, _, _)| (workspace.as_str(), node_id.as_str()))
        .collect();
    let missing = missing_embeddings(storage, tenant, repo, branch, &embedded).await?;

    let total = entries.len() + missing.len();
    let (mut queued, mut skipped, mut errors) = (0usize, 0usize, 0usize);
    let report_progress = |done: usize| async move {
        if done % 100 == 0 || done == total {
            let _ = registry
                .update_progress(&job.id, done as f32 / total as f32)
                .await;
        }
    };

    for (idx, (workspace, node_id, revision, dims)) in entries.iter().enumerate() {
        if !force && *dims == expected_dims {
            skipped += 1;
        } else {
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
        report_progress(idx + 1).await;
    }

    // A node with no embedding is queued the way a node event queues it — at
    // the branch head and under the same dedup key — so an embedding job the
    // node already has pending absorbs this one instead of running twice.
    let (mut missing_queued, mut missing_pending) = (0usize, 0usize);
    for (idx, (workspace, node_id, revision)) in missing.iter().enumerate() {
        let job_type = JobType::EmbeddingGenerate {
            node_id: node_id.clone(),
        };
        let dedup_key = format!(
            "{tenant}:{repo}:{branch}:{workspace}:{}:{revision}",
            job_type.dedup_key()
        );
        let embed_context = JobContext {
            tenant_id: tenant.to_string(),
            repo_id: repo.to_string(),
            branch: branch.to_string(),
            workspace_id: workspace.clone(),
            revision: *revision,
            metadata: HashMap::new(),
        };
        let embed_job = JobId::new();
        let registered = match storage.job_data_store().put(&embed_job, &embed_context) {
            Ok(()) => {
                registry
                    .register_job_with_id_idempotent(
                        embed_job.clone(),
                        job_type,
                        tenant.to_string(),
                        dedup_key,
                        None,
                    )
                    .await
            }
            Err(e) => Err(e),
        };
        match registered {
            Ok(true) => missing_queued += 1,
            Ok(false) => {
                let _ = storage.job_data_store().delete(tenant, &embed_job);
                missing_pending += 1;
            }
            Err(e) => {
                tracing::error!(node_id = %node_id, error = %e, "failed to queue embedding for a node without one");
                errors += 1;
            }
        }
        report_progress(entries.len() + idx + 1).await;
    }

    Ok(json!({
        "expected_dimensions": expected_dims,
        "force": force,
        "checked": entries.len(),
        "queued": queued + missing_queued,
        "skipped": skipped,
        "missing": missing.len(),
        "missing_queued": missing_queued,
        "missing_already_pending": missing_pending,
        "errors": errors,
    }))
}

/// Every node on the branch head that is eligible for an embedding and has
/// none stored, as `(workspace, node_id, branch head revision)`.
async fn missing_embeddings(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    branch: &str,
    embedded: &HashSet<(&str, &str)>,
) -> Result<Vec<(String, String, HLC)>> {
    let head = storage
        .branches()
        .get_branch(tenant, repo, branch)
        .await?
        .ok_or_else(|| Error::NotFound(format!("branch '{branch}' not found")))?
        .head;

    let mut node_types: HashMap<String, Option<NodeType>> = HashMap::new();
    let mut missing = Vec::new();
    for workspace in crate::management::list_workspaces(storage, tenant, repo).await? {
        let nodes = storage
            .nodes()
            .list_all(
                StorageScope::new(tenant, repo, branch, &workspace),
                ListOptions::default(),
            )
            .await?;
        for node in nodes {
            if embedded.contains(&(workspace.as_str(), node.id.as_str())) {
                continue;
            }
            if !node_types.contains_key(&node.node_type) {
                let def = storage
                    .node_types()
                    .get(
                        BranchScope::new(tenant, repo, branch),
                        &node.node_type,
                        None,
                    )
                    .await
                    .unwrap_or(None);
                node_types.insert(node.node_type.clone(), def);
            }
            let def = node_types.get(&node.node_type).and_then(Option::as_ref);
            if index_settings_for(&workspace, &node.node_type, def).vector {
                missing.push((workspace.clone(), node.id, head));
            }
        }
    }
    Ok(missing)
}
