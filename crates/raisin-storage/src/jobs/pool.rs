// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Generic worker pool trait for managing background job workers

use async_trait::async_trait;
use raisin_error::Result;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

/// Trait for managing a pool of background workers
#[async_trait]
pub trait WorkerPool: Send + Sync {
    /// Start the worker pool
    ///
    /// Spawns N worker tasks that continuously poll the job queue and process jobs.
    ///
    /// # Returns
    ///
    /// Vector of join handles for the spawned worker tasks
    async fn start(&self) -> Result<Vec<JoinHandle<()>>>;

    /// Stop the worker pool gracefully
    ///
    /// Signals all workers to finish their current jobs and shut down.
    async fn stop(&self);

    /// Get current worker pool statistics
    async fn stats(&self) -> WorkerPoolStats;
}

/// Statistics about worker pool operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPoolStats {
    /// Number of active worker threads (total across all pools)
    pub active_workers: usize,
    /// Number of jobs pending in queue (total across all pools)
    pub pending_jobs: usize,
    /// Total number of completed jobs
    pub completed_jobs: u64,
    /// Total number of failed jobs
    pub failed_jobs: u64,
    /// Average job processing time in milliseconds
    pub avg_processing_time_ms: Option<f64>,
    /// Per-category pool statistics (empty for single-pool mode)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category_stats: Vec<CategoryPoolStats>,
}

/// Statistics for a single category pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryPoolStats {
    /// Category name (realtime, background, system)
    pub category: String,
    /// Active handler tasks in this pool
    pub active_handler_tasks: usize,
    /// Available handler semaphore permits
    pub handler_permits_available: usize,
    /// Maximum handler semaphore permits
    pub handler_permits_max: usize,
    /// Queue depth: high priority
    pub queue_depth_high: usize,
    /// Queue depth: normal priority
    pub queue_depth_normal: usize,
    /// Queue depth: low priority
    pub queue_depth_low: usize,
    /// Number of dispatcher workers
    pub dispatcher_workers: usize,
}
