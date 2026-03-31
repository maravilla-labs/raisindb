//! Type definitions for operation log

use hashbrown::HashMap;

/// Statistics about the operation log
#[derive(Debug, Clone)]
pub struct OpLogStats {
    pub total_operations: usize,
    pub operations_per_node: HashMap<String, usize>,
    pub oldest_operation_timestamp: Option<u64>,
    pub newest_operation_timestamp: Option<u64>,
}

impl OpLogStats {
    /// Get the age of the oldest operation in days
    pub fn oldest_age_days(&self) -> Option<u64> {
        self.oldest_operation_timestamp.map(|oldest_ms| {
            let now_ms = chrono::Utc::now().timestamp_millis() as u64;
            if now_ms > oldest_ms {
                (now_ms - oldest_ms) / (1000 * 60 * 60 * 24)
            } else {
                0
            }
        })
    }
}
