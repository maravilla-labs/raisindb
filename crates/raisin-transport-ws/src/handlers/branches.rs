// SPDX-License-Identifier: BSL-1.1

//! Branch management operation handlers

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{BranchRepository, Storage};
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        BranchComparePayload, BranchCreatePayload, BranchDeletePayload, BranchDiffPayload,
        BranchGetHeadPayload, BranchGetPayload, BranchListPayload, BranchMergePayload,
        BranchUpdateHeadPayload, RequestEnvelope, ResponseEnvelope,
    },
};

/// Handle branch creation
pub async fn handle_branch_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchCreatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Parse from_revision if provided
    let from_revision = payload
        .from_revision
        .as_ref()
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| WsError::InvalidRequest(format!("Invalid from_revision: {}", e)))?;

    let branch = state
        .storage
        .branches()
        .create_branch(
            tenant_id,
            repo,
            &payload.name,
            "system", // TODO: Get actor from connection state
            from_revision,
            payload.from_branch.clone(), // upstream_branch
            payload.protected.unwrap_or(false),
            payload.include_revision_history,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(branch)?,
    )))
}

/// Handle branch get
pub async fn handle_branch_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchGetPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let branch = state
        .storage
        .branches()
        .get_branch(tenant_id, repo, &payload.name)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Branch not found: {}", payload.name)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(branch)?,
    )))
}

/// Handle branch list
pub async fn handle_branch_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let _payload: BranchListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let branches = state
        .storage
        .branches()
        .list_branches(tenant_id, repo)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(branches)?,
    )))
}

/// Handle branch deletion
pub async fn handle_branch_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    state
        .storage
        .branches()
        .delete_branch(tenant_id, repo, &payload.name)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({"success": true}),
    )))
}

/// Handle get branch HEAD revision
pub async fn handle_branch_get_head<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchGetHeadPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let head_revision = state
        .storage
        .branches()
        .get_head(tenant_id, repo, &payload.name)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({"revision": head_revision}),
    )))
}

/// Handle update branch HEAD revision
pub async fn handle_branch_update_head<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchUpdateHeadPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Parse revision from HLC string format
    let revision = payload
        .revision
        .parse()
        .map_err(|e| WsError::InvalidRequest(format!("Invalid revision: {}", e)))?;

    state
        .storage
        .branches()
        .set_head(tenant_id, repo, &payload.name, revision)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({"success": true}),
    )))
}

/// Handle branch comparison (calculate divergence)
#[cfg(feature = "storage-rocksdb")]
pub async fn handle_branch_compare<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + 'static,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchComparePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Use the concrete RocksDB handle already held by WsState (the generic
    // `state.storage` cannot be downcast through the `S: Storage` erasure).
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| WsError::OperationError("RocksDB storage not available".to_string()))?;

    let divergence = rocksdb_storage
        .branches_impl()
        .calculate_divergence(tenant_id, repo, &payload.branch, &payload.base_branch)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(divergence)?,
    )))
}

/// Stub for non-RocksDB builds
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_branch_compare<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_SUPPORTED".to_string(),
        "Branch comparison requires RocksDB storage feature".to_string(),
    )))
}

/// Handle branch diff (per-node added/modified/deleted relative to the base)
#[cfg(feature = "storage-rocksdb")]
pub async fn handle_branch_diff<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + 'static,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchDiffPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| WsError::OperationError("RocksDB storage not available".to_string()))?;

    let diff = rocksdb_storage
        .branches_impl()
        .diff_branches(tenant_id, repo, &payload.branch, &payload.base_branch)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(diff)?,
    )))
}

/// Stub for non-RocksDB builds
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_branch_diff<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_SUPPORTED".to_string(),
        "Branch diff requires RocksDB storage feature".to_string(),
    )))
}

/// Handle branch merge
#[cfg(feature = "storage-rocksdb")]
pub async fn handle_branch_merge<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: BranchMergePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Use the concrete RocksDB handle already held by WsState, mirroring the
    // HTTP merge handler (raisin-transport-http/src/handlers/branches.rs).
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| WsError::OperationError("RocksDB storage not available".to_string()))?;

    // payload.strategy: Option<String> -> MergeStrategy. Default to ThreeWay;
    // merge_branches() fast-forwards internally whenever that is possible.
    let strategy = match payload.strategy.as_deref() {
        Some("FastForward") | Some("fast_forward") => raisin_context::MergeStrategy::FastForward,
        _ => raisin_context::MergeStrategy::ThreeWay,
    };
    let message = payload.message.as_deref().unwrap_or("Merge");

    let result = rocksdb_storage
        .branches_impl()
        .merge_branches(
            tenant_id,
            repo,
            &payload.target_branch,
            &payload.source_branch,
            strategy,
            message,
            "system", // TODO: derive actor from connection_state once available
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}

/// Stub for non-RocksDB builds
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_branch_merge<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_SUPPORTED".to_string(),
        "Branch merging requires RocksDB storage feature".to_string(),
    )))
}
