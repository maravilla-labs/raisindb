//! Batch Execution Configuration
//!
//! Controls when and how batch execution is used vs row-based execution.
//! This allows for adaptive query execution based on query characteristics.

use super::super::batch::BatchConfig;

/// Configuration for batch execution behavior
///
/// Controls when the query engine should use batch-aware execution vs
/// traditional row-at-a-time execution.
#[derive(Debug, Clone)]
pub struct BatchExecutionConfig {
    /// Underlying batch configuration (batch size, etc.)
    pub batch_config: BatchConfig,

    /// Enable batch execution for table scans
    ///
    /// Table scans benefit most from batching as they process many rows.
    /// Set to false to disable batch execution for table scans (useful for debugging).
    pub enable_batch_scans: bool,

    /// Enable batch execution for index scans (prefix, property)
    ///
    /// Index scans may return fewer rows, so batching benefit varies.
    /// Set to false if index scans show no improvement with batching.
    pub enable_batch_index_scans: bool,

    /// Minimum estimated rows to trigger batch execution
    ///
    /// If a scan is estimated to return fewer than this many rows,
    /// use row-based execution to avoid batching overhead.
    ///
    /// Set to 0 to always use batch execution when enabled.
    /// Recommended: 100-1000 depending on query patterns.
    pub min_rows_for_batch: usize,

    /// Enable adaptive execution
    ///
    /// When true, the executor will dynamically choose between row and batch
    /// execution based on runtime statistics. When false, the decision is
    /// made at planning time based on estimates.
    ///
    /// Future enhancement - currently not implemented.
    pub enable_adaptive_execution: bool,
}

impl Default for BatchExecutionConfig {
    fn default() -> Self {
        Self {
            batch_config: BatchConfig::default(),
            enable_batch_scans: true,
            enable_batch_index_scans: true,
            min_rows_for_batch: 100, // Only use batching for 100+ rows
            enable_adaptive_execution: false,
        }
    }
}

impl BatchExecutionConfig {
    /// Create a new batch execution configuration
    pub fn new(batch_config: BatchConfig) -> Self {
        Self {
            batch_config,
            ..Default::default()
        }
    }

    /// Create configuration that always uses batch execution
    ///
    /// Useful for OLAP workloads where most queries scan many rows.
    pub fn always_batch() -> Self {
        Self {
            batch_config: BatchConfig::default(),
            enable_batch_scans: true,
            enable_batch_index_scans: true,
            min_rows_for_batch: 0, // No threshold
            enable_adaptive_execution: false,
        }
    }

    /// Create configuration that never uses batch execution
    ///
    /// Useful for OLTP workloads with point queries and small result sets.
    pub fn never_batch() -> Self {
        Self {
            batch_config: BatchConfig::default(),
            enable_batch_scans: false,
            enable_batch_index_scans: false,
            min_rows_for_batch: usize::MAX, // Impossible threshold
            enable_adaptive_execution: false,
        }
    }

    /// Create configuration optimized for low latency
    ///
    /// Uses smaller batches to reduce latency at the cost of some throughput.
    pub fn low_latency() -> Self {
        Self {
            batch_config: BatchConfig::small_batches(),
            enable_batch_scans: true,
            enable_batch_index_scans: false, // Index scans are typically fast already
            min_rows_for_batch: 1000,        // Higher threshold
            enable_adaptive_execution: false,
        }
    }

    /// Create configuration optimized for high throughput
    ///
    /// Uses larger batches to maximize throughput, accepting higher latency.
    pub fn high_throughput() -> Self {
        Self {
            batch_config: BatchConfig::large_batches(),
            enable_batch_scans: true,
            enable_batch_index_scans: true,
            min_rows_for_batch: 10, // Lower threshold
            enable_adaptive_execution: false,
        }
    }

    /// Check if batch execution should be used for a table scan
    ///
    /// # Arguments
    ///
    /// * `estimated_rows` - Estimated number of rows the scan will return
    ///
    /// # Returns
    ///
    /// `true` if batch execution should be used, `false` otherwise
    pub fn should_use_batch_for_table_scan(&self, estimated_rows: Option<usize>) -> bool {
        if !self.enable_batch_scans {
            return false;
        }

        if let Some(rows) = estimated_rows {
            rows >= self.min_rows_for_batch
        } else {
            // No estimate available, default to using batch if enabled
            true
        }
    }

    /// Check if batch execution should be used for an index scan
    ///
    /// # Arguments
    ///
    /// * `estimated_rows` - Estimated number of rows the scan will return
    ///
    /// # Returns
    ///
    /// `true` if batch execution should be used, `false` otherwise
    pub fn should_use_batch_for_index_scan(&self, estimated_rows: Option<usize>) -> bool {
        if !self.enable_batch_index_scans {
            return false;
        }

        if let Some(rows) = estimated_rows {
            rows >= self.min_rows_for_batch
        } else {
            // No estimate available, default to using batch if enabled
            true
        }
    }

    /// Get the underlying batch configuration
    pub fn batch_config(&self) -> &BatchConfig {
        &self.batch_config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BatchExecutionConfig::default();
        assert!(config.enable_batch_scans);
        assert!(config.enable_batch_index_scans);
        assert_eq!(config.min_rows_for_batch, 100);
        assert_eq!(config.batch_config.default_batch_size, 1000);
    }

    #[test]
    fn test_always_batch() {
        let config = BatchExecutionConfig::always_batch();
        assert!(config.should_use_batch_for_table_scan(Some(1)));
        assert!(config.should_use_batch_for_index_scan(Some(1)));
        assert!(config.should_use_batch_for_table_scan(None));
    }

    #[test]
    fn test_never_batch() {
        let config = BatchExecutionConfig::never_batch();
        assert!(!config.should_use_batch_for_table_scan(Some(1_000_000)));
        assert!(!config.should_use_batch_for_index_scan(Some(1_000_000)));
        assert!(!config.should_use_batch_for_table_scan(None));
    }

    #[test]
    fn test_low_latency() {
        let config = BatchExecutionConfig::low_latency();
        assert!(config.should_use_batch_for_table_scan(Some(2000)));
        assert!(!config.should_use_batch_for_table_scan(Some(500)));
        assert!(!config.should_use_batch_for_index_scan(Some(2000))); // Disabled
        assert_eq!(config.batch_config.default_batch_size, 100);
    }

    #[test]
    fn test_high_throughput() {
        let config = BatchExecutionConfig::high_throughput();
        assert!(config.should_use_batch_for_table_scan(Some(20)));
        assert!(!config.should_use_batch_for_table_scan(Some(5)));
        assert!(config.should_use_batch_for_index_scan(Some(20)));
        assert_eq!(config.batch_config.default_batch_size, 5000);
    }

    #[test]
    fn test_min_rows_threshold() {
        let config = BatchExecutionConfig {
            batch_config: BatchConfig::default(),
            enable_batch_scans: true,
            enable_batch_index_scans: true,
            min_rows_for_batch: 500,
            enable_adaptive_execution: false,
        };

        assert!(!config.should_use_batch_for_table_scan(Some(499)));
        assert!(config.should_use_batch_for_table_scan(Some(500)));
        assert!(config.should_use_batch_for_table_scan(Some(1000)));
    }

    #[test]
    fn test_no_estimate_defaults_to_enabled() {
        let config = BatchExecutionConfig::default();
        assert!(config.should_use_batch_for_table_scan(None));
        assert!(config.should_use_batch_for_index_scan(None));
    }

    #[test]
    fn test_custom_batch_size() {
        let batch_config = BatchConfig::new(2000);
        let exec_config = BatchExecutionConfig::new(batch_config);
        assert_eq!(exec_config.batch_config().default_batch_size, 2000);
    }
}
