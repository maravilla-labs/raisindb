//! Operation replay engine for applying operations in causal order
//!
//! Handles sorting, grouping, CRDT merging, and idempotency checking.

use crate::crdt::{CrdtMerge, MergeResult};
use crate::operation::{Operation, OperationTarget};
use hashbrown::HashMap;
use std::cmp::Ordering;
use std::collections::HashSet;
use uuid::Uuid;

use super::idempotency::{IdempotencyTracker, InMemoryIdempotencyTracker};
use super::types::{ConflictInfo, ReplayResult};

/// Engine for replaying operations in causal order
pub struct ReplayEngine {
    /// Tracker for operation IDs that have already been applied (for idempotency)
    idempotency_tracker: Box<dyn IdempotencyTracker>,
}

impl ReplayEngine {
    /// Create a new replay engine with in-memory idempotency tracking
    pub fn new() -> Self {
        Self {
            idempotency_tracker: Box::new(InMemoryIdempotencyTracker::new()),
        }
    }

    /// Create a replay engine with a custom idempotency tracker
    pub fn with_tracker(tracker: Box<dyn IdempotencyTracker>) -> Self {
        Self {
            idempotency_tracker: tracker,
        }
    }

    /// Create a replay engine with a set of already-applied operation IDs (in-memory)
    pub fn with_applied_ops(applied_ops: HashSet<Uuid>) -> Self {
        Self {
            idempotency_tracker: Box::new(InMemoryIdempotencyTracker::with_applied_ops(
                applied_ops,
            )),
        }
    }

    /// Replay a batch of operations in causal order
    ///
    /// This is the main entry point for the replay engine. It:
    /// 1. Filters out already-applied operations (idempotency)
    /// 2. Sorts operations by causal order (vector clocks)
    /// 3. Groups operations by target entity
    /// 4. Applies CRDT merge rules
    /// 5. Returns the final set of operations to apply
    pub fn replay(&mut self, operations: Vec<Operation>) -> ReplayResult {
        // Step 1: Filter out already-applied operations
        let (new_ops, skipped) = self.filter_applied(operations);

        if new_ops.is_empty() {
            return ReplayResult {
                applied: vec![],
                conflicts: vec![],
                skipped,
            };
        }

        // Step 2: Sort by causal order
        let sorted_ops = Self::causal_sort(new_ops);

        // Step 3: Group by target entity
        let grouped_ops = Self::group_by_target(sorted_ops);

        // Step 4: Apply CRDT merge rules
        let mut applied = Vec::new();
        let mut conflicts = Vec::new();

        for (target, ops) in grouped_ops {
            match CrdtMerge::merge_operations(ops.clone()) {
                MergeResult::Winner(op) => {
                    applied.push(op);
                }
                MergeResult::Conflict {
                    winner,
                    losers,
                    conflict_type,
                } => {
                    applied.push(winner.clone());
                    conflicts.push(ConflictInfo {
                        winner,
                        losers,
                        conflict_type,
                        target: target.clone(),
                    });
                }
            }
        }

        // Mark all applied operations
        for op in &applied {
            if let Err(e) = self
                .idempotency_tracker
                .mark_applied(&op.op_id, op.timestamp_ms)
            {
                tracing::error!(
                    op_id = %op.op_id,
                    error = %e,
                    "Failed to mark operation as applied"
                );
            }
        }

        ReplayResult {
            applied,
            conflicts,
            skipped,
        }
    }

    /// Filter out operations that have already been applied
    fn filter_applied(&self, operations: Vec<Operation>) -> (Vec<Operation>, Vec<Operation>) {
        let mut new_ops = Vec::new();
        let mut skipped = Vec::new();

        for op in operations {
            match self.idempotency_tracker.is_applied(&op.op_id) {
                Ok(true) => skipped.push(op),
                Ok(false) => new_ops.push(op),
                Err(e) => {
                    // Log error but don't skip operation (safety: reapply if uncertain)
                    tracing::warn!(
                        op_id = %op.op_id,
                        error = %e,
                        "Failed to check idempotency, will reapply"
                    );
                    new_ops.push(op);
                }
            }
        }

        (new_ops, skipped)
    }

    /// Sort operations by causal order using vector clocks
    ///
    /// This implements a topological sort based on the happens-before relationship.
    /// Operations are sorted such that if A happens-before B, then A appears before B.
    ///
    /// For concurrent operations, we use a stable sort with timestamp and node_id as tie-breakers.
    pub fn causal_sort(mut operations: Vec<Operation>) -> Vec<Operation> {
        // Use stable sort to preserve insertion order for concurrent operations
        operations.sort_by(Self::compare_causal);
        operations
    }

    /// Compare two operations for causal ordering
    fn compare_causal(a: &Operation, b: &Operation) -> Ordering {
        if a.vector_clock.happens_before(&b.vector_clock) {
            return Ordering::Less;
        }
        if a.vector_clock.happens_after(&b.vector_clock) {
            return Ordering::Greater;
        }

        // Concurrent operations - use timestamp as tie-breaker
        match a.timestamp_ms.cmp(&b.timestamp_ms) {
            Ordering::Less => Ordering::Less,
            Ordering::Greater => Ordering::Greater,
            Ordering::Equal => {
                // Final tie-breaker: cluster_node_id (deterministic)
                a.cluster_node_id.cmp(&b.cluster_node_id)
            }
        }
    }

    /// Group operations by their target entity
    ///
    /// Operations targeting the same entity are grouped together for CRDT merging.
    pub fn group_by_target(operations: Vec<Operation>) -> HashMap<OperationTarget, Vec<Operation>> {
        let mut grouped: HashMap<OperationTarget, Vec<Operation>> = HashMap::new();

        for op in operations {
            let target = op.target();
            grouped.entry(target).or_insert_with(Vec::new).push(op);
        }

        grouped
    }

    /// Mark an operation as applied (for idempotency)
    pub fn mark_applied(&mut self, op_id: Uuid, timestamp_ms: u64) -> Result<(), String> {
        self.idempotency_tracker.mark_applied(&op_id, timestamp_ms)
    }

    /// Check if an operation has been applied
    pub fn is_applied(&self, op_id: &Uuid) -> Result<bool, String> {
        self.idempotency_tracker.is_applied(op_id)
    }
}

impl Default for ReplayEngine {
    fn default() -> Self {
        Self::new()
    }
}
