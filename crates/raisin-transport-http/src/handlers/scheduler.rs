// SPDX-License-Identifier: BSL-1.1

//! One-shot scheduled invocation handlers.
//!
//! Repo-scoped HTTP API for scheduling a single future function or flow run:
//!
//! - `POST   /api/scheduler/{repo}/invocations` — schedule an invocation
//! - `GET    /api/scheduler/{repo}/invocations` — list (filter by
//!   `external_key` / `status` query params)
//! - `GET    /api/scheduler/{repo}/invocations/{job_id}` — inspect one
//! - `DELETE /api/scheduler/{repo}/invocations/{job_id}` — cancel before fire
//!
//! The target node is resolved with the caller's auth context before
//! scheduling, so row-level security gates which functions/flows a caller
//! can schedule against.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::Deserialize;

use crate::middleware::TenantInfo;
use crate::{error::ApiError, state::AppState};

const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";

/// Request body for scheduling a one-shot invocation.
#[derive(Debug, Deserialize)]
pub struct CreateInvocationRequest {
    /// "function" or "flow"
    pub target_kind: String,
    /// Path of the target function or flow node
    pub target_path: String,
    /// Input passed to the target when it fires
    #[serde(default)]
    pub input: serde_json::Value,
    /// When to fire (RFC3339); a past time dispatches immediately
    pub run_at: String,
    /// Optional caller-supplied idempotency/lookup key
    #[serde(default)]
    pub external_key: Option<String>,
    /// Branch the invocation executes against (defaults to "main")
    #[serde(default)]
    pub branch: Option<String>,
    /// Workspace the invocation executes against (defaults to "functions")
    #[serde(default)]
    pub workspace: Option<String>,
    /// Retry attempts if the invocation fails (defaults to 0 — one-shot)
    #[serde(default)]
    pub max_retries: Option<u32>,
}

/// Query filters for listing invocations.
#[derive(Debug, Deserialize, Default)]
pub struct ListInvocationsQuery {
    #[serde(default)]
    pub external_key: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

// ============================================================================
// RocksDB-backed implementation
// ============================================================================

#[cfg(feature = "storage-rocksdb")]
mod inner {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use raisin_models::auth::AuthContext;
    use raisin_rocksdb::{
        META_ACTOR, META_EXTERNAL_KEY, META_INPUT, META_SCHEDULED_FOR, META_TARGET_PATH,
    };
    use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};

    pub(super) fn rocksdb_handle(
        state: &AppState,
    ) -> Result<Arc<raisin_rocksdb::RocksDBStorage>, ApiError> {
        state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| ApiError::internal("RocksDB storage not available"))
            .cloned()
    }

    pub(super) fn job_status_to_string(status: &JobStatus) -> &'static str {
        match status {
            JobStatus::Scheduled => "scheduled",
            JobStatus::Running | JobStatus::Executing => "running",
            JobStatus::Completed => "completed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed(_) => "failed",
        }
    }

    /// Render a scheduled invocation (job + context) as its wire JSON shape.
    pub(super) fn invocation_json(info: &JobInfo, context: &JobContext) -> serde_json::Value {
        let (invocation_id, target_kind) = match &info.job_type {
            JobType::ScheduledInvocation {
                invocation_id,
                target_kind,
            } => (invocation_id.clone(), target_kind.clone()),
            _ => (String::new(), String::new()),
        };
        serde_json::json!({
            "job_id": info.id.to_string(),
            "invocation_id": invocation_id,
            "target_kind": target_kind,
            "target_path": context.metadata.get(META_TARGET_PATH),
            "input": context.metadata.get(META_INPUT),
            "actor": context.metadata.get(META_ACTOR),
            "external_key": context.metadata.get(META_EXTERNAL_KEY),
            "run_at": context.metadata.get(META_SCHEDULED_FOR),
            "status": job_status_to_string(&info.status),
            "error": info.error,
            "result": info.result,
        })
    }

    /// Collect this tenant + repository's scheduled invocations.
    pub(super) async fn list_repo_invocations(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        tenant_id: &str,
        repo: &str,
    ) -> Vec<(JobInfo, JobContext)> {
        let jobs = rocksdb.job_registry().list_jobs_by_tenant(tenant_id).await;
        let mut out = Vec::new();
        for job in jobs {
            if !matches!(job.job_type, JobType::ScheduledInvocation { .. }) {
                continue;
            }
            let Ok(Some(context)) = rocksdb.job_data_store().get(tenant_id, &job.id) else {
                continue;
            };
            if context.repo_id != repo {
                continue;
            }
            out.push((job, context));
        }
        out
    }

    /// Resolve a scheduled invocation by job id, verifying it belongs to
    /// this tenant + repository (prevents cross-repo cancellation).
    pub(super) async fn resolve_invocation(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        tenant_id: &str,
        repo: &str,
        job_id: &str,
    ) -> Result<(JobInfo, JobContext), ApiError> {
        let id = JobId::from_string(job_id.to_string());
        let info = rocksdb
            .job_registry()
            .get_job_info(&id)
            .await
            .map_err(|_| {
                ApiError::not_found(format!("Scheduled invocation '{}' not found", job_id))
            })?;
        if !matches!(info.job_type, JobType::ScheduledInvocation { .. }) {
            return Err(ApiError::not_found(format!(
                "Job '{}' is not a scheduled invocation",
                job_id
            )));
        }
        let context = rocksdb
            .job_data_store()
            .get(tenant_id, &id)
            .map_err(|e| ApiError::internal(e.to_string()))?
            .ok_or_else(|| {
                ApiError::not_found(format!("Scheduled invocation '{}' not found", job_id))
            })?;
        if context.tenant_id != tenant_id || context.repo_id != repo {
            return Err(ApiError::not_found(format!(
                "Scheduled invocation '{}' not found in repository '{}'",
                job_id, repo
            )));
        }
        Ok((info, context))
    }

    /// Resolve the target node with the caller's auth context so RLS gates
    /// which functions/flows can be scheduled against.
    pub(super) async fn resolve_target(
        state: &AppState,
        tenant_id: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        target_kind: &str,
        target_path: &str,
        auth_context: Option<&AuthContext>,
    ) -> Result<(), ApiError> {
        let path = if target_path.starts_with('/') {
            target_path.to_string()
        } else {
            format!("/{}", target_path)
        };
        let node_svc = state.node_service_for_context(
            tenant_id,
            repo,
            branch,
            workspace,
            auth_context.cloned(),
        );
        node_svc
            .get_by_path(&path)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "Scheduled invocation target {} '{}' not found",
                    target_kind, target_path
                ))
            })?;
        Ok(())
    }

    /// Register the delayed `ScheduledInvocation` job (context first).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn register_invocation(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        tenant_id: &str,
        repo: &str,
        req: &CreateInvocationRequest,
        run_at: chrono::DateTime<chrono::Utc>,
        actor: &str,
    ) -> Result<(JobId, String), ApiError> {
        let invocation_id = nanoid::nanoid!();
        let job_type = JobType::ScheduledInvocation {
            invocation_id: invocation_id.clone(),
            target_kind: req.target_kind.clone(),
        };

        let mut metadata = HashMap::new();
        metadata.insert(
            META_TARGET_PATH.to_string(),
            serde_json::json!(req.target_path),
        );
        metadata.insert(META_INPUT.to_string(), req.input.clone());
        metadata.insert(META_ACTOR.to_string(), serde_json::json!(actor));
        if let Some(key) = &req.external_key {
            metadata.insert(META_EXTERNAL_KEY.to_string(), serde_json::json!(key));
        }
        metadata.insert(
            META_SCHEDULED_FOR.to_string(),
            serde_json::json!(run_at.to_rfc3339()),
        );

        let context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo.to_string(),
            branch: req
                .branch
                .clone()
                .unwrap_or_else(|| DEFAULT_BRANCH.to_string()),
            workspace_id: req
                .workspace
                .clone()
                .unwrap_or_else(|| FUNCTIONS_WORKSPACE.to_string()),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = JobId::new();
        rocksdb
            .job_data_store()
            .put(&job_id, &context)
            .map_err(|e| ApiError::internal(e.to_string()))?;

        // max_retries defaults to 0: a failed one-shot must not silently
        // re-fire unless the caller opts into retries.
        rocksdb
            .job_registry()
            .register_job_at_with_id(
                job_id.clone(),
                job_type,
                tenant_id.to_string(),
                run_at,
                Some(req.max_retries.unwrap_or(0)),
            )
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

        Ok((job_id, invocation_id))
    }
}

/// Schedule a one-shot invocation of a function or flow.
#[cfg(feature = "storage-rocksdb")]
pub async fn create_invocation(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(repo): Path<String>,
    auth: Option<Extension<raisin_models::auth::AuthContext>>,
    Json(req): Json<CreateInvocationRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rocksdb = inner::rocksdb_handle(&state)?;
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_context = auth.map(|Extension(ctx)| ctx);

    if req.target_kind != "function" && req.target_kind != "flow" {
        return Err(ApiError::validation_failed(format!(
            "Invalid target_kind '{}' (expected 'function' or 'flow')",
            req.target_kind
        )));
    }

    let run_at = chrono::DateTime::parse_from_rfc3339(&req.run_at)
        .map_err(|e| {
            ApiError::validation_failed(format!(
                "Invalid 'run_at' timestamp '{}': {} (expected RFC3339)",
                req.run_at, e
            ))
        })?
        .with_timezone(&chrono::Utc);

    // Resolve the target with the caller's auth so RLS gates visibility.
    let branch = req.branch.as_deref().unwrap_or(DEFAULT_BRANCH);
    let workspace = req.workspace.as_deref().unwrap_or(FUNCTIONS_WORKSPACE);
    inner::resolve_target(
        &state,
        tenant_id,
        &repo,
        branch,
        workspace,
        &req.target_kind,
        &req.target_path,
        auth_context.as_ref(),
    )
    .await?;

    let actor = auth_context
        .as_ref()
        .and_then(|a| a.user_id.clone())
        .unwrap_or_else(|| "http_api".to_string());

    let (job_id, invocation_id) =
        inner::register_invocation(&rocksdb, tenant_id, &repo, &req, run_at, &actor).await?;

    tracing::info!(
        job_id = %job_id,
        invocation_id = %invocation_id,
        target_kind = %req.target_kind,
        target_path = %req.target_path,
        run_at = %run_at,
        "Scheduled one-shot invocation via HTTP"
    );

    Ok(Json(serde_json::json!({
        "job_id": job_id.to_string(),
        "invocation_id": invocation_id,
        "status": "scheduled",
        "run_at": run_at.to_rfc3339(),
    })))
}

/// List scheduled invocations in a repository.
#[cfg(feature = "storage-rocksdb")]
pub async fn list_invocations(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(repo): Path<String>,
    Query(query): Query<ListInvocationsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use raisin_rocksdb::META_EXTERNAL_KEY;

    let rocksdb = inner::rocksdb_handle(&state)?;
    let tenant_id = tenant_info.tenant_id.as_str();

    let invocations = inner::list_repo_invocations(&rocksdb, tenant_id, &repo).await;
    let items: Vec<serde_json::Value> = invocations
        .iter()
        .filter(|(info, ctx)| {
            if let Some(key) = &query.external_key {
                if ctx.metadata.get(META_EXTERNAL_KEY).and_then(|v| v.as_str())
                    != Some(key.as_str())
                {
                    return false;
                }
            }
            if let Some(status) = &query.status {
                if inner::job_status_to_string(&info.status) != status {
                    return false;
                }
            }
            true
        })
        .map(|(info, ctx)| inner::invocation_json(info, ctx))
        .collect();

    Ok(Json(serde_json::json!({ "invocations": items })))
}

/// Get a single scheduled invocation.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_invocation(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rocksdb = inner::rocksdb_handle(&state)?;
    let tenant_id = tenant_info.tenant_id.as_str();

    let (info, context) = inner::resolve_invocation(&rocksdb, tenant_id, &repo, &job_id).await?;
    Ok(Json(inner::invocation_json(&info, &context)))
}

/// Cancel a scheduled invocation before it fires.
#[cfg(feature = "storage-rocksdb")]
pub async fn cancel_invocation(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rocksdb = inner::rocksdb_handle(&state)?;
    let tenant_id = tenant_info.tenant_id.as_str();

    let (info, _context) = inner::resolve_invocation(&rocksdb, tenant_id, &repo, &job_id).await?;
    rocksdb
        .job_registry()
        .cancel_job(&info.id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "job_id": info.id.to_string(),
        "status": "cancelled",
    })))
}

// ============================================================================
// Stubs when RocksDB is not available
// ============================================================================

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn create_invocation(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<TenantInfo>,
    Path(_repo): Path<String>,
    Json(_req): Json<CreateInvocationRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::internal(
        "Scheduled invocations require RocksDB backend",
    ))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn list_invocations(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<TenantInfo>,
    Path(_repo): Path<String>,
    Query(_query): Query<ListInvocationsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::internal(
        "Scheduled invocations require RocksDB backend",
    ))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_invocation(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<TenantInfo>,
    Path((_repo, _job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::internal(
        "Scheduled invocations require RocksDB backend",
    ))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn cancel_invocation(
    State(_state): State<AppState>,
    Extension(_tenant_info): Extension<TenantInfo>,
    Path((_repo, _job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::internal(
        "Scheduled invocations require RocksDB backend",
    ))
}
