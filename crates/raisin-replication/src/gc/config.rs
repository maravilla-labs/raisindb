//! Configuration and result types for garbage collection

use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

use super::watermarks::PeerWatermarks;

/// Configuration for garbage collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    /// Maximum age in days before operations are force-deleted (fail-safe)
    pub max_age_days: u64,

    /// Maximum operation log size in bytes before emergency GC triggers
    pub max_log_size_bytes: u64,

    /// Target log size after emergency GC (90% of max)
    pub target_log_size_bytes: u64,

    /// Minimum number of peers that must acknowledge before GC
    /// (0 = wait for all peers, N = wait for N peers)
    pub min_peer_acknowledgments: usize,

    /// Enable aggressive GC during emergency (ignores min_peer_acknowledgments)
    pub emergency_gc_enabled: bool,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            max_age_days: 30,
            max_log_size_bytes: 10 * 1024 * 1024 * 1024, // 10 GB
            target_log_size_bytes: 9 * 1024 * 1024 * 1024, // 9 GB
            min_peer_acknowledgments: 0,                 // Wait for all peers by default
            emergency_gc_enabled: true,
        }
    }
}

/// Result of a garbage collection run
#[derive(Debug, Clone)]
pub struct GcResult {
    /// Number of operations deleted
    pub deleted_count: usize,

    /// Bytes reclaimed
    pub bytes_reclaimed: u64,

    /// Strategy used for this GC run
    pub strategy: GcStrategy,

    /// Per-node deletion counts
    pub deleted_by_node: HashMap<String, usize>,

    /// Watermarks used for this GC run
    pub watermarks: PeerWatermarks,
}

/// Strategy used for garbage collection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcStrategy {
    /// Normal acknowledgment-based GC
    AcknowledgmentBased,

    /// Time-based fail-safe (force delete old operations)
    TimeBasedFailsafe,

    /// Emergency GC due to size limits
    Emergency,

    /// No GC performed (nothing to delete)
    NoOp,
}
