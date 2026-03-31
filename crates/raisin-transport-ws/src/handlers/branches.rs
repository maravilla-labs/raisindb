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
        BranchComparePayload, BranchCreatePayload, BranchDeletePayload, BranchGetHeadPayload,
        BranchGetPayload, BranchListPayload, BranchMergePayload, BranchUpdateHeadPayload,
        RequestEnvelope, ResponseEnvelope,
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
        .update_head(tenant_id, repo, &payload.name, revision)
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

    // We need to downcast to RocksDB storage to access calculate_divergence
    // This is a limitation of the current storage abstraction
    let storage_any = &state.storage as &dyn std::any::Any;

    if let Some(rocksdb_storage) = storage_any.downcast_ref::<raisin_rocksdb::RocksDBStorage>() {
        let divergence = rocksdb_storage
            .branches_impl()
            .calculate_divergence(tenant_id, repo, &payload.branch, &payload.base_branch)
            .await?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::to_value(divergence)?,
        )))
    } else {
        Err(WsError::OperationError(
            "Branch comparison requires RocksDB storage".to_string(),
        ))
    }
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

    // TODO: Implement merge once MergeStrategy is defined in raisin_storage
    // For now, return not implemented
    _ = (tenant_id, repo, payload); // Suppress unused variable warnings

    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Branch merging not yet implemented".to_string(),
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
