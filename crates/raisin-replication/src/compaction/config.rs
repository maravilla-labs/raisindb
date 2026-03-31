//! Configuration and result types for operation log compaction

use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

/// Configuration for operation log compaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Minimum age of operations to compact (seconds)
    ///
    /// Operations younger than this will be preserved to allow for proper
    /// conflict resolution during sync. Default: 3600 (1 hour)
    pub min_age_secs: u64,

    /// Whether to merge consecutive SetProperty operations
    ///
    /// When enabled, sequences of property updates on the same node/property
    /// will be collapsed to only the final value. Default: true
    pub merge_property_updates: bool,

    /// Maximum operations to process per compaction run
    ///
    /// Limits the batch size to prevent long-running compaction jobs.
    /// Default: 100,000
    pub batch_size: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            min_age_secs: 3600, // 1 hour
            merge_property_updates: true,
            batch_size: 100_000,
        }
    }
}

/// Result of a compaction operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// Number of operations before compaction
    pub original_count: usize,

    /// Number of operations after compaction
    pub compacted_count: usize,

    /// Number of operations merged (removed)
    pub merged_count: usize,

    /// Estimated bytes saved
    pub bytes_saved: usize,

    /// Per-node statistics
    pub per_node_stats: HashMap<String, NodeCompactionStats>,
}

/// Statistics for compaction of a specific cluster node's operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCompactionStats {
    /// Original operation count for this node
    pub original_count: usize,

    /// Compacted operation count for this node
    pub compacted_count: usize,

    /// Number of SetProperty sequences merged
    pub property_sequences_merged: usize,
}
