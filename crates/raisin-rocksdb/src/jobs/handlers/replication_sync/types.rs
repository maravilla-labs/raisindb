//! Request and response types for replication sync.

use raisin_replication::{Operation, VectorClock};
use serde::{Deserialize, Serialize};

/// Response from fetching operations from a peer
#[derive(Debug, Deserialize)]
pub(super) struct OperationsResponse {
    pub operations: Vec<Operation>,
    pub vector_clock: VectorClock,
    pub total_available: usize,
    pub has_more: bool,
}

/// Request for applying operations batch to a peer
#[derive(Debug, Serialize)]
pub(super) struct ApplyOperationsBatchRequest {
    pub operations: Vec<Operation>,
    pub peer_id: String,
    pub peer_vector_clock: VectorClock,
}

/// Response from applying operations batch
#[derive(Debug, Deserialize)]
pub(super) struct ApplyOperationsBatchResponse {
    pub applied_count: usize,
    pub skipped_count: usize,
    pub conflicts_count: usize,
    pub vector_clock: VectorClock,
    pub applied_operation_ids: Vec<String>,
}
