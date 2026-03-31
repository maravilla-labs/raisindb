//! Types for replay engine results

use crate::crdt::ConflictType;
use crate::operation::{Operation, OperationTarget};

/// Result of replaying operations
#[derive(Debug, Clone)]
pub struct ReplayResult {
    /// Operations that were successfully applied
    pub applied: Vec<Operation>,

    /// Conflicts that were detected (even if auto-resolved)
    pub conflicts: Vec<ConflictInfo>,

    /// Operations that were skipped (already applied)
    pub skipped: Vec<Operation>,
}

/// Information about a detected conflict
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    /// The operation that won
    pub winner: Operation,

    /// The operations that lost
    pub losers: Vec<Operation>,

    /// Type of conflict
    pub conflict_type: ConflictType,

    /// The target entity that had the conflict
    pub target: OperationTarget,
}
