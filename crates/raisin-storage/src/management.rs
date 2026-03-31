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

//! Management operations for storage implementations
//!
//! These traits provide enterprise-grade operations like integrity checking,
//! self-healing, monitoring, and maintenance for all storage backends.

use crate::Storage;
use async_trait::async_trait;
use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Management operations for storage backends
#[async_trait]
pub trait ManagementOps: Storage {
    /// Check integrity of data for a specific tenant
    async fn check_integrity(&self, tenant: &str) -> Result<IntegrityReport>;

    /// Verify indexes consistency for a tenant
    async fn verify_indexes(&self, tenant: &str) -> Result<Vec<IndexIssue>>;

    /// Rebuild indexes for a tenant
    async fn rebuild_indexes(&self, tenant: &str, index_type: IndexType) -> Result<RebuildStats>;

    /// Clean up orphaned data for a tenant
    async fn cleanup_orphans(&self, tenant: &str) -> Result<u32>;

    /// Get health status for the storage system
    async fn get_health(&self, tenant: Option<&str>) -> Result<HealthStatus>;

    /// Get metrics for monitoring
    async fn get_metrics(&self, tenant: Option<&str>) -> Result<Metrics>;

    /// Trigger manual compaction
    async fn compact(&self, tenant: Option<&str>) -> Result<CompactionStats>;

    /// Create a backup for a tenant
    async fn backup_tenant(&self, tenant: &str, dest: &Path) -> Result<BackupInfo>;

    /// Restore a tenant from backup
    async fn restore_tenant(&self, tenant: &str, src: &Path) -> Result<()>;

    /// Backup all tenants
    async fn backup_all(&self, dest: &Path) -> Result<Vec<BackupInfo>>;
}

/// Background job management for storage backends
#[async_trait]
pub trait BackgroundJobs: Storage {
    /// Start background jobs (integrity scanning, auto-healing, etc.)
    fn start_background_jobs(&self) -> Result<crate::JobHandle>;

    /// Schedule an integrity scan for a tenant
    fn schedule_integrity_scan(&self, tenant: &str, interval: Duration) -> Result<crate::JobId>;

    /// Get status of a background job (async for accessing shared state)
    async fn get_job_status(&self, job_id: &crate::JobId) -> Result<crate::JobStatus> {
        // Use global job registry by default
        crate::jobs::global_registry().get_status(job_id).await
    }

    /// Cancel a background job (async for accessing shared state)
    async fn cancel_job(&self, job_id: &crate::JobId) -> Result<()> {
        // Use global job registry by default
        crate::jobs::global_registry().cancel_job(job_id).await
    }

    /// Delete a background job (async for accessing shared state)
    async fn delete_job(&self, job_id: &crate::JobId) -> Result<()> {
        // Use global job registry by default
        crate::jobs::global_registry().delete_job(job_id).await
    }

    /// Delete multiple background jobs in a single batch operation
    ///
    /// More efficient than calling delete_job in a loop - acquires locks once.
    /// Returns (deleted_count, skipped_count) where skipped jobs are running/scheduled.
    async fn delete_jobs_batch(&self, job_ids: &[crate::JobId]) -> (usize, usize) {
        crate::jobs::global_registry()
            .delete_jobs_batch(job_ids)
            .await
    }

    /// List all active background jobs (async for accessing shared state)
    async fn list_jobs(&self) -> Result<Vec<crate::JobInfo>> {
        // Use global job registry by default
        Ok(crate::jobs::global_registry().list_jobs().await)
    }

    /// Wait for a job to complete
    async fn wait_for_job(&self, job_id: &crate::JobId) -> Result<crate::JobStatus> {
        crate::jobs::global_registry()
            .wait_for_completion(job_id)
            .await
    }

    /// Get full job information including results (async for accessing shared state)
    async fn get_job_info(&self, job_id: &crate::JobId) -> Result<crate::JobInfo> {
        // Use global job registry by default
        crate::jobs::global_registry().get_job_info(job_id).await
    }

    /// Purge all jobs from persistent storage (nuclear option)
    ///
    /// Deletes ALL job entries regardless of status or deserializability.
    /// Returns the number of entries purged.
    async fn purge_all_jobs(&self) -> Result<usize> {
        Err(raisin_error::Error::Validation(
            "purge_all_jobs not supported by this storage backend".to_string(),
        ))
    }

    /// Purge only orphaned (undeserializable) jobs from persistent storage
    ///
    /// Only deletes entries that fail deserialization. Returns the number purged.
    async fn purge_orphaned_jobs(&self) -> Result<usize> {
        Err(raisin_error::Error::Validation(
            "purge_orphaned_jobs not supported by this storage backend".to_string(),
        ))
    }

    /// Get job queue statistics including queue depths and persisted entry counts
    async fn get_job_queue_stats(&self) -> Result<JobQueueStats> {
        Err(raisin_error::Error::Validation(
            "get_job_queue_stats not supported by this storage backend".to_string(),
        ))
    }

    /// Force-fail jobs that have been stuck in Running state for too long
    ///
    /// Finds all jobs with status=Running where started_at is older than
    /// `stuck_minutes` ago, and marks them as Failed.
    /// Returns (failed_count, list of job IDs that were force-failed).
    async fn force_fail_stuck_jobs(&self, stuck_minutes: u64) -> Result<(usize, Vec<String>)> {
        Err(raisin_error::Error::Validation(
            "force_fail_stuck_jobs not supported by this storage backend".to_string(),
        ))
    }
}

/// Statistics about the job queue system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobQueueStats {
    pub queue: QueueDepthStats,
    pub workers: WorkerStats,
    pub persisted: PersistedStats,
    /// Per-category queue depth breakdown (Realtime, Background, System)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<CategoryQueueDepthStats>,
}

/// Queue depth statistics per priority level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueDepthStats {
    pub high_queue_len: usize,
    pub high_queue_capacity: usize,
    pub normal_queue_len: usize,
    pub normal_queue_capacity: usize,
    pub low_queue_len: usize,
    pub low_queue_capacity: usize,
    pub total_high_dispatched: u64,
    pub total_normal_dispatched: u64,
    pub total_low_dispatched: u64,
}

/// Per-category queue depth breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryQueueDepthStats {
    /// Category name: "Realtime", "Background", or "System"
    pub category: String,
    pub high_queue_len: usize,
    pub normal_queue_len: usize,
    pub low_queue_len: usize,
    pub total_dispatched: u64,
}

/// Worker pool statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStats {
    pub pool_size: usize,
}

/// Persisted job storage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStats {
    pub total_entries: usize,
    pub orphaned_entries: usize,
}

// Data structures for management operations

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub tenant: String,
    pub scan_time: chrono::DateTime<chrono::Utc>,
    pub nodes_checked: u64,
    pub issues_found: Vec<Issue>,
    pub health_score: f32, // 0.0 to 1.0
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Issue {
    OrphanedNode {
        id: String,
        parent_id: Option<String>,
    },
    MissingIndex {
        node_id: String,
        index_type: IndexType,
    },
    InconsistentIndex {
        node_id: String,
        expected: String,
        actual: String,
    },
    CorruptedData {
        node_id: String,
        error: String,
    },
    BrokenReference {
        from_id: String,
        to_id: String,
        ref_type: String,
    },
    DuplicateChild {
        parent_id: String,
        child_id: String,
    },
    MissingWorkspace {
        node_id: String,
        workspace_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexIssue {
    pub index_type: IndexType,
    pub node_id: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IndexType {
    Property,
    Reference,
    ChildOrder,
    FullText,
    Vector,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildStats {
    pub index_type: IndexType,
    pub items_processed: u64,
    pub errors: u64,
    pub duration_ms: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: HealthLevel,
    pub tenant: Option<String>,
    pub checks: Vec<HealthCheck>,
    pub needs_healing: bool,
    pub last_check: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HealthLevel {
    Healthy,
    Degraded,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub name: String,
    pub status: HealthLevel,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub tenant: Option<String>,
    pub operations_per_sec: f64,
    pub error_rate: f64,
    pub disk_usage_bytes: u64,
    pub index_sizes: std::collections::HashMap<String, u64>,
    pub node_count: u64,
    pub active_connections: u32,
    pub cache_hit_rate: f64,
    pub last_compaction: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionStats {
    pub tenant: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub duration_ms: u64,
    pub files_compacted: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub tenant: String,
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub node_count: u64,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairResult {
    pub tenant: String,
    pub issues_repaired: usize,
    pub issues_failed: usize,
    pub repairs_by_type: std::collections::HashMap<String, usize>,
    pub duration_ms: u64,
    pub errors: Vec<String>,
}

// New types for index management

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReport {
    pub index_type: String,
    pub status: IndexStatus,
    pub issues: Vec<String>,
    pub health_score: f32,
    pub total_entries: u64,
    pub corrupted_entries: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexStatus {
    Healthy,
    Degraded,
    Missing,
    Corrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexHealth {
    pub index_type: String,
    pub memory_usage_bytes: u64,
    pub disk_usage_bytes: u64,
    pub entry_count: u64,
    pub cache_hit_rate: Option<f32>,
    pub last_optimized: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeStats {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub duration_ms: u64,
    pub segments_merged: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreStats {
    pub entries_restored: u64,
    pub entries_skipped: u64,
    pub duration_ms: u64,
}

/// Extended index management operations for fulltext and vector indexes
#[async_trait]
pub trait IndexManagement: Send + Sync {
    // RocksDB index operations (already exists in ManagementOps)
    // We keep these here for consistency

    // Fulltext (Tantivy) operations
    async fn verify_fulltext_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexReport>;

    async fn rebuild_fulltext_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RebuildStats>;

    async fn optimize_fulltext_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<OptimizeStats>;

    async fn purge_fulltext_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<()>;

    async fn fulltext_index_health(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexHealth>;

    // Vector (HNSW) operations - stubs for Phase 3
    async fn verify_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexReport>;

    async fn rebuild_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RebuildStats>;

    async fn optimize_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<OptimizeStats>;

    async fn restore_vector_index(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<RestoreStats>;

    async fn vector_index_health(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<IndexHealth>;
}
