//! # Operation Log Compaction
//!
//! This module provides smart compaction of the operation log to reduce storage
//! footprint and improve synchronization efficiency.
//!
//! ## Safety Guarantees
//!
//! The compaction system preserves CRDT semantics by:
//! - Only merging operations from the **same cluster node** (preserves causality)
//! - Only merging operations on the **same storage node + property** (preserves semantics)
//! - Preserving **vector clocks** from the latest operation (maintains causal ordering)
//! - Never merging across **different operation types** (only SetProperty)
//! - Respecting **minimum age** threshold (avoids compacting recent ops needed for conflict resolution)

mod compactor;
pub mod config;
#[cfg(test)]
mod tests;

pub use compactor::OperationLogCompactor;
pub use config::{CompactionConfig, CompactionResult, NodeCompactionStats};
