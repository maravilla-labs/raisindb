//! Configuration for the embedding worker.

use std::time::Duration;

/// Configuration for the embedding worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub batch_size: usize,
    pub poll_interval: Duration,
    pub max_retries: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            batch_size: 5,
            poll_interval: Duration::from_secs(2),
            max_retries: 3,
        }
    }
}
