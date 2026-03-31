// SPDX-License-Identifier: BSL-1.1

//! Flow execution handlers: run, test, resume, status, cancel, and delete.
//!
//! These handlers are thin HTTP adapters that delegate to the shared
//! [`raisin_flow_runtime::service`] module for transport-agnostic logic.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_storage::{DeleteNodeOptions, NodeRepository, Storage, StorageScope};

use crate::{error::ApiError, state::AppState};

use super::helpers::map_storage_error;
use super::types::{
    CancelFlowInstanceResponse, FlowInstanceStatusResponse, ResumeFlowRequest, RunFlowRequest,
    RunFlowResponse, RunFlowTestRequest,
};
use super::{DEFAULT_BRANCH, SYSTEM_WORKSPACE, TENANT_ID};

// ============================================================================
// Shared helper
// ============================================================================

/// Get a `&dyn FlowJobScheduler` from the app state.
#[cfg(feature = "storage-rocksdb")]
fn get_scheduler(
    state: &AppState,
) -> Result<&dyn raisin_flow_runtime::service::FlowJobScheduler, ApiError> {
    raisin_rocksdb::get_flow_job_scheduler(&state.rocksdb_storage)
        .map_err(|e| ApiError::internal(e.to_string()))
}

// ============================================================================
// Run Flow
// ============================================================================

/// Execute a flow by path.
///
/// Creates a `FlowInstance` and queues a `FlowInstanceExecution` job.
/// Returns the instance ID and job ID for tracking.
#[cfg(feature = "storage-rocksdb")]
pub async fn run_flow(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    auth_context: Option<Extension<AuthContext>>,
    Json(req): Json<RunFlowRequest>,
) -> Result<Json<RunFlowResponse>, ApiError> {
    let scheduler = get_scheduler(&state)?;

    let actor = auth_context
        .as_ref()
        .and_then(|ext| ext.user_id.clone())
        .unwrap_or_else(|| "http_api".to_string());

    let actor_home = auth_context.as_ref().and_then(|ext| ext.home.clone());

    let result = raisin_flow_runtime::service::run_flow(
        state.storage.as_ref(),
        scheduler,
        &repo,
        &req.flow_path,
        req.input.clone(),
        actor,
        actor_home,
    )
    .await?;

    Ok(Json(RunFlowResponse {
        instance_id: result.instance_id,
        job_id: result.job_id,
        status: "queued".to_string(),
    }))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn run_flow(
    State(_state): State<AppState>,
    Path(_repo): Path<String>,
    Json(_req): Json<RunFlowRequest>,
) -> Result<Json<RunFlowResponse>, ApiError> {
    Err(ApiError::internal(
        "Flow execution requires RocksDB backend",
    ))
}

// ============================================================================
// Test Flow Execution
// ============================================================================

/// Execute a flow in test mode.
///
/// Creates a `FlowInstance` with test configuration and queues execution.
/// Test runs can mock function responses and run in an isolated branch.
#[cfg(feature = "storage-rocksdb")]
pub async fn run_flow_test(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Json(req): Json<RunFlowTestRequest>,
) -> Result<Json<RunFlowResponse>, ApiError> {
    let scheduler = get_scheduler(&state)?;

    let result = raisin_flow_runtime::service::run_flow_test(
        state.storage.as_ref(),
        scheduler,
        &repo,
        &req.flow_path,
        req.input.clone(),
        req.test_config,
    )
    .await?;

    Ok(Json(RunFlowResponse {
        instance_id: result.instance_id,
        job_id: result.job_id,
        status: "queued".to_string(),
    }))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn run_flow_test(
    State(_state): State<AppState>,
    Path(_repo): Path<String>,
    Json(_req): Json<RunFlowTestRequest>,
) -> Result<Json<RunFlowResponse>, ApiError> {
    Err(ApiError::internal(
        "Test flow execution requires RocksDB backend",
    ))
}

// ============================================================================
// Resume Flow Instance
// ============================================================================

/// Resume a paused flow instance.
///
/// Verifies the instance exists and is in `Waiting` state, then queues a
/// `FlowInstanceExecution` job with `execution_type: "resume"` and the
/// caller-supplied `resume_data`.
#[cfg(feature = "storage-rocksdb")]
pub async fn resume_flow(
    State(state): State<AppState>,
    Path((repo, instance_id)): Path<(String, String)>,
    _auth_context: Option<Extension<AuthContext>>,
    Json(req): Json<ResumeFlowRequest>,
) -> Result<Json<RunFlowResponse>, ApiError> {
    let scheduler = get_scheduler(&state)?;

    let result = raisin_flow_runtime::service::resume_flow(
        state.storage.as_ref(),
        scheduler,
        &repo,
        &instance_id,
        req.resume_data,
    )
    .await?;

    Ok(Json(RunFlowResponse {
        instance_id: result.instance_id,
        job_id: result.job_id,
        status: "resumed".to_string(),
    }))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn resume_flow(
    State(_state): State<AppState>,
    Path((_repo, _instance_id)): Path<(String, String)>,
    Json(_req): Json<ResumeFlowRequest>,
) -> Result<Json<RunFlowResponse>, ApiError> {
    Err(ApiError::internal("Flow resume requires RocksDB backend"))
}

// ============================================================================
// Get Flow Instance Status
// ============================================================================

/// Get the current status of a flow instance.
///
/// Reads the flow instance node from the `raisin:system` workspace and
/// returns its status, variables, and metadata.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_flow_instance(
    State(state): State<AppState>,
    Path((repo, instance_id)): Path<(String, String)>,
    _auth_context: Option<Extension<AuthContext>>,
) -> Result<Json<FlowInstanceStatusResponse>, ApiError> {
    let status = raisin_flow_runtime::service::get_instance_status(
        state.storage.as_ref(),
        &repo,
        &instance_id,
    )
    .await?;

    Ok(Json(FlowInstanceStatusResponse {
        id: status.id,
        status: status.status,
        variables: status.variables,
        flow_path: status.flow_path,
        started_at: status.started_at,
        error: status.error,
    }))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_flow_instance(
    State(_state): State<AppState>,
    Path((_repo, _instance_id)): Path<(String, String)>,
) -> Result<Json<FlowInstanceStatusResponse>, ApiError> {
    Err(ApiError::internal(
        "Flow instance status requires RocksDB backend",
    ))
}

// ============================================================================
// Cancel Flow Instance
// ============================================================================

/// Cancel a running or waiting flow instance.
///
/// Sets the instance status to `cancelled` and attempts to cancel the
/// associated job. Returns 400 if the instance is already in a terminal state.
#[cfg(feature = "storage-rocksdb")]
pub async fn cancel_flow_instance(
    State(state): State<AppState>,
    Path((repo, instance_id)): Path<(String, String)>,
    _auth_context: Option<Extension<AuthContext>>,
) -> Result<Json<CancelFlowInstanceResponse>, ApiError> {
    let scheduler = get_scheduler(&state)?;

    raisin_flow_runtime::service::cancel_instance(
        state.storage.as_ref(),
        scheduler,
        &repo,
        &instance_id,
    )
    .await?;

    Ok(Json(CancelFlowInstanceResponse {
        id: instance_id,
        status: "cancelled".to_string(),
    }))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn cancel_flow_instance(
    State(_state): State<AppState>,
    Path((_repo, _instance_id)): Path<(String, String)>,
) -> Result<Json<CancelFlowInstanceResponse>, ApiError> {
    Err(ApiError::internal(
        "Flow instance cancel requires RocksDB backend",
    ))
}

// ============================================================================
// Delete Flow Instance
// ============================================================================

/// Delete a flow instance node.
///
/// Only allows deletion of instances in terminal states (completed, failed,
/// cancelled, rolled_back). Returns 400 if the instance is still active.
#[cfg(feature = "storage-rocksdb")]
pub async fn delete_flow_instance(
    State(state): State<AppState>,
    Path((repo, instance_id)): Path<(String, String)>,
    _auth_context: Option<Extension<AuthContext>>,
) -> Result<axum::http::StatusCode, ApiError> {
    let scope = StorageScope::new(TENANT_ID, &repo, DEFAULT_BRANCH, SYSTEM_WORKSPACE);

    // Look up by path first (instance UUID), then by node ID
    let instance_path = format!("/flows/instances/{}", instance_id);
    let instance_node = match state
        .storage
        .nodes()
        .get_by_path(scope, &instance_path, None)
        .await
        .map_err(map_storage_error)?
    {
        Some(node) => node,
        None => state
            .storage
            .nodes()
            .get(scope, &instance_id, None)
            .await
            .map_err(map_storage_error)?
            .ok_or_else(|| {
                ApiError::not_found(format!("Flow instance '{}' not found", instance_id))
            })?,
    };

    let instance: raisin_flow_runtime::types::FlowInstance = serde_json::from_value(
        serde_json::to_value(&instance_node.properties).map_err(|e| {
            ApiError::internal(format!("Failed to serialize instance properties: {}", e))
        })?,
    )
    .map_err(|e| ApiError::internal(format!("Failed to deserialize flow instance: {}", e)))?;

    if !instance.is_terminated() {
        return Err(ApiError::validation_failed(format!(
            "Flow instance '{}' is still active (status: {:?}). Cancel it first.",
            instance_id, instance.status
        )));
    }

    state
        .storage
        .nodes()
        .delete(scope, &instance_node.id, DeleteNodeOptions::default())
        .await
        .map_err(map_storage_error)?;

    tracing::info!(instance_id = %instance_id, "Deleted flow instance");

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn delete_flow_instance(
    State(_state): State<AppState>,
    Path((_repo, _instance_id)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    Err(ApiError::internal(
        "Flow instance delete requires RocksDB backend",
    ))
}
