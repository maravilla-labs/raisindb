// SPDX-License-Identifier: BSL-1.1

//! Function listing and detail handlers.
//!
//! Provides endpoints for listing all functions in a repository,
//! retrieving function details (with optional code), and browsing
//! execution history.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};

use crate::middleware::TenantInfo;
use crate::{error::ApiError, state::AppState};
use raisin_models::auth::AuthContext;

use super::helpers::{
    analyze_triggers, build_function_details, find_function_node, load_function_code,
    map_storage_error, property_as_bool, property_as_string,
};
use super::types::{
    ExecutionRecord, FunctionDetails, FunctionSummary, GetFunctionQuery, ListFunctionsQuery,
};
use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE};

#[cfg(feature = "storage-rocksdb")]
use super::helpers::job_status_label;
#[cfg(feature = "storage-rocksdb")]
use super::types::ListExecutionsQuery;
#[cfg(feature = "storage-rocksdb")]
use raisin_storage::jobs::{JobStatus, JobType};

/// List functions in a repository.
pub async fn list_functions(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Query(query): Query<ListFunctionsQuery>,
) -> Result<Json<Vec<FunctionSummary>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let node_svc = state.node_service_for_context(
        tenant_id,
        &repo,
        DEFAULT_BRANCH,
        FUNCTIONS_WORKSPACE,
        auth.map(|Extension(ctx)| ctx),
    );

    let nodes = node_svc
        .list_by_type("raisin:Function")
        .await
        .map_err(map_storage_error)?;

    let mut functions: Vec<FunctionSummary> = nodes
        .iter()
        .filter_map(|node| {
            let name = property_as_string(node.properties.get("name"))
                .unwrap_or_else(|| node.name.clone());
            let enabled = property_as_bool(node.properties.get("enabled")).unwrap_or(true);

            // Filter by enabled status
            if !query.include_disabled && !enabled {
                return None;
            }
            if let Some(filter_enabled) = query.enabled {
                if enabled != filter_enabled {
                    return None;
                }
            }

            let language = property_as_string(node.properties.get("language"))
                .unwrap_or_else(|| "javascript".into());

            // Filter by language
            if let Some(ref lang_filter) = query.language {
                if &language != lang_filter {
                    return None;
                }
            }

            let (has_http, has_event, has_schedule) =
                analyze_triggers(node.properties.get("triggers"));

            Some(FunctionSummary {
                path: node.path.clone(),
                name,
                title: property_as_string(node.properties.get("title"))
                    .unwrap_or_else(|| node.name.clone()),
                description: property_as_string(node.properties.get("description")),
                language,
                enabled,
                execution_mode: property_as_string(node.properties.get("execution_mode"))
                    .unwrap_or_else(|| "async".into()),
                has_http_trigger: has_http,
                has_event_triggers: has_event,
                has_schedule_triggers: has_schedule,
            })
        })
        .collect();

    // Apply pagination
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    functions = functions.into_iter().skip(offset).take(limit).collect();

    Ok(Json(functions))
}

/// Get function details.
pub async fn get_function(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(query): Query<GetFunctionQuery>,
) -> Result<Json<FunctionDetails>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_ctx = auth.map(|Extension(ctx)| ctx);
    let function_node =
        find_function_node(&state, tenant_id, &repo, &name, auth_ctx.as_ref()).await?;

    let code = if query.include_code {
        Some(load_function_code(&state, tenant_id, &repo, &function_node).await?)
    } else {
        None
    };

    let details = build_function_details(&function_node, code)?;
    Ok(Json(details))
}

/// List execution history for a function.
#[cfg(feature = "storage-rocksdb")]
pub async fn list_executions(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(query): Query<ListExecutionsQuery>,
) -> Result<Json<Vec<ExecutionRecord>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_ctx = auth.map(|Extension(ctx)| ctx);
    let function_node =
        find_function_node(&state, tenant_id, &repo, &name, auth_ctx.as_ref()).await?;

    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    // Merge persisted job history (survives restarts and in-memory registry
    // eviction, 24h retention) with the live registry (authoritative for
    // running jobs and freshly completed results).
    let jobs = super::helpers::merged_function_jobs(rocksdb, tenant_id).await?;
    let mut records = Vec::new();

    for job in jobs {
        if let JobType::FunctionExecution {
            function_path,
            trigger_name,
            execution_id,
        } = &job.job_type
        {
            if function_path != &function_node.path {
                continue;
            }

            // Two repositories can host a function at the same path; the job
            // itself only carries the path, so scope by the repo recorded in
            // the persisted job context. Jobs without a context (legacy) keep
            // the old path-only behavior.
            if !super::helpers::job_belongs_to_repo(rocksdb, tenant_id, &job.id, &repo) {
                continue;
            }

            if let Some(ref filter) = query.status {
                if job_status_label(&job.status) != filter.as_str() {
                    continue;
                }
            }

            if let Some(ref trig) = query.trigger_name {
                if trigger_name.as_deref() != Some(trig) {
                    continue;
                }
            }

            let (status, error) = match &job.status {
                JobStatus::Scheduled => ("scheduled".to_string(), None),
                JobStatus::Running | JobStatus::Executing => ("running".to_string(), None),
                JobStatus::Completed => ("completed".to_string(), None),
                JobStatus::Cancelled => ("cancelled".to_string(), None),
                JobStatus::Failed(e) => ("failed".to_string(), Some(e.clone())),
            };

            let duration_ms = job
                .completed_at
                .map(|c| (c - job.started_at).num_milliseconds() as u64);

            records.push(ExecutionRecord {
                execution_id: execution_id.clone(),
                function_path: function_path.clone(),
                trigger_name: trigger_name.clone(),
                status,
                started_at: job.started_at.to_rfc3339(),
                completed_at: job.completed_at.map(|t| t.to_rfc3339()),
                duration_ms,
                result: job.result.clone(),
                error,
            });
        }
    }

    // Newest first; HashMap iteration order would otherwise make pagination
    // non-deterministic.
    records.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    let paginated = records.into_iter().skip(offset).take(limit).collect();

    Ok(Json(paginated))
}

/// Stub execution listing without RocksDB.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn list_executions(
    State(_state): State<AppState>,
    Path((_repo, name)): Path<(String, String)>,
    Query(_query): Query<ListExecutionsQuery>,
) -> Result<Json<Vec<ExecutionRecord>>, ApiError> {
    Err(ApiError::internal(format!(
        "Function '{}' executions unavailable without RocksDB",
        name
    )))
}

/// Get execution details.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_execution(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, _name, execution_id)): Path<(String, String, String)>,
) -> Result<Json<ExecutionRecord>, ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let jobs = super::helpers::merged_function_jobs(rocksdb, &tenant_info.tenant_id).await?;
    for job in jobs {
        if let JobType::FunctionExecution {
            function_path,
            trigger_name,
            execution_id: exec_id,
        } = &job.job_type
        {
            if exec_id != &execution_id {
                continue;
            }

            // Don't serve an execution that belongs to a different repository
            // than the one in the URL (execution ids are tenant-wide).
            if !super::helpers::job_belongs_to_repo(rocksdb, &tenant_info.tenant_id, &job.id, &repo)
            {
                continue;
            }

            let (status, error) = match &job.status {
                JobStatus::Scheduled => ("scheduled".to_string(), None),
                JobStatus::Running | JobStatus::Executing => ("running".to_string(), None),
                JobStatus::Completed => ("completed".to_string(), None),
                JobStatus::Cancelled => ("cancelled".to_string(), None),
                JobStatus::Failed(e) => ("failed".to_string(), Some(e.clone())),
            };

            let duration_ms = job
                .completed_at
                .map(|c| (c - job.started_at).num_milliseconds() as u64);

            return Ok(Json(ExecutionRecord {
                execution_id: exec_id.clone(),
                function_path: function_path.clone(),
                trigger_name: trigger_name.clone(),
                status,
                started_at: job.started_at.to_rfc3339(),
                completed_at: job.completed_at.map(|t| t.to_rfc3339()),
                duration_ms,
                result: job.result.clone(),
                error,
            }));
        }
    }

    Err(ApiError::not_found(format!(
        "Execution '{}' not found",
        execution_id
    )))
}

/// Stub execution detail without RocksDB.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_execution(
    State(_state): State<AppState>,
    Path((_repo, _name, execution_id)): Path<(String, String, String)>,
) -> Result<Json<ExecutionRecord>, ApiError> {
    Err(ApiError::not_found(format!(
        "Execution '{}' not available without RocksDB",
        execution_id
    )))
}
