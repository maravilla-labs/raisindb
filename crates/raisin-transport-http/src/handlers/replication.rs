//! HTTP handlers for CRDT replication synchronization
//!
//! This module provides REST API endpoints for exchanging operations between
//! peers in a distributed RaisinDB cluster using CRDT replication.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

use crate::error::ApiError;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use raisin_replication::{Operation, VectorClock};

/// Request parameters for fetching operations
#[derive(Debug, Deserialize)]
pub struct GetOperationsQuery {
    /// Fetch operations with sequence number greater than this value
    /// If None, fetch all operations
    pub since_op_seq: Option<u64>,

    /// Filter operations from this specific node
    /// If None, fetch operations from all nodes
    pub node_id: Option<String>,

    /// Filter operations by branches
    /// Can be specified multiple times: ?branch=main&branch=develop
    /// If None or empty, fetch operations from all branches
    #[serde(default)]
    pub branch: Vec<String>,

    /// Maximum number of operations to return
    /// If None, no limit
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    1000
}

/// Response containing a batch of operations
#[derive(Debug, Serialize)]
pub struct OperationsResponse {
    /// List of operations
    pub operations: Vec<Operation>,

    /// Current vector clock for this repository
    pub vector_clock: VectorClock,

    /// Total number of operations available (for pagination)
    pub total_available: usize,

    /// Whether there are more operations available
    pub has_more: bool,
}

/// Request body for applying a batch of operations
#[derive(Debug, Deserialize)]
pub struct ApplyOperationsBatchRequest {
    /// Operations to apply
    pub operations: Vec<Operation>,

    /// Sending peer's ID
    pub peer_id: String,

    /// Sending peer's current vector clock
    pub peer_vector_clock: VectorClock,
}

/// Response for batch operation application
#[derive(Debug, Serialize)]
pub struct ApplyOperationsBatchResponse {
    /// Number of operations successfully applied
    pub applied_count: usize,

    /// Number of operations skipped (already applied)
    pub skipped_count: usize,

    /// Number of conflicts detected
    pub conflicts_count: usize,

    /// Updated vector clock after applying operations
    pub vector_clock: VectorClock,

    /// List of operation IDs that were applied
    pub applied_operation_ids: Vec<String>,
}

/// GET /api/replication/:tenant/:repo/operations
///
/// Fetch operations from this node for synchronization
pub async fn get_operations(
    State(_state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Query(params): Query<GetOperationsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        // Only RocksDB implementation supports replication currently
        return Err(ApiError::not_implemented(
            "Replication is only supported with RocksDB storage backend",
        ));
    }

    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::state::get_rocksdb_from_state;

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            since_op_seq = ?params.since_op_seq,
            node_id = ?params.node_id,
            branch_filter = ?params.branch,
            limit = params.limit,
            "Fetching operations for replication"
        );

        let db = get_rocksdb_from_state(&_state)?;
        let oplog_repo = raisin_rocksdb::OpLogRepository::new(db);

        // Fetch operations based on parameters
        let mut operations = if let Some(node_id) = &params.node_id {
            // Fetch from specific node
            let since_seq = params.since_op_seq.unwrap_or(0);
            oplog_repo.get_operations_from_seq(&tenant_id, &repo_id, node_id, since_seq)?
        } else {
            // Fetch from all nodes
            let all_ops = oplog_repo.get_all_operations(&tenant_id, &repo_id)?;
            let mut ops_vec: Vec<Operation> = all_ops.into_values().flatten().collect();

            // Filter by since_op_seq if provided
            if let Some(since_seq) = params.since_op_seq {
                ops_vec.retain(|op| op.op_seq > since_seq);
            }

            // Sort by timestamp for consistent ordering
            ops_vec.sort_by_key(|op| op.timestamp_ms);
            ops_vec
        };

        // Apply branch filter to all operations (both specific node and all nodes)
        if !params.branch.is_empty() {
            operations.retain(|op| params.branch.contains(&op.branch));
        }

        let total_available = operations.len();
        let has_more = total_available > params.limit;

        // Apply limit
        let operations: Vec<Operation> = operations.into_iter().take(params.limit).collect();

        // Build current vector clock
        let all_ops = oplog_repo.get_all_operations(&tenant_id, &repo_id)?;
        let mut vector_clock = VectorClock::new();
        for (node_id, ops) in all_ops {
            if let Some(max_op) = ops.iter().max_by_key(|op| op.op_seq) {
                vector_clock.set(&node_id, max_op.op_seq);
            }
        }

        let response = OperationsResponse {
            operations,
            vector_clock,
            total_available,
            has_more,
        };

        Ok((StatusCode::OK, Json(response)))
    }
}

/// POST /api/replication/:tenant/:repo/operations/batch
///
/// Apply a batch of operations from a remote peer
pub async fn apply_operations_batch(
    State(_state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Json(request): Json<ApplyOperationsBatchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        return Err(ApiError::not_implemented(
            "Replication is only supported with RocksDB storage backend",
        ));
    }

    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::state::get_rocksdb_from_state;

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            peer_id = %request.peer_id,
            operations_count = request.operations.len(),
            "Applying operations batch from peer"
        );

        let db = get_rocksdb_from_state(&_state)?;
        let oplog_repo = raisin_rocksdb::OpLogRepository::new(db.clone());

        // Filter operations we haven't seen yet
        let total_operations_count = request.operations.len();
        let mut new_operations = Vec::new();
        for op in request.operations {
            // Check if we've already seen this operation
            let existing_ops =
                oplog_repo.get_operations_from_node(&tenant_id, &repo_id, &op.cluster_node_id)?;

            let already_exists = existing_ops
                .iter()
                .any(|existing| existing.op_id == op.op_id || existing.op_seq == op.op_seq);

            if !already_exists {
                new_operations.push(op);
            }
        }

        let skipped_count = total_operations_count - new_operations.len();

        // Apply new operations using ReplayEngine
        let mut replay_engine = raisin_replication::ReplayEngine::new();
        let replay_result = replay_engine.replay(new_operations);

        // Store applied operations
        if !replay_result.applied.is_empty() {
            oplog_repo.put_operations_batch(&replay_result.applied)?;
        }

        let applied_count = replay_result.applied.len();
        let conflicts_count = replay_result.conflicts.len();

        // Acknowledge operations from peer
        for op in &replay_result.applied {
            oplog_repo.acknowledge_operations(
                &tenant_id,
                &repo_id,
                &op.cluster_node_id,
                request.peer_id.clone(),
                op.op_seq,
            )?;
        }

        // Build updated vector clock
        let all_ops = oplog_repo.get_all_operations(&tenant_id, &repo_id)?;
        let mut vector_clock = VectorClock::new();
        for (node_id, ops) in all_ops {
            if let Some(max_op) = ops.iter().max_by_key(|op| op.op_seq) {
                vector_clock.set(&node_id, max_op.op_seq);
            }
        }

        let applied_operation_ids: Vec<String> = replay_result
            .applied
            .iter()
            .map(|op| op.op_id.to_string())
            .collect();

        if conflicts_count > 0 {
            warn!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                conflicts = conflicts_count,
                "Conflicts detected during operation replay"
            );
        }

        let response = ApplyOperationsBatchResponse {
            applied_count,
            skipped_count,
            conflicts_count,
            vector_clock,
            applied_operation_ids,
        };

        Ok((StatusCode::OK, Json(response)))
    }
}

/// GET /api/replication/:tenant/:repo/vector-clock
///
/// Get the current vector clock for a repository
pub async fn get_vector_clock(
    State(_state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        return Err(ApiError::not_implemented(
            "Replication is only supported with RocksDB storage backend",
        ));
    }

    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::state::get_rocksdb_from_state;

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            "Fetching vector clock"
        );

        let db = get_rocksdb_from_state(&_state)?;
        let oplog_repo = raisin_rocksdb::OpLogRepository::new(db);

        // Build vector clock from all operations
        let all_ops = oplog_repo.get_all_operations(&tenant_id, &repo_id)?;
        let mut vector_clock = VectorClock::new();

        for (node_id, ops) in all_ops {
            if let Some(max_op) = ops.iter().max_by_key(|op| op.op_seq) {
                vector_clock.set(&node_id, max_op.op_seq);
            }
        }

        let response = serde_json::json!({
            "vector_clock": vector_clock,
            "nodes": vector_clock.node_ids().collect::<Vec<_>>(),
        });

        Ok((StatusCode::OK, Json(response)))
    }
}

/// Response containing the current vector clock
#[derive(Debug, Serialize)]
pub struct VectorClockResponse {
    /// The vector clock
    pub vector_clock: VectorClock,

    /// List of node IDs in the clock
    pub nodes: Vec<String>,

    /// Watermarks: highest sequence per node
    pub watermarks: HashMap<String, u64>,
}
