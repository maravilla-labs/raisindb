//! Shared helper functions for operation log management
//!
//! This module contains utility functions used across multiple oplog modules:
//! - Serialization/deserialization helpers
//! - Key construction
//! - WriteBatch helpers
//! - Iterator helpers

use crate::keys::oplog_key;
use crate::{cf, cf_handle};
use raisin_error::{Error, Result};
use raisin_replication::Operation;
use rocksdb::{ColumnFamily, WriteBatch, DB};
use std::sync::Arc;

/// Serialize an operation to MessagePack format with enhanced error logging
///
/// This centralizes serialization logic and ensures consistent error handling
/// across all serialization sites.
pub(super) fn serialize_operation(op: &Operation) -> Result<Vec<u8>> {
    let value = rmp_serde::to_vec_named(op).map_err(|e| {
        // Enhanced logging for UpdateNodeType operations (common failure case)
        if matches!(
            op.op_type,
            raisin_replication::OpType::UpdateNodeType { .. }
        ) {
            tracing::error!(
                op_id = %op.op_id,
                tenant_id = %op.tenant_id,
                repo_id = %op.repo_id,
                op_seq = op.op_seq,
                "Failed to serialize UpdateNodeType operation: {}",
                e
            );
        }
        Error::storage(format!("Failed to serialize operation: {}", e))
    })?;

    // Debug logging for UpdateNodeType operations
    if matches!(
        op.op_type,
        raisin_replication::OpType::UpdateNodeType { .. }
    ) {
        tracing::debug!(
            op_id = %op.op_id,
            tenant_id = %op.tenant_id,
            repo_id = %op.repo_id,
            op_seq = op.op_seq,
            serialized_size = value.len(),
            "Serialized UpdateNodeType operation"
        );
    }

    Ok(value)
}

/// Deserialize an operation from MessagePack format with enhanced error logging
///
/// This centralizes deserialization logic and provides rich context in error messages.
///
/// # Arguments
///
/// * `value` - The serialized MessagePack bytes
/// * `context` - Optional context string for error messages (e.g., "get_operations_from_seq")
/// * `tenant_id` - Optional tenant ID for logging
/// * `repo_id` - Optional repo ID for logging
pub(super) fn deserialize_operation(
    value: &[u8],
    context: Option<&str>,
    tenant_id: Option<&str>,
    repo_id: Option<&str>,
) -> Result<Operation> {
    let op: Operation = rmp_serde::from_slice(value).map_err(|e| {
        let ctx = context.unwrap_or("unknown");
        tracing::error!(
            tenant_id = tenant_id.unwrap_or("unknown"),
            repo_id = repo_id.unwrap_or("unknown"),
            value_len = value.len(),
            context = ctx,
            "Failed to deserialize operation from {}: {}",
            ctx,
            e
        );
        Error::storage(format!("Failed to deserialize operation: {}", e))
    })?;

    // Debug logging for UpdateNodeType operations
    if matches!(
        op.op_type,
        raisin_replication::OpType::UpdateNodeType { .. }
    ) {
        let ctx = context.unwrap_or("unknown");
        tracing::debug!(
            op_id = %op.op_id,
            tenant_id = %op.tenant_id,
            repo_id = %op.repo_id,
            op_seq = op.op_seq,
            context = ctx,
            "Deserialized UpdateNodeType operation in {}",
            ctx
        );
    }

    Ok(op)
}

/// Build an operation log key from an operation
///
/// This centralizes key construction to ensure consistency.
pub(super) fn build_operation_key(op: &Operation) -> Vec<u8> {
    oplog_key(
        &op.tenant_id,
        &op.repo_id,
        &op.cluster_node_id,
        op.op_seq,
        op.timestamp_ms,
    )
}

/// Write a batch atomically with consistent error handling
pub(super) fn write_batch(db: &Arc<DB>, batch: WriteBatch) -> Result<()> {
    db.write(batch)
        .map_err(|e| Error::storage(format!("Failed to write batch: {}", e)))
}

/// Get the operation log column family handle
pub(super) fn get_oplog_cf(db: &Arc<DB>) -> Result<&ColumnFamily> {
    cf_handle(db, cf::OPERATION_LOG)
}
